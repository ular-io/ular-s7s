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

/// External work requested by a key handler, executed at the `App` boundary
/// while the TUI remains mounted. Handovers that must unmount the terminal are
/// modeled separately as discrete `*_request` fields on `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppEffect {
    /// Global refresh (Ctrl+U / palette "Refresh All"): rescan sessions, then
    /// query usage and force-refresh model catalogs in the background.
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

    /// Global Ctrl+U: refresh session list, usage stats, and model catalogs
    /// concurrently (shared across all main screens). Model catalogs are
    /// force-refreshed (bypassing version gates) to capture plan changes, though
    /// silently in the background unlike the usage feedback.
    fn run_refresh_all(&mut self) {
        self.refresh_sessions();
        self.start_usage_fetch();
        self.start_models_fetch(true);
        self.status_msg = Some(format!(
            "session update complete · {} · updating usage…",
            self.scan_info
        ));
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
