//! Profiles: groups of Agent type + name + config folder path + OAuth token (persisted only).
//!
//! Even for the same agent type, different config folders denote separate profiles (supporting multiple subscriptions).
//! The profile list is owned and saved by the app in `~/.config/s7s/profiles.json`.
//! If the file is missing, the default 3 builtin profiles are seeded at each agent's default root.
//!
//! Several s7s instances may share the file. The TUI saves each edit as a
//! [`ProfileChange`] applied by id onto a freshly read copy under a lock
//! ([`ProfileStore::commit`]), so one instance never drops another's profiles;
//! its in-memory list is replaced from disk only at startup and on `ctrl+u`.

use crate::config::{config_base_dir, expand};
use crate::model::Agent;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single profile. `path` is the root directory of the agent config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Internal identifier. Fixed slugs for builtin profiles; time-based unique slugs for user-added ones.
    pub id: String,
    pub agent: Agent,
    /// Display name on screen (e.g. "Claude", "Claude-Team Share").
    pub name: String,
    /// Agent config root (e.g., `~/.claude`). Session directories are derived from this path.
    pub path: PathBuf,
    /// OAuth token. Persisted only and not used yet.
    #[serde(default)]
    pub oauth_token: Option<String>,
    /// Compatibility flag indicating whether this profile has a header shortcut.
    /// `shortcut` stores the authoritative position.
    #[serde(default = "default_true")]
    pub active: bool,
    /// One-based header shortcut position. Kept separate from vector order so the profile table
    /// remains in registration order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<u8>,
    /// Indication of builtin status. Builtin profiles cannot be deleted or have their agent changed.
    #[serde(default)]
    pub builtin: bool,
}

fn default_true() -> bool {
    true
}

impl Profile {
    /// Session scanning target directory. Derived from the config root for each agent type.
    pub fn sessions_dir(&self) -> PathBuf {
        match self.agent {
            Agent::Claude => self.path.join("projects"),
            Agent::Codex => self.path.join("sessions"),
            // For Antigravity, the CLI directory itself is the session root (contains conversations/ etc.).
            Agent::Antigravity => self.path.clone(),
        }
    }

    /// Environment variable to inject for usage query and resume execution. Antigravity has no known variables.
    ///
    /// Do not inject for the default path profile: specifying `CLAUDE_CONFIG_DIR` for `~/.claude`
    /// alters the lookup compared to the default keychain query, resulting in a re-login prompt.
    /// Thus, the default path profiles maintain existing behavior without environment variables.
    pub fn env_var(&self) -> Option<(&'static str, &Path)> {
        if self.is_default_root() {
            return None;
        }
        match self.agent {
            Agent::Claude => Some(("CLAUDE_CONFIG_DIR", self.path.as_path())),
            Agent::Codex => Some(("CODEX_HOME", self.path.as_path())),
            Agent::Antigravity => None,
        }
    }

    /// Returns true if the path is identical to the agent's default config root.
    /// Used to determine whether to skip usage queries for extra Antigravity profiles.
    pub fn is_default_root(&self) -> bool {
        self.path == default_root(self.agent)
    }
}

/// Returns true if executing login (initial setup) is meaningful for the given agent/path combination.
/// Since Antigravity does not support custom config paths via env variables, executing it for a custom path
/// will only log into the default account; hence it is excluded.
pub fn login_runnable(agent: Agent, path: &Path) -> bool {
    agent != Agent::Antigravity || path == default_root(agent)
}

/// Default config root for each agent (relative to home `~`).
fn default_root(agent: Agent) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match agent {
        Agent::Claude => home.join(".claude"),
        Agent::Codex => home.join(".codex"),
        Agent::Antigravity => home.join(".gemini/antigravity-cli"),
    }
}

/// Fixed ID for builtin profiles.
fn builtin_id(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "builtin-claude",
        Agent::Antigravity => "builtin-antigravity",
        Agent::Codex => "builtin-codex",
    }
}

/// Capitalized display name of the agent (used in radio forms and default profile names).
pub fn agent_display_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude",
        Agent::Antigravity => "Antigravity",
        Agent::Codex => "Codex",
    }
}

/// Unique ID for additional profiles (based on epoch milliseconds).
pub fn gen_id() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("profile-{ms}")
}

/// serialization structure for profiles.json.
#[derive(Debug, Serialize, Deserialize)]
struct ProfilesFile {
    version: u32,
    profiles: Vec<Profile>,
}

/// Maximum number of profiles exposed in the header and through number shortcuts.
pub const MAX_PROFILE_SHORTCUTS: usize = 5;

/// Fixed table order for built-in profiles. User profiles follow in registration order.
const BUILTIN_PROFILE_ORDER: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Antigravity];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutToggle {
    Assigned(u8),
    Removed,
    Full,
    Invalid,
}

/// List of profiles. Vector order is registration order; shortcut order is stored separately.
#[derive(Debug, Clone)]
pub struct ProfileStore {
    pub profiles: Vec<Profile>,
}

/// One persisted profile edit, applied by id onto the current file contents.
#[derive(Debug, Clone)]
pub enum ProfileChange {
    /// Replace the profile with this id, or append it when absent.
    Upsert(Profile),
    /// Remove a non-builtin profile.
    Remove(String),
    /// Header shortcut order as this instance sees it (`ordered`). Numbered
    /// profiles this instance has never loaded (`known` lacks them, i.e. another
    /// instance added them) keep a shortcut after `ordered` while slots remain.
    Shortcuts {
        ordered: Vec<String>,
        known: Vec<String>,
    },
}

impl ProfileStore {
    /// Loads profiles.json. If missing, seeds the default 3 builtin profiles and saves immediately.
    /// Missing builtin profiles (e.g. deleted via manual edits) are restored on reload.
    pub fn load() -> Self {
        let path = profiles_file_path();
        let store = Self::load_from(&path);
        // Saving failure is non-fatal (re-tries on the next save opportunity).
        if !path.exists() {
            store.save().ok();
        }
        store
    }

    /// Startup load: an unreadable file yields the builtins (previous behavior).
    fn load_from(path: &Path) -> Self {
        Self::load_checked(path).unwrap_or_else(|_| Self::normalized(Vec::new()))
    }

    /// Loads profiles.json, reporting a read or parse failure instead of
    /// replacing it with the builtins. A missing file is the seeded default.
    /// Used by `ctrl+u` reloads and by [`Self::commit`], which must not
    /// overwrite a file it could not read.
    pub fn load_checked(path: &Path) -> anyhow::Result<Self> {
        let profiles = match std::fs::read_to_string(path) {
            Ok(data) => {
                serde_json::from_str::<ProfilesFile>(&data)
                    .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?
                    .profiles
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self::normalized(profiles))
    }

    fn normalized(mut profiles: Vec<Profile>) -> Self {
        // Expand if `~/` is present in the raw path (handles manual configuration edits).
        for p in &mut profiles {
            if let Some(s) = p.path.to_str() {
                p.path = expand(s);
            }
        }

        // Re-seed missing builtins ahead of custom profiles so builtins win path deduplication.
        for (i, agent) in BUILTIN_PROFILE_ORDER.iter().enumerate() {
            if !profiles.iter().any(|p| p.id == builtin_id(*agent)) {
                profiles.insert(i.min(profiles.len()), seed_builtin(*agent));
            }
        }

        // Dedup (agent, path) combinations, keeping only the first occurrence (handles manual edits).
        let mut seen: Vec<(Agent, PathBuf)> = Vec::new();
        profiles.retain(|p| {
            let key = (p.agent, p.path.clone());
            if seen.contains(&key) {
                false
            } else {
                seen.push(key);
                true
            }
        });

        normalize_table_order(&mut profiles);
        normalize_shortcuts(&mut profiles);

        ProfileStore { profiles }
    }

    /// Saves to profiles.json. Restricts permissions to 0600 on Unix systems due to raw tokens.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&profiles_file_path())
    }

    /// Atomic replace with a `0600` file from the start (it holds raw tokens),
    /// so a concurrently reloading instance never reads a partial file.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let file = ProfilesFile {
            version: 1,
            profiles: self.profiles.clone(),
        };
        let data = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        crate::store_lock::replace_file(path, &data).map_err(std::io::Error::other)
    }

    fn apply(&mut self, change: &ProfileChange) {
        match change {
            ProfileChange::Upsert(profile) => {
                match self.profiles.iter_mut().find(|p| p.id == profile.id) {
                    Some(stored) => *stored = profile.clone(),
                    None => self.profiles.push(profile.clone()),
                }
                normalize_table_order(&mut self.profiles);
            }
            ProfileChange::Remove(id) => self.profiles.retain(|p| p.builtin || &p.id != id),
            ProfileChange::Shortcuts { ordered, known } => {
                let mut ids: Vec<String> = ordered
                    .iter()
                    .filter(|id| self.find(id).is_some())
                    .cloned()
                    .collect();
                let unseen: Vec<String> = self
                    .numbered_profiles()
                    .into_iter()
                    .filter(|p| !known.contains(&p.id) && !ids.contains(&p.id))
                    .map(|p| p.id.clone())
                    .collect();
                ids.extend(unseen);
                ids.truncate(MAX_PROFILE_SHORTCUTS);
                apply_shortcut_ids(&mut self.profiles, &ids);
            }
        }
        normalize_shortcuts(&mut self.profiles);
    }

    /// Re-reads `path` under the store lock, applies `changes`, and writes it
    /// back. An unreadable file is left untouched (error).
    pub fn commit(path: &Path, changes: &[ProfileChange]) -> anyhow::Result<()> {
        crate::store_lock::with_store_lock(path, || {
            let mut store = Self::load_checked(path)?;
            for change in changes {
                store.apply(change);
            }
            store.save_to(path).map_err(Into::into)
        })
    }

    /// [`ProfileChange::Shortcuts`] describing this store's current order.
    pub fn shortcut_change(&self) -> ProfileChange {
        ProfileChange::Shortcuts {
            ordered: self
                .numbered_profiles()
                .into_iter()
                .map(|p| p.id.clone())
                .collect(),
            known: self.profiles.iter().map(|p| p.id.clone()).collect(),
        }
    }

    /// Looks up a profile by ID.
    pub fn find(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// List of numbered profiles ordered by shortcut position, independent of table order.
    pub fn numbered_profiles(&self) -> Vec<&Profile> {
        let mut numbered: Vec<(usize, &Profile)> = self
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, profile)| profile.active)
            .collect();
        numbered.sort_by_key(|(idx, profile)| (profile.shortcut.unwrap_or(u8::MAX), *idx));
        numbered
            .into_iter()
            .take(MAX_PROFILE_SHORTCUTS)
            .map(|(_, profile)| profile)
            .collect()
    }

    /// Inserts a profile at the requested numbered position and compacts the remaining positions.
    /// If all five positions are occupied, the previous fifth profile becomes unnumbered.
    pub fn assign_shortcut_slot(&mut self, profile_idx: usize, slot_idx: usize) -> bool {
        if profile_idx >= self.profiles.len() || slot_idx >= MAX_PROFILE_SHORTCUTS {
            return false;
        }

        normalize_shortcuts(&mut self.profiles);
        let selected_id = self.profiles[profile_idx].id.clone();
        let mut ordered_ids: Vec<String> = self
            .numbered_profiles()
            .into_iter()
            .filter(|profile| profile.id != selected_id)
            .map(|profile| profile.id.clone())
            .collect();
        ordered_ids.insert(slot_idx.min(ordered_ids.len()), selected_id);
        ordered_ids.truncate(MAX_PROFILE_SHORTCUTS);
        apply_shortcut_ids(&mut self.profiles, &ordered_ids);
        true
    }

    /// Removes the selected profile from the numbered shortcuts.
    pub fn remove_shortcut(&mut self, profile_idx: usize) -> bool {
        let Some(profile) = self.profiles.get_mut(profile_idx) else {
            return false;
        };
        if !profile.active {
            return false;
        }
        profile.active = false;
        profile.shortcut = None;
        normalize_shortcuts(&mut self.profiles);
        true
    }

    /// Toggles the selected profile shortcut. New assignments append after the current last slot.
    pub fn toggle_shortcut(&mut self, profile_idx: usize) -> ShortcutToggle {
        let Some(profile) = self.profiles.get(profile_idx) else {
            return ShortcutToggle::Invalid;
        };
        if profile.active {
            self.remove_shortcut(profile_idx);
            return ShortcutToggle::Removed;
        }

        let shortcut_count = self.numbered_profiles().len();
        if shortcut_count >= MAX_PROFILE_SHORTCUTS {
            return ShortcutToggle::Full;
        }
        if self.assign_shortcut_slot(profile_idx, shortcut_count) {
            ShortcutToggle::Assigned((shortcut_count + 1) as u8)
        } else {
            ShortcutToggle::Invalid
        }
    }

    /// Checks for (agent, expanded path) duplicates. `exclude_id` ignores the profile being edited.
    pub fn duplicate_exists(&self, agent: Agent, path: &Path, exclude_id: Option<&str>) -> bool {
        self.profiles
            .iter()
            .filter(|p| Some(p.id.as_str()) != exclude_id)
            .any(|p| p.agent == agent && p.path == path)
    }

    /// Checks for name duplicates (compares whitespace-trimmed values). `exclude_id` ignores the profile being edited.
    pub fn name_exists(&self, name: &str, exclude_id: Option<&str>) -> bool {
        let name = name.trim();
        self.profiles
            .iter()
            .filter(|p| Some(p.id.as_str()) != exclude_id)
            .any(|p| p.name.trim() == name)
    }
}

fn normalize_table_order(profiles: &mut Vec<Profile>) {
    let mut ordered = Vec::with_capacity(profiles.len());
    for agent in BUILTIN_PROFILE_ORDER {
        if let Some(idx) = profiles
            .iter()
            .position(|profile| profile.id == builtin_id(agent))
        {
            ordered.push(profiles.remove(idx));
        }
    }
    ordered.append(profiles);
    *profiles = ordered;
}

fn normalize_shortcuts(profiles: &mut [Profile]) {
    let mut active_indices: Vec<usize> = profiles
        .iter()
        .enumerate()
        .filter_map(|(idx, profile)| profile.active.then_some(idx))
        .collect();
    active_indices.sort_by_key(|idx| (profiles[*idx].shortcut.unwrap_or(u8::MAX), *idx));
    let ids: Vec<String> = active_indices
        .into_iter()
        .take(MAX_PROFILE_SHORTCUTS)
        .map(|idx| profiles[idx].id.clone())
        .collect();
    apply_shortcut_ids(profiles, &ids);
}

fn apply_shortcut_ids(profiles: &mut [Profile], ordered_ids: &[String]) {
    for profile in profiles.iter_mut() {
        profile.active = false;
        profile.shortcut = None;
    }
    for (idx, id) in ordered_ids.iter().enumerate() {
        if let Some(profile) = profiles.iter_mut().find(|profile| profile.id == *id) {
            profile.active = true;
            profile.shortcut = Some((idx + 1) as u8);
        }
    }
}

/// Seeds a builtin profile at the agent's default config root.
/// (The former config.toml dir-override absorption was removed together with
/// the legacy session dir keys — custom roots are extra profiles now.)
fn seed_builtin(agent: Agent) -> Profile {
    Profile {
        id: builtin_id(agent).to_string(),
        agent,
        name: agent_display_name(agent).to_string(),
        path: default_root(agent),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: true,
    }
}

/// profiles.json path: `~/.config/s7s/profiles.json`.
pub fn profiles_file_path() -> PathBuf {
    config_base_dir().join("profiles.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_builtin_uses_default_roots() {
        let home = dirs::home_dir().unwrap();
        let store = ProfileStore::load_from(Path::new("/nonexistent/profiles.json"));
        assert_eq!(store.profiles.len(), 3);
        assert!(store.profiles.iter().all(|p| p.builtin && p.active));
        assert_eq!(
            store
                .profiles
                .iter()
                .map(|profile| profile.agent)
                .collect::<Vec<_>>(),
            vec![Agent::Claude, Agent::Codex, Agent::Antigravity]
        );
        let claude = store.find("builtin-claude").unwrap();
        assert_eq!(claude.path, home.join(".claude"));
        assert_eq!(claude.sessions_dir(), home.join(".claude/projects"));
        let codex = store.find("builtin-codex").unwrap();
        assert_eq!(codex.path, home.join(".codex"));
        assert_eq!(codex.sessions_dir(), home.join(".codex/sessions"));
        let agy = store.find("builtin-antigravity").unwrap();
        assert_eq!(agy.path, home.join(".gemini/antigravity-cli"));
        assert_eq!(agy.sessions_dir(), agy.path);
    }

    #[test]
    fn env_var_mapping() {
        let store = ProfileStore::load_from(Path::new("/nonexistent/profiles.json"));
        // Default path profiles have no env injected (avoids re-login prompts under explicit config).
        let claude = store.find("builtin-claude").unwrap();
        assert!(claude.env_var().is_none());
        let agy = store.find("builtin-antigravity").unwrap();
        assert!(agy.env_var().is_none());
        assert!(agy.is_default_root());

        // Only inject env variables for non-default config paths.
        let team = Profile {
            id: "p".into(),
            agent: Agent::Claude,
            name: "Team".into(),
            path: PathBuf::from("/tmp/claude-team"),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        };
        assert_eq!(team.env_var().unwrap().0, "CLAUDE_CONFIG_DIR");
        let codex2 = Profile {
            agent: Agent::Codex,
            path: PathBuf::from("/tmp/codex-2"),
            ..team.clone()
        };
        assert_eq!(codex2.env_var().unwrap().0, "CODEX_HOME");
    }

    #[test]
    fn save_load_roundtrip_and_builtin_reseed() {
        let dir = std::env::temp_dir().join(format!("ular-profile-test-{}", std::process::id()));
        let file = dir.join("profiles.json");
        let mut store = ProfileStore::load_from(&file);
        store.profiles.push(Profile {
            id: "profile-x".into(),
            agent: Agent::Claude,
            name: "Claude-Team".into(),
            path: PathBuf::from("/tmp/claude-team"),
            oauth_token: Some("tok".into()),
            active: false,
            shortcut: None,
            builtin: false,
        });
        // Remove one builtin -> verify it is re-seeded on load.
        store.profiles.retain(|p| p.id != "builtin-codex");
        store.save_to(&file).unwrap();

        let loaded = ProfileStore::load_from(&file);
        assert!(loaded.find("builtin-codex").is_some());
        let extra = loaded.find("profile-x").unwrap();
        assert_eq!(extra.name, "Claude-Team");
        assert_eq!(extra.oauth_token.as_deref(), Some("tok"));
        assert!(!extra.active);
        assert!(loaded.duplicate_exists(Agent::Claude, Path::new("/tmp/claude-team"), None));
        assert!(!loaded.duplicate_exists(
            Agent::Claude,
            Path::new("/tmp/claude-team"),
            Some("profile-x")
        ));
        assert!(loaded.name_exists("Claude-Team", None));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_agent_path_deduped_on_load() {
        let dir = std::env::temp_dir().join(format!("ular-profile-dedup-{}", std::process::id()));
        let file = dir.join("profiles.json");
        let mut store = ProfileStore::load_from(&file);
        let dup = Profile {
            id: "profile-dup".into(),
            agent: Agent::Claude,
            name: "Dup".into(),
            path: store.find("builtin-claude").unwrap().path.clone(),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        };
        store.profiles.push(dup);
        store.save_to(&file).unwrap();
        let loaded = ProfileStore::load_from(&file);
        assert!(loaded.find("profile-dup").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_pins_builtins_first_and_preserves_custom_registration_order() {
        let dir = std::env::temp_dir().join(format!("ular-profile-order-{}", std::process::id()));
        let file = dir.join("profiles.json");
        let mut custom_one = test_profile("custom-one", false);
        custom_one.path = PathBuf::from("/tmp/custom-one");
        let mut custom_two = test_profile("custom-two", false);
        custom_two.path = PathBuf::from("/tmp/custom-two");
        let store = ProfileStore {
            profiles: vec![
                custom_one,
                seed_builtin(Agent::Antigravity),
                custom_two,
                seed_builtin(Agent::Claude),
                seed_builtin(Agent::Codex),
            ],
        };
        store.save_to(&file).unwrap();

        let loaded = ProfileStore::load_from(&file);
        assert_eq!(
            loaded
                .profiles
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "builtin-claude",
                "builtin-codex",
                "builtin-antigravity",
                "custom-one",
                "custom-two"
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn test_profile(name: &str, active: bool) -> Profile {
        Profile {
            id: name.to_string(),
            agent: Agent::Claude,
            name: name.to_string(),
            path: PathBuf::from(format!("/tmp/{name}")),
            oauth_token: None,
            active,
            shortcut: None,
            builtin: false,
        }
    }

    #[test]
    fn assigning_second_slot_shifts_and_evicts_fifth() {
        let mut store = ProfileStore {
            profiles: ["one", "two", "three", "four", "five"]
                .into_iter()
                .map(|name| test_profile(name, true))
                .chain(std::iter::once(test_profile("new", false)))
                .collect(),
        };
        let registration_order: Vec<String> = store.profiles.iter().map(|p| p.id.clone()).collect();

        assert!(store.assign_shortcut_slot(5, 1));
        assert_eq!(
            store
                .profiles
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
            registration_order,
            "Assigning a shortcut must not reorder the profile table"
        );
        assert_eq!(
            store
                .numbered_profiles()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "new", "two", "three", "four"]
        );
        assert!(!store.find("five").unwrap().active);
    }

    #[test]
    fn deactivating_second_slot_compacts_following_numbers() {
        let mut store = ProfileStore {
            profiles: ["one", "two", "three", "four", "five"]
                .into_iter()
                .map(|name| test_profile(name, true))
                .collect(),
        };
        let registration_order: Vec<String> = store.profiles.iter().map(|p| p.id.clone()).collect();

        assert!(store.remove_shortcut(1));
        assert_eq!(
            store
                .profiles
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
            registration_order,
            "Removing a shortcut must not reorder the profile table"
        );
        assert_eq!(
            store
                .numbered_profiles()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "three", "four", "five"]
        );
        assert!(!store.remove_shortcut(1));
    }

    #[test]
    fn numbered_profiles_never_expose_more_than_five() {
        let mut profiles: Vec<Profile> = (1..=6)
            .map(|n| test_profile(&format!("p{n}"), true))
            .collect();
        normalize_shortcuts(&mut profiles);
        let store = ProfileStore { profiles };

        assert_eq!(store.numbered_profiles().len(), MAX_PROFILE_SHORTCUTS);
        assert!(!store.find("p6").unwrap().active);
    }

    #[test]
    fn space_toggle_appends_removes_and_rejects_when_full() {
        let mut store = ProfileStore {
            profiles: ["one", "two"]
                .into_iter()
                .map(|name| test_profile(name, true))
                .chain(std::iter::once(test_profile("new", false)))
                .collect(),
        };
        assert_eq!(store.toggle_shortcut(2), ShortcutToggle::Assigned(3));
        assert_eq!(
            store
                .numbered_profiles()
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two", "new"]
        );
        assert_eq!(store.toggle_shortcut(2), ShortcutToggle::Removed);

        let mut full = ProfileStore {
            profiles: ["one", "two", "three", "four", "five"]
                .into_iter()
                .map(|name| test_profile(name, true))
                .chain(std::iter::once(test_profile("new", false)))
                .collect(),
        };
        assert_eq!(full.toggle_shortcut(5), ShortcutToggle::Full);
        assert!(!full.find("new").unwrap().active);
    }
}
