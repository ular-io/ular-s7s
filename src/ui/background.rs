//! Background usage/model probe job coordination (R10b).
//!
//! Isolates the receiver channels and in-flight tracking for the background
//! usage and model-catalog probes out of `App`. Only the *coordination* state
//! lives here — the receivers and the model-loading dedup guard — so key
//! handlers and rendering never touch receiver internals directly. The result
//! caches (`UsageState`, `ModelCatalog`) stay on `App` because they are read
//! and written across features (session/profile/new-session rendering, profile
//! deletion cleanup, new-session launch persistence).
//!
//! `App` keeps thin forwarding methods (`start_usage_fetch`, `poll_usage`, …)
//! that read the App-side caches/profiles and delegate only the receiver
//! operations to this struct. `drain_*` returns owned results so the borrow of
//! `App::background` is released before `App` mutates its caches (avoids the
//! borrow pressure of holding `&mut background` and `&mut usage` together —
//! plan §15.2).
//!
//! Spawning stays synchronous/threaded (thread + mpsc); this introduces no
//! async runtime (plan §8.2). The `cfg!(test)` spawn guard stays in the `App`
//! methods so unit tests neither spawn PTYs nor mutate cache state.

use crate::models::{self, ModelsResult};
use crate::profile::Profile;
use crate::usage::{self, UsageResult};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, TryRecvError};

/// Coordination state for background usage/model probe jobs.
#[derive(Default)]
pub(crate) struct BackgroundState {
    /// Receivers for active usage query results (removed on completion). Items are (profile_id, UsageResult).
    /// Maintained as a vector to allow concurrent runs of full updates (Ctrl+U) and incremental updates (profile add/edit).
    usage_rxs: Vec<Receiver<(String, usage::UsageResult)>>,
    /// Receivers for active model query results (removed on completion).
    models_rxs: Vec<Receiver<(String, models::ModelsResult)>>,
    /// Profile IDs with active model queries (prevents duplicate PTY queries for the same profile).
    models_loading: HashSet<String>,
}

impl BackgroundState {
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

    /// Returns whether any background query (usage or models) is in progress
    /// (used to determine polling frequency in the main loop).
    pub(crate) fn in_flight(&self) -> bool {
        !self.usage_rxs.is_empty() || !self.models_rxs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::BackgroundState;

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
