//! Background session refresh snapshots and safe publication of their cache.

use super::{effect::RefreshAllPhase, App, UiMode};
use crate::{handoff::HandoffTurn, model::Session, scan::ScanResult};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SessionKey {
    agent: crate::model::Agent,
    profile_id: String,
    id: String,
}

impl SessionKey {
    pub(crate) fn of(session: &Session) -> Self {
        Self {
            agent: session.agent,
            profile_id: session.profile_id.clone(),
            id: session.id.clone(),
        }
    }

    pub(crate) fn matches(&self, session: &Session) -> bool {
        self.agent == session.agent
            && self.profile_id == session.profile_id
            && self.id == session.id
    }
}

pub(crate) type DetailRefresh = (SessionKey, Vec<HandoffTurn>);

pub(crate) struct RefreshResult {
    pub(crate) scan: ScanResult,
    pub(crate) detail: Option<DetailRefresh>,
    cache: Option<StagedCache>,
}

impl RefreshResult {
    pub(crate) fn collect(
        profiles: &[crate::profile::Profile],
        detail: Option<SessionKey>,
        cache_path: &Path,
        workspace_path: &Path,
    ) -> Self {
        let (scan, cache) = crate::scan::collect_at(profiles, false, cache_path, workspace_path);
        let detail = detail.and_then(|key| {
            scan.sessions
                .iter()
                .find(|s| key.matches(s))
                .map(|s| (key, crate::handoff::load_turns(s)))
        });
        // Serialize and write off the UI thread. Only the final rename occurs
        // on acceptance; superseded workers can never publish their cache.
        static NEXT_CACHE: AtomicU64 = AtomicU64::new(0);
        let staging = cache_path.with_extension(format!(
            "refresh-{}-{}",
            std::process::id(),
            NEXT_CACHE.fetch_add(1, Ordering::Relaxed)
        ));
        let staged = StagedCache {
            staging,
            destination: cache_path.to_owned(),
        };
        let cache = cache.save(&staged.staging).ok().map(|_| staged);
        Self {
            scan,
            detail,
            cache,
        }
    }

    pub(crate) fn publish_cache(&mut self) {
        if let Some(cache) = self.cache.take() {
            let _ = std::fs::rename(&cache.staging, &cache.destination);
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(sessions: Vec<Session>) -> Self {
        Self {
            scan: ScanResult {
                scanned_files: sessions.len(),
                reparsed_files: 0,
                sessions,
            },
            detail: None,
            cache: None,
        }
    }
}

struct StagedCache {
    staging: PathBuf,
    destination: PathBuf,
}

impl Drop for StagedCache {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.staging);
    }
}

impl App {
    /// Rescans sessions on disk to refresh the session list (utilizes mtime-based incremental cache).
    ///
    /// Only modified or new files are parsed. Tracks selection by (agent, profile, id)
    /// to preserve cursor position post-refresh (even if content modifications re-order lists to the top).
    /// Used after handovers and mutations. Ctrl+U scans on a worker and shares
    /// only result application with this synchronous path.
    pub fn refresh_sessions(&mut self) {
        self.cancel_refresh_scan();
        let result = crate::scan::scan(&self.profiles.profiles, false);
        let detail = self.detail_key().and_then(|key| {
            result
                .sessions
                .iter()
                .find(|s| key.matches(s))
                .map(|s| (key, crate::handoff::load_turns(s)))
        });
        self.apply_session_scan(result, detail);
        // A handover is where `←` most often moves a session to the background.
        self.request_agent_status();
    }

    /// Applies a completed index against the selection and detail target that
    /// exist now, including navigation performed while the worker was running.
    pub(crate) fn apply_session_scan(
        &mut self,
        result: crate::scan::ScanResult,
        detail: Option<DetailRefresh>,
    ) {
        let prev = self.current().map(SessionKey::of);
        let detail_key = self.detail_key();
        let preview = (
            self.preview_scroll,
            self.preview_expanded,
            self.table_state.borrow().offset(),
        );
        self.scan_info = format!(
            "{} sessions · reparsed {}/{}",
            result.sessions.len(),
            result.reparsed_files,
            result.scanned_files
        );
        self.sessions = result.sessions;
        self.rebuild_all_folders();
        self.recompute();

        // Restore selection: if the same session still passes the filters, move cursor to its new index.
        if let Some(key) = prev {
            if let Some(pos) = self
                .filtered
                .iter()
                .position(|&i| key.matches(&self.sessions[i]))
            {
                self.selected = pos;
                self.preview_scroll = preview.0;
                self.preview_expanded = preview.1;
                *self.table_state.borrow_mut().offset_mut() = preview.2;
            }
        }

        // Rebind the current Detail target. Only replace its turns when the
        // result was loaded for that identity; navigation may have changed it.
        if self.detail.is_some() {
            let found = detail_key
                .as_ref()
                .and_then(|key| self.sessions.iter().position(|s| key.matches(s)));
            match found {
                Some(idx) => {
                    let turns = detail
                        .filter(|(key, _)| Some(key) == detail_key.as_ref())
                        .map(|(_, turns)| turns);
                    if turns.as_ref().is_some_and(Vec::is_empty) {
                        self.close_session_detail();
                    } else if let Some(d) = self.detail.as_mut() {
                        d.session_idx = idx;
                        if let Some(turns) = turns {
                            d.selected = d.selected.min(turns.len() - 1);
                            d.expanded_prompt = d.expanded_prompt.filter(|&i| i < turns.len());
                            d.turns = turns;
                        }
                    }
                }
                None => self.close_session_detail(),
            }
        }
    }

    pub(crate) fn detail_key(&self) -> Option<SessionKey> {
        self.detail
            .as_ref()
            .and_then(|d| self.sessions.get(d.session_idx))
            .map(SessionKey::of)
    }

    /// Invalidates snapshots before a mutation or a synchronous handover rescan.
    /// Dropping the receiver also discards a late worker's staged cache.
    pub(crate) fn cancel_refresh_scan(&mut self) {
        self.background.cancel_refresh();
        self.refresh_all = RefreshAllPhase::Idle;
    }

    pub(crate) fn poll_refresh_scan(&mut self) -> bool {
        // Dialogs retain indices into the old index/folder list. Let the user
        // finish or cancel before swapping it, and never rebind a handover target.
        if !matches!(self.mode, UiMode::Table | UiMode::Keyword)
            || self.resume_request.is_some()
            || self.new_session_request.is_some()
            || self.login_request.is_some()
            || self.terminal_request.is_some()
            || self.pending_effect.is_some()
            || self.should_quit
        {
            return false;
        }
        let Some(result) = self.background.take_refresh() else {
            return false;
        };
        match result {
            Ok(mut result) => {
                result.publish_cache();
                self.apply_session_scan(result.scan, result.detail);
                self.status_msg = Some(self.refresh_status_scanned());
            }
            Err(err) => self.status_msg = Some(format!("Session refresh failed: {err}")),
        }
        self.refresh_all.mark_scanned();
        true
    }
}

#[cfg(test)]
mod tests;
