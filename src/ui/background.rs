//! Receiver and in-flight coordination for usage, model, and session-refresh jobs.
//!
//! Application caches remain on `App`. This module owns channels and duplicate
//! query guards, returning owned results before the application mutates its
//! state. Jobs use threads and mpsc without an async runtime. Probe spawn guards
//! live on `App`; session-refresh tests use controlled receivers and isolated
//! stores.

use crate::agent_status;
use crate::models::{self, ModelsResult};
use crate::profile::Profile;
use crate::usage::{self, UsageResult};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};

/// Coordination state for background probes and session-refresh jobs.
#[derive(Default)]
pub(crate) struct BackgroundState {
    /// Receivers for active usage query results (removed on completion). Items are (profile_id, UsageResult).
    /// Maintained as a vector to allow concurrent runs of full updates (Ctrl+U) and incremental updates (profile add/edit).
    usage_rxs: Vec<Receiver<(String, usage::UsageResult)>>,
    /// One active session scan, including a completed result awaiting a safe UI mode.
    refresh_rx: Option<Receiver<Result<super::refresh::RefreshResult, String>>>,
    /// Receivers for active model query results (removed on completion).
    models_rxs: Vec<Receiver<(String, models::ModelsResult)>>,
    /// Profile IDs with active model queries (prevents duplicate PTY queries for the same profile).
    models_loading: HashSet<String>,
    /// One active live-session sweep over every Claude and agy profile.
    agent_status_rx: Option<Receiver<(String, Option<agent_status::StatusMap>)>>,
}

impl BackgroundState {
    pub(crate) fn spawn_refresh(
        &mut self,
        profiles: Vec<Profile>,
        detail: Option<super::refresh::SessionKey>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.refresh_rx = Some(rx);
        // App tests use controlled snapshots instead of touching real stores.
        #[cfg(test)]
        {
            let _ = (profiles, detail);
            let _ = tx.send(Ok(super::refresh::RefreshResult::fixture(Vec::new())));
        }
        #[cfg(not(test))]
        {
            let failure_tx = tx.clone();
            if let Err(err) = std::thread::Builder::new()
                .name("s7s-refresh".into())
                .spawn(move || {
                    let result = super::refresh::RefreshResult::collect(
                        &profiles,
                        detail,
                        &crate::config::cache_path(),
                        &crate::config::session_workspaces_path(),
                    );
                    let _ = tx.send(Ok(result));
                })
            {
                let _ = failure_tx.send(Err(err.to_string()));
            }
        }
    }

    pub(crate) fn take_refresh(&mut self) -> Option<Result<super::refresh::RefreshResult, String>> {
        let rx = self.refresh_rx.as_ref()?;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("worker disconnected".into()),
        };
        self.refresh_rx = None;
        Some(result)
    }

    pub(crate) fn cancel_refresh(&mut self) {
        self.refresh_rx = None;
    }

    #[cfg(test)]
    pub(crate) fn set_refresh_receiver(
        &mut self,
        rx: Receiver<Result<super::refresh::RefreshResult, String>>,
    ) {
        self.refresh_rx = Some(rx);
    }

    /// Spawns a usage query for `targets` and retains its receiver.
    pub(crate) fn spawn_usage(&mut self, targets: Vec<Profile>) {
        self.usage_rxs.push(usage::spawn_fetch(targets));
    }

    /// Drains all pending usage results, dropping receivers whose channel has
    /// disconnected (query fully complete).
    pub(crate) fn drain_usage(&mut self) -> Vec<(String, UsageResult)> {
        let mut results: Vec<(String, UsageResult)> = Vec::new();
        // Iterate receivers to collect incoming messages; remove channels that are fully completed (Disconnected).
        self.usage_rxs.retain(|rx| loop {
            match rx.try_recv() {
                Ok(item) => results.push(item),
                Err(TryRecvError::Empty) => break true,
                Err(TryRecvError::Disconnected) => break false,
            }
        });
        results
    }

    /// Returns whether a usage query task is in progress.
    pub(crate) fn usage_in_flight(&self) -> bool {
        !self.usage_rxs.is_empty()
    }

    /// Returns whether a model query task is in progress.
    pub(crate) fn models_in_flight(&self) -> bool {
        !self.models_rxs.is_empty()
    }

    /// Whether a model query is already active for `profile_id` (dedup guard).
    pub(crate) fn is_models_loading(&self, profile_id: &str) -> bool {
        self.models_loading.contains(profile_id)
    }

    /// Marks `targets` as loading, spawns a model query, and retains its receiver.
    pub(crate) fn spawn_models(
        &mut self,
        targets: Vec<Profile>,
        cached_versions: HashMap<String, Option<String>>,
        force: bool,
    ) {
        for p in &targets {
            self.models_loading.insert(p.id.clone());
        }
        self.models_rxs
            .push(models::spawn_fetch(targets, cached_versions, force));
    }

    /// Drains all pending model results, dropping completed receivers and
    /// clearing the loading guard for completed profiles (and fully once all
    /// channels finish, to clean up any trailing markers).
    pub(crate) fn drain_models(&mut self) -> Vec<(String, ModelsResult)> {
        let mut results: Vec<(String, ModelsResult)> = Vec::new();
        self.models_rxs.retain(|rx| loop {
            match rx.try_recv() {
                Ok(item) => results.push(item),
                Err(TryRecvError::Empty) => break true,
                Err(TryRecvError::Disconnected) => break false,
            }
        });
        for (profile_id, _) in &results {
            self.models_loading.remove(profile_id);
        }
        if self.models_rxs.is_empty() {
            // Defensively clear loading tracking to clean up trailing markers once all channels complete.
            self.models_loading.clear();
        }
        results
    }

    /// Starts a live-session status sweep unless one is already running.
    pub(crate) fn spawn_agent_status(&mut self, profiles: Vec<Profile>) {
        if self.agent_status_rx.is_none() {
            self.agent_status_rx = Some(agent_status::spawn_fetch(profiles));
        }
    }

    /// Drains finished per-profile status results; the receiver is dropped once
    /// the sweep has answered for every profile.
    pub(crate) fn drain_agent_status(&mut self) -> Vec<(String, Option<agent_status::StatusMap>)> {
        let mut results = Vec::new();
        let Some(rx) = self.agent_status_rx.as_ref() else {
            return results;
        };
        loop {
            match rx.try_recv() {
                Ok(item) => results.push(item),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.agent_status_rx = None;
                    break;
                }
            }
        }
        results
    }

    #[cfg(test)]
    pub(crate) fn set_agent_status_receiver(
        &mut self,
        rx: Receiver<(String, Option<agent_status::StatusMap>)>,
    ) {
        self.agent_status_rx = Some(rx);
    }

    /// Returns whether any background job is in progress
    /// (used to determine polling frequency in the main loop).
    pub(crate) fn in_flight(&self) -> bool {
        !self.usage_rxs.is_empty()
            || !self.models_rxs.is_empty()
            || self.refresh_rx.is_some()
            || self.agent_status_rx.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::BackgroundState;

    #[test]
    fn usage_completion_does_not_overwrite_session_refresh_progress() {
        use crate::ui::{effect::RefreshAllPhase, test_support::empty_app};
        for phase in [RefreshAllPhase::Prepared, RefreshAllPhase::Scanning] {
            let mut app = empty_app();
            app.refresh_all = phase;
            app.status_msg = Some("updating sessions and usage…".into());
            let (tx, rx) = std::sync::mpsc::channel();
            tx.send(("profile".into(), crate::usage::UsageResult::Unavailable))
                .unwrap();
            drop(tx);
            app.background.usage_rxs.push(rx);
            assert!(app.poll_usage());
            assert!(!app.usage_in_flight());
            assert_eq!(
                app.status_msg.as_deref(),
                Some("updating sessions and usage…")
            );
        }
    }

    #[test]
    fn fresh_state_has_no_jobs_in_flight() {
        let bg = BackgroundState::default();
        assert!(!bg.usage_in_flight());
        assert!(!bg.models_in_flight());
        assert!(!bg.in_flight());
        assert!(!bg.is_models_loading("any-profile"));
    }

    #[test]
    fn draining_with_no_receivers_yields_nothing() {
        let mut bg = BackgroundState::default();
        assert!(bg.drain_usage().is_empty());
        assert!(bg.drain_models().is_empty());
        assert!(!bg.in_flight());
    }
}
