//! The per-profile model catalog cache persisted in `~/.config/s7s/models.json`.

use super::{LastSelection, ProfileModels};
use crate::config::config_base_dir;
use crate::model::Agent;
use crate::profile::Profile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Cache of model lists per profile (key = profile id), persisted in models.json.
#[derive(Debug, Default)]
pub struct ModelCatalog {
    pub(super) entries: HashMap<String, ProfileModels>,
}

/// Serialization structure for models.json.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ModelsFile {
    pub(super) version: u32,
    pub(super) profiles: HashMap<String, ProfileModels>,
}

/// models.json schema/content version. Bumped whenever previously cached values can be
/// wrong rather than merely stale — a mismatch discards the whole file so the version gate
/// cannot keep serving bad entries (`default_model` / `last_selected` included).
///
/// - 1: initial.
/// - 2: claude `--model` values are normalized aliases (see `claude_model_value`); v1 caches may
///   hold the raw display name of the 1M-context row (`opus (1m context)`), which no CLI
///   accepts.
/// - 3: agy `models` rows are split into tab-separated slug/display-name fields; v2 caches
///   may hold the whole row (including a tab) as an invalid `--model` value.
/// - 4: claude picker rows include versioned names; old caches can contain incomplete lists.
pub(super) const MODELS_FILE_VERSION: u32 = 4;

impl ModelCatalog {
    /// Loads models.json. Returns an empty catalog if missing or corrupted.
    pub fn load() -> Self {
        Self::load_from(&models_file_path())
    }

    pub(super) fn load_from(path: &Path) -> Self {
        let entries = std::fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str::<ModelsFile>(&data).ok())
            .filter(|f| f.version == MODELS_FILE_VERSION)
            .map(|f| f.profiles)
            .unwrap_or_default();
        ModelCatalog { entries }
    }

    /// Saves to models.json (failures are non-fatal and can be ignored by callers).
    pub fn save(&self) -> std::io::Result<()> {
        // Unit tests must never touch the real user cache (start_new_session_at persists the
        // last pick through this path). `save_to` stays unguarded for roundtrip tests.
        if cfg!(test) {
            return Ok(());
        }
        self.save_to(&models_file_path())
    }

    pub(super) fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = ModelsFile {
            version: MODELS_FILE_VERSION,
            profiles: self.entries.clone(),
        };
        let data = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        std::fs::write(path, data)
    }

    /// Inserts (replaces) a profile's model catalog. Background re-fetches produce a fresh
    /// `ProfileModels` with `last_selected == None`, so we carry over the previously stored
    /// user pick to avoid wiping it on every refresh.
    pub fn insert(&mut self, profile_id: String, mut models: ProfileModels) {
        if models.last_selected.is_none() {
            if let Some(prev) = self.entries.get(&profile_id) {
                models.last_selected = prev.last_selected.clone();
            }
        }
        self.entries.insert(profile_id, models);
    }

    /// Records the model the user last launched with for a profile. No-op if the profile has
    /// no cached entry yet (rare: first run before any fetch completes — the pick simply isn't
    /// remembered until a catalog exists to hold it).
    pub fn set_last_selected(&mut self, profile_id: &str, selection: LastSelection) {
        if let Some(pm) = self.entries.get_mut(profile_id) {
            pm.last_selected = Some(selection);
        }
    }

    /// Clears cache when a profile is deleted.
    pub fn remove(&mut self, profile_id: &str) {
        self.entries.remove(profile_id);
    }

    /// Cached CLI version of a profile (input for version gate).
    pub fn cached_version(&self, profile_id: &str) -> Option<Option<String>> {
        self.entries.get(profile_id).map(|m| m.cli_version.clone())
    }

    /// Model list of a profile. Since environment variables cannot be injected to Antigravity's extra profiles,
    /// they lack query results and instead share the list of the default path profile (queried once globally).
    pub fn for_profile(&self, profile: &Profile) -> Option<&ProfileModels> {
        self.entries.get(&profile.id).or_else(|| {
            (profile.agent == Agent::Antigravity)
                .then(|| {
                    self.entries
                        .values()
                        .find(|m| m.agent == Agent::Antigravity)
                })
                .flatten()
        })
    }
}

/// models.json path: `~/.config/s7s/models.json`.
pub(super) fn models_file_path() -> PathBuf {
    config_base_dir().join("models.json")
}
