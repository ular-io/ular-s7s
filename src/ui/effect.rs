//! External application effects requested by key handlers.
//!
//! Key handlers describe requested external work by enqueuing an [`AppEffect`]
//! into `App::pending_effect` instead of performing filesystem, rescan, or
//! background-probe work inline. The boundary executes the effect: in-place
//! effects (rename, delete, rescan) run here in [`App::apply_effect`] while the
//! TUI stays mounted; the terminal-unmounting handovers (resume / new session /
//! login / terminal) keep their existing discrete request fields drained by the
//! `runtime` event loop.
//!
//! The event loop applies pending effects immediately after each input event.
//! Global refresh schedules a worker after loading feedback renders; other
//! in-place effects remain synchronous. No async runtime is required.
//!
//! Pure state recomputation (`recompute`, `rebuild_all_folders`) is not an
//! effect; only work that touches the filesystem, rescans session storage, or
//! spawns background probes is modeled here.

use crate::ui::{App, Screen, UiMode};
use std::path::PathBuf;

/// Lifecycle of a global refresh. Prepare renders loading feedback, Scanning
/// keeps the event loop responsive, and Scanned merges requests until the
/// completion frame has rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RefreshAllPhase {
    /// No refresh cycle active; the next request starts one.
    #[default]
    Idle,
    /// Prepare ran; the session scan runs right after the next completed draw.
    Prepared,
    /// The worker is scanning; input and background results continue normally.
    Scanning,
    /// The scan ran; the cycle stays active (merging repeat requests) until the
    /// completion frame has rendered.
    Scanned,
}

impl RefreshAllPhase {
    /// Handles a refresh request: returns true when a new cycle starts (the
    /// caller must run the prepare step), false to merge into the active cycle.
    pub(crate) fn begin(&mut self) -> bool {
        if *self == RefreshAllPhase::Idle {
            *self = RefreshAllPhase::Prepared;
            true
        } else {
            false
        }
    }

    /// Whether a session scan is scheduled to run after the next draw.
    pub(crate) fn scan_scheduled(self) -> bool {
        self == RefreshAllPhase::Prepared
    }

    /// Marks the worker result as applied (the cycle remains active).
    pub(crate) fn mark_scanned(&mut self) {
        *self = RefreshAllPhase::Scanned;
    }

    /// Ends the cycle after the completion result renders, so the next
    /// request starts a fresh cycle.
    pub(crate) fn finish(&mut self) {
        if *self == RefreshAllPhase::Scanned {
            *self = RefreshAllPhase::Idle;
        }
    }
}

/// External work requested by a key handler, executed at the `App` boundary
/// while the TUI remains mounted. Handovers that must unmount the terminal are
/// modeled separately as discrete `*_request` fields on `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppEffect {
    /// Global refresh (Ctrl+U / palette "Refresh All"): start the background
    /// usage/model probes and schedule a session rescan for right after the
    /// next draw (see [`RefreshAllPhase`]).
    RefreshAll,
    /// Persist the selected session's bookmark before updating its title marker.
    ToggleBookmark { idx: usize },
    /// Rename the session at `idx` to `title` via the owning agent CLI, then
    /// rescan. The rename dialog is closed only on success; a failure keeps it
    /// open with an error so the user can retry. Pre-flight validation (empty
    /// title, missing profile) is performed by the handler before enqueuing.
    RenameSession { idx: usize, title: String },
    /// Re-point the session at `idx` to `folder`: record it in the s7s-owned
    /// folder store and rescan. Nothing on disk moves and the agent transcript
    /// is never rewritten, so the change is reversible by recording the old
    /// folder again. A write failure keeps the dialog open with the error.
    ChangeSessionFolder { idx: usize, folder: PathBuf },
    /// Delete the session at `idx`: remove its on-disk artifacts, drop it from
    /// the list, rebuild folders/filters, and return from the Detail screen if
    /// open. A filesystem failure leaves the list unchanged with an error.
    DeleteSession { idx: usize },
    /// Persist the profile store after a form commit, then rescan sessions and
    /// incrementally fetch usage/models for the saved profile `id`. A save
    /// failure keeps the form open with an error. When `request_login` is set
    /// (config-directory creation path), a login handover is requested for
    /// agents that support it. The store is already mutated in memory by the
    /// handler; this effect owns the disk write and everything after it.
    ProfileSaved {
        id: String,
        name: String,
        request_login: bool,
    },
}

impl App {
    /// Executes and clears the pending effect, if any. Called by the event loop
    /// immediately after each dispatched key event so effect timing matches the
    /// previous inline behavior (no redraw in between).
    pub(crate) fn apply_effect(&mut self) {
        let Some(effect) = self.pending_effect.take() else {
            return;
        };
        if !matches!(
            effect,
            AppEffect::RefreshAll | AppEffect::ToggleBookmark { .. }
        ) {
            self.cancel_refresh_scan();
        }
        match effect {
            AppEffect::RefreshAll => self.run_refresh_all(),
            AppEffect::ToggleBookmark { idx } => self.run_toggle_bookmark(idx),
            AppEffect::RenameSession { idx, title } => self.run_rename_session(idx, title),
            AppEffect::ChangeSessionFolder { idx, folder } => {
                self.run_change_session_folder(idx, folder)
            }
            AppEffect::DeleteSession { idx } => self.run_delete_session(idx),
            AppEffect::ProfileSaved {
                id,
                name,
                request_login,
            } => self.run_profile_saved(id, name, request_login),
        }
    }

    /// Global Ctrl+U, prepare phase (shared across all main screens): start the
    /// background usage/model probes, show an in-progress status, and schedule
    /// a background session scan to start right after the next draw
    /// ([`App::start_scheduled_refresh_scan`]) so loading feedback is immediate.
    /// Repeat requests while a cycle is active merge into it. Model catalogs are force-refreshed (bypassing
    /// version gates) to capture plan changes. The usage/model fetches go
    /// through the existing start methods so the `Loading` phase flips only
    /// together with an actually spawned probe (never set the phase directly).
    fn run_refresh_all(&mut self) {
        if !self.refresh_all.begin() {
            // Merged into the active cycle: no second prepare/scan. Restore the
            // cycle's progress message, which the key handler just cleared.
            self.status_msg = Some(match self.refresh_all {
                RefreshAllPhase::Prepared | RefreshAllPhase::Scanning => {
                    Self::refresh_status_preparing()
                }
                _ => self.refresh_status_scanned(),
            });
            return;
        }
        self.start_usage_fetch();
        self.start_models_fetch(true);
        self.status_msg = Some(Self::refresh_status_preparing());
    }

    /// Status while the preparing frame is on screen (scan still pending).
    fn refresh_status_preparing() -> String {
        "updating sessions and usage…".to_string()
    }

    /// Status after the scan completes, including any remaining usage query.
    pub(crate) fn refresh_status_scanned(&self) -> String {
        let suffix = if self.usage_in_flight() {
            " · updating usage…"
        } else {
            ""
        };
        format!("session update complete · {}{}", self.scan_info, suffix)
    }

    /// Whether a Ctrl+U session scan is scheduled (checked by the event loop
    /// right after each draw).
    pub(crate) fn refresh_scan_scheduled(&self) -> bool {
        self.refresh_all.scan_scheduled()
    }

    /// Runs the session scan scheduled by [`AppEffect::RefreshAll`]. Called by
    /// the event loop right after the preparing frame is rendered — never in
    /// response to a new input event. The cycle stays active (merging queued
    /// repeat requests) until [`App::finish_refresh_cycle`].
    pub(crate) fn start_scheduled_refresh_scan(&mut self) {
        if !self.refresh_scan_scheduled() {
            return;
        }
        self.refresh_all = RefreshAllPhase::Scanning;
        self.background
            .spawn_refresh(self.profiles.profiles.clone(), self.detail_key());
    }

    /// Ends a completed refresh cycle after its completion frame renders.
    /// Prepared and running cycles remain active and merge repeat requests.
    pub(crate) fn finish_refresh_cycle(&mut self) {
        self.refresh_all.finish();
    }

    /// Renames the session's title via the owning agent CLI and rescans. On
    /// success the rename dialog is dismissed and the cursor tracks the renamed
    /// session; on failure the dialog stays open with the error message so the
    /// user can retry. The session/profile are re-resolved here because state
    /// may have changed between enqueue and execution (defensive; nothing
    /// mutates in the intervening no-redraw window today).
    fn run_rename_session(&mut self, idx: usize, title: String) {
        let Some(session) = self.sessions.get(idx).cloned() else {
            self.status_msg = Some("No session selected".to_string());
            return;
        };
        // Metadata paths and CLI env derive from the owning profile; never fall
        // back to the default root (wrong account store for extra profiles).
        let Some(profile) = self.profiles.find(&session.profile_id).cloned() else {
            self.status_msg =
                Some("Rename failed: session profile not found — refresh with ctrl+u".to_string());
            return;
        };
        match crate::rename::rename_session(&profile, &session, &title) {
            Ok(()) => {
                self.rename_modal = None;
                self.rename_target = None;
                self.mode = UiMode::Table;
                self.refresh_sessions();
                self.status_msg = Some(format!("Renamed session: {}", title));
            }
            Err(err) => {
                self.status_msg = Some(format!("Rename failed: {err}"));
            }
        }
    }

    /// Records the session's new folder in the s7s-owned store and rescans, so
    /// the list column and the next resume follow it together. The agent's own
    /// storage is untouched: the folder written there stays as the session's
    /// starting folder, and dropping the record restores it.
    fn run_change_session_folder(&mut self, idx: usize, folder: PathBuf) {
        let Some(session) = self.sessions.get(idx).cloned() else {
            self.status_msg = Some("Folder change target no longer exists".to_string());
            return;
        };
        match crate::session_workspace::record(
            &crate::config::session_workspaces_path(),
            &session.profile_id,
            &session.id,
            &folder,
        ) {
            Ok(()) => {
                self.change_folder = None;
                self.change_folder_target = None;
                self.mode = UiMode::Table;
                self.refresh_sessions();
                self.status_msg = Some(format!("Folder changed: {}", folder.display()));
            }
            Err(err) => match self.change_folder.as_mut() {
                Some(state) => state.error = Some(format!("Change failed: {err}")),
                None => self.status_msg = Some(format!("Change failed: {err}")),
            },
        }
    }

    /// Deletes the session's on-disk artifacts and removes it from the list. A
    /// filesystem failure leaves the list untouched with an error message. On
    /// success `recompute` clamps the cursor within bounds, so selection shifts
    /// to the following row; if invoked from the Detail screen it closes and
    /// returns to the search view.
    fn run_delete_session(&mut self, idx: usize) {
        let Some(session) = self.sessions.get(idx).cloned() else {
            self.status_msg = Some("Delete target no longer exists".to_string());
            return;
        };
        let outcome = match crate::session_delete::delete_session(
            &self.profiles,
            &session,
            &self.bookmarks_path,
        ) {
            Ok(outcome) => outcome,
            Err(err) => {
                self.status_msg = Some(format!("Delete failed: {err}"));
                return;
            }
        };
        if let Some(bookmarks) = outcome.bookmarks {
            self.bookmarks = bookmarks;
        } else {
            self.bookmarks.set(&session, false);
        }
        self.sessions.remove(idx);
        self.rebuild_all_folders();
        self.recompute();
        if self.screen == Screen::Detail {
            self.close_session_detail();
        }
        let mut message = format!("Deleted [{}] {}", session.agent.label(), session.title());
        if let Some(warning) = outcome.bookmark_warning {
            message.push_str(&format!("; {warning}"));
        }
        self.status_msg = Some(message);
    }

    /// Persists the profile store, then rescans and incrementally fetches
    /// usage/models for the saved profile. A save failure keeps the form open
    /// with an error (mirroring the previous inline behavior). When
    /// `request_login` is set, a login handover is requested for agents whose
    /// config folder supports environment overrides; other agents get a
    /// manual-login status message instead.
    fn run_profile_saved(&mut self, id: String, name: String, request_login: bool) {
        if let Err(e) = self.profiles.save() {
            if let Some(form) = self.profile_form.as_mut() {
                form.error = Some(format!("failed to save profiles.json: {e}"));
            }
            // Restore profile form mode since this might have been invoked from
            // the config-directory confirmation modal.
            self.mode = UiMode::ProfileForm;
            return;
        }

        self.profile_form = None;
        self.mode = UiMode::Table;
        // Immediately load sessions for the new/modified profile directory (rescan is cheap thanks to mtime caching).
        // Since usage and model catalogs queries require expensive PTY runs, only run incremental updates for the saved profile
        // (models are forcefully queried in case the config directory path has changed).
        self.refresh_sessions();
        self.start_usage_fetch_for(std::slice::from_ref(&id));
        self.start_models_fetch_for(std::slice::from_ref(&id), true);
        self.status_msg = Some(format!("Profile saved: {name}"));

        if request_login {
            // Custom Antigravity paths do not support environment variable overrides, rendering login launches meaningless.
            let runnable = self
                .profiles
                .find(&id)
                .map(|p| crate::profile::login_runnable(p.agent, &p.path))
                .unwrap_or(false);
            if runnable {
                self.login_request = Some(id);
            } else {
                self.status_msg = Some(format!(
                    "Profile saved: {name} — log in manually (custom config folder not supported)"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RefreshAllPhase;

    #[test]
    fn refresh_phase_starts_one_cycle_and_merges_repeats() {
        let mut phase = RefreshAllPhase::default();
        assert!(!phase.scan_scheduled());

        // First request starts the cycle and schedules exactly one scan.
        assert!(phase.begin());
        assert!(phase.scan_scheduled());

        // Repeats before the preparing frame renders merge into the cycle.
        assert!(!phase.begin());
        assert!(phase.scan_scheduled());

        // The scan ran; nothing further is scheduled, and requests queued
        // during the scan still merge instead of scheduling a second scan.
        phase.mark_scanned();
        assert!(!phase.scan_scheduled());
        assert!(!phase.begin());
        assert!(!phase.scan_scheduled());

        // After the completion frame, a new request starts a fresh cycle.
        phase.finish();
        assert!(phase.begin());
        assert!(phase.scan_scheduled());
    }
}
