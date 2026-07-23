//! Explicit application effects (R10a).
//!
//! Key handlers describe requested external work by enqueuing an [`AppEffect`]
//! into `App::pending_effect` instead of performing filesystem, rescan, or
//! background-probe work inline. The boundary executes the effect: in-place
//! effects (rename, delete, rescan) run here in [`App::apply_effect`] while the
//! TUI stays mounted; the terminal-unmounting handovers (resume / new session /
//! login / terminal) keep their existing discrete request fields drained by the
//! `runtime` event loop.
//!
//! Effect execution stays synchronous / threaded exactly as before — this phase
//! introduces no async runtime (plan §8.2). To preserve the current per-event
//! timing, the event loop applies the pending effect immediately after each
//! dispatched key event, so no redraw occurs between a handler and its effect.
//!
//! Pure state recomputation (`recompute`, `rebuild_all_folders`) is not an
//! effect; only work that touches the filesystem, rescans session storage, or
//! spawns background probes is modeled here.

use crate::ui::{App, Screen, UiMode};

/// Lifecycle of the two-phase global refresh (Ctrl+U / palette "Refresh All").
///
/// A refresh cycle renders one preparing frame before the synchronous session
/// scan so the user sees loading feedback immediately instead of a stale frame:
///
/// 1. `begin` — the effect runs the prepare step (background usage/model
///    probes + in-progress status) and schedules the scan.
/// 2. The event loop draws the prepared state, then runs the scheduled scan
///    right away without waiting for another input event (`mark_scanned`).
/// 3. Repeat requests arriving anywhere in the cycle merge into it (`begin`
///    returns false). The loop drains queued input after the scan and only then
///    ends the cycle (`finish`), so Ctrl+U presses queued during the scan
///    cannot schedule a second scan; a press after the completion frame starts
///    a fresh cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RefreshAllPhase {
    /// No refresh cycle active; the next request starts one.
    #[default]
    Idle,
    /// Prepare ran; the session scan runs right after the next completed draw.
    Prepared,
    /// The scan ran; the cycle stays active (merging repeat requests) until the
    /// completion frame is about to render.
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

    /// Marks the scheduled scan as executed (the cycle remains active).
    pub(crate) fn mark_scanned(&mut self) {
        *self = RefreshAllPhase::Scanned;
    }

    /// Ends the cycle: the completion result is about to render, so the next
    /// request starts a fresh cycle.
    pub(crate) fn finish(&mut self) {
        *self = RefreshAllPhase::Idle;
    }
}

/// External work requested by a key handler, executed at the `App` boundary
/// while the TUI remains mounted. Handovers that must unmount the terminal are
/// modeled separately as discrete `*_request` fields on `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppEffect {
    /// Global refresh (Ctrl+U / palette "Refresh All"): start the background
    /// usage/model probes and schedule a session rescan for right after the
    /// next draw (two-phase; see [`RefreshAllPhase`]).
    RefreshAll,
    /// Rename the session at `idx` to `title` via the owning agent CLI, then
    /// rescan. The rename dialog is closed only on success; a failure keeps it
    /// open with an error so the user can retry. Pre-flight validation (empty
    /// title, missing profile) is performed by the handler before enqueuing.
    RenameSession { idx: usize, title: String },
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
        match effect {
            AppEffect::RefreshAll => self.run_refresh_all(),
            AppEffect::RenameSession { idx, title } => self.run_rename_session(idx, title),
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
    /// the synchronous session scan to run right after the next draw
    /// ([`App::run_scheduled_refresh_scan`]) so the loading state is visible
    /// before the 1–2s scan blocks the loop. Repeat requests while a cycle is
    /// active merge into it. Model catalogs are force-refreshed (bypassing
    /// version gates) to capture plan changes. The usage/model fetches go
    /// through the existing start methods so the `Loading` phase flips only
    /// together with an actually spawned probe (never set the phase directly).
    fn run_refresh_all(&mut self) {
        if !self.refresh_all.begin() {
            // Merged into the active cycle: no second prepare/scan. Restore the
            // cycle's progress message, which the key handler just cleared.
            self.status_msg = Some(match self.refresh_all {
                RefreshAllPhase::Prepared => Self::refresh_status_preparing(),
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

    /// Status once the session scan has completed (usage still updating).
    fn refresh_status_scanned(&self) -> String {
        format!(
            "session update complete · {} · updating usage…",
            self.scan_info
        )
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
    pub(crate) fn run_scheduled_refresh_scan(&mut self) {
        if !self.refresh_all.scan_scheduled() {
            return;
        }
        self.refresh_all.mark_scanned();
        self.refresh_sessions();
        self.status_msg = Some(self.refresh_status_scanned());
    }

    /// Ends the refresh cycle. Called by the event loop after the post-scan
    /// input drain, immediately before the completion frame renders, so only a
    /// Ctrl+U pressed after that frame starts a fresh cycle.
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
        if let Err(err) = self.delete_session_artifacts(&session) {
            self.status_msg = Some(format!("Delete failed: {err}"));
            return;
        }
        self.sessions.remove(idx);
        self.rebuild_all_folders();
        self.recompute();
        if self.screen == Screen::Detail {
            self.close_session_detail();
        }
        self.status_msg = Some(format!(
            "Deleted [{}] {}",
            session.agent.label(),
            session.title()
        ));
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
