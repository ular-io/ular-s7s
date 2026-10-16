//! Usage and model-catalog probe jobs: spawning through `BackgroundState` and
//! applying their results to the `App`-owned caches.

use super::{effect, App};
use crate::models;
use crate::usage::{self, UsagePhase};

impl App {
    /// Spawns parallel queries to fetch usage (remaining %) for all fetchable profiles. Ignored if queries are already in flight.
    pub fn start_usage_fetch(&mut self) {
        if self.usage_in_flight() {
            return;
        }
        let ids: Vec<String> = self
            .profiles
            .profiles
            .iter()
            .map(|p| p.id.clone())
            .collect();
        self.start_usage_fetch_for(&ids);
    }

    /// Spawns parallel queries to fetch usage for specified profiles (used for incremental refreshes on profile add/edit).
    /// Only skips profiles currently in `Loading` state. Since non-fetchable states (missing directories, unsupportive agents)
    /// are modeled as results (`UsageResult::MissingDir` / `Unavailable`), all profiles are targetable,
    /// and profiles not included in an ongoing global query can start query tasks immediately.
    pub fn start_usage_fetch_for(&mut self, profile_ids: &[String]) {
        // Prevent launching interactive CLI processes (PTY) during unit tests.
        if cfg!(test) {
            return;
        }
        let targets: Vec<crate::profile::Profile> = self
            .profiles
            .profiles
            .iter()
            .filter(|p| profile_ids.contains(&p.id))
            .filter(|p| self.usage.entry(&p.id).phase != UsagePhase::Loading)
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        for p in &targets {
            // Retain the prior successful value (`last`) while turning on the progress indicator.
            self.usage.entry_mut(&p.id).phase = UsagePhase::Loading;
        }
        self.background.spawn_usage(targets);
    }

    /// Applies background usage query results to app state. Returns true if updates occurred (triggering a redraw).
    pub fn poll_usage(&mut self) -> bool {
        if !self.background.usage_in_flight() {
            return false;
        }
        let results = self.background.drain_usage();
        let updated = !results.is_empty();
        for (profile_id, res) in results {
            let entry = self.usage.entry_mut(&profile_id);
            match res {
                usage::UsageResult::Ready(snapshot) => {
                    entry.last = Some(snapshot);
                    entry.phase = UsagePhase::Ready;
                }
                // Logged out, missing CLI, missing dir, or unavailable: clear `last` to prevent misleading displays.
                usage::UsageResult::NotLoggedIn => {
                    entry.last = None;
                    entry.phase = UsagePhase::NotLoggedIn;
                }
                usage::UsageResult::NotInstalled => {
                    entry.last = None;
                    entry.phase = UsagePhase::NotInstalled;
                }
                usage::UsageResult::MissingDir => {
                    entry.last = None;
                    entry.phase = UsagePhase::MissingDir;
                }
                usage::UsageResult::Unavailable => {
                    entry.last = None;
                    entry.phase = UsagePhase::Unavailable;
                }
                // Keep the prior value (`last`) even if the current query failed.
                usage::UsageResult::Failed(_) => entry.phase = UsagePhase::Failed,
            }
        }
        if updated
            && !self.background.usage_in_flight()
            && !matches!(
                self.refresh_all,
                effect::RefreshAllPhase::Prepared | effect::RefreshAllPhase::Scanning
            )
        {
            self.status_msg = Some("usage update complete".to_string());
        }
        updated
    }

    /// Returns whether a usage query task is in progress (used to determine polling frequency in the main loop).
    pub fn usage_in_flight(&self) -> bool {
        self.background.usage_in_flight()
    }

    /// Spawns parallel queries to fetch model catalogs for all profiles.
    ///
    /// If `force` is false (app startup), bypasses queries for profiles where the cached CLI version
    /// matches the current version (version gate to avoid expensive claude PTY spin-up costs).
    /// If `force` is true (Ctrl+U), forcefully queries all profiles. Skips profiles with active tasks.
    pub fn start_models_fetch(&mut self, force: bool) {
        let ids: Vec<String> = self
            .profiles
            .profiles
            .iter()
            .map(|p| p.id.clone())
            .collect();
        self.start_models_fetch_for(&ids, force);
    }

    /// Spawns queries to fetch model catalogs for specified profiles (used for incremental updates on profile add/edit).
    pub fn start_models_fetch_for(&mut self, profile_ids: &[String], force: bool) {
        // Prevent launching interactive CLI processes (PTY) during unit tests.
        if cfg!(test) {
            return;
        }
        let targets: Vec<crate::profile::Profile> = self
            .profiles
            .profiles
            .iter()
            .filter(|p| profile_ids.contains(&p.id))
            .filter(|p| !self.background.is_models_loading(&p.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let cached_versions = targets
            .iter()
            .filter_map(|p| self.models.cached_version(&p.id).map(|v| (p.id.clone(), v)))
            .collect();
        self.background
            .spawn_models(targets, cached_versions, force);
    }

    /// Applies background model catalog query results. Returns true if updates occurred.
    ///
    /// Preserves existing caches on query failure or unavailability. Since CLIs do not filter out
    /// invalid model names, we must not clear cached catalogs on unsuccessful updates.
    pub fn poll_models(&mut self) -> bool {
        if !self.background.models_in_flight() {
            return false;
        }
        // Draining clears the per-profile loading guard; applying the results to
        // the model cache and persisting them stays here (the cache is App-owned).
        let results = self.background.drain_models();
        let mut dirty = false;
        for (profile_id, res) in results {
            if let models::ModelsResult::Ready(pm) = res {
                self.models.insert(profile_id, pm);
                dirty = true;
            }
        }
        if dirty {
            self.models.save().ok();
        }
        dirty
    }
}
