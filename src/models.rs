//! Querying and caching of selectable models per agent CLI (for the New Session model dropdown).
//!
//! Methods of enumeration (verified in 2026-07, see docs/models.md):
//! - claude: Lacks an enumeration command, so we scrape the `/model` screen via PTY (sharing
//!   the same driver `usage::drive_screen` with usage). Since the list depends on plans/accounts,
//!   we query it per profile (injecting `CLAUDE_CONFIG_DIR`) and obtain the current default model
//!   marked with ✔.
//! - codex: `codex debug models` prints a JSON catalog (only `visibility=="list"` is used).
//!   The default model is the top-level `model` key in `<CODEX_HOME>/config.toml`.
//! - agy: `agy models` prints display names line-by-line. Since environment injection is not supported,
//!   we only query the default path profile, and other profiles share its result (fallback in
//!   `ModelCatalog::for_profile`). The default model is the top-level `model` key in `settings.json`.
//!
//! Because CLIs do not validate invalid model names (agy silently falls back to the default model,
//! codex does not validate on startup - verified), this module is responsible for the accuracy of the list.
//! On failure, the existing cache is preserved.
//!
//! Cache is located at `~/.config/s7s/models.json` (keyed by profile ID). Since model lists only change
//! on CLI upgrades or plan changes, we skip querying on startup if the cached CLI version matches the current `--version`
//! (version gate). Ctrl+U triggers a forced re-query to cover plan changes.

use crate::config::config_base_dir;
use crate::model::Agent;
use crate::profile::Profile;
use crate::usage;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// A single model entry to display in the dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    /// String to pass as CLI `--model` argument (claude=alias, codex=slug, agy=display name).
    pub value: String,
    /// Display name on screen (claude uses original notation like "Fable", others use the same as value).
    pub label: String,
    /// Additional description (description text, empty if none).
    #[serde(default)]
    pub note: String,
}

/// Query result of a profile's model list (cached unit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileModels {
    pub agent: Agent,
    /// CLI version at the time of query (for startup version gate comparison). If None, always re-query.
    pub cli_version: Option<String>,
    pub models: Vec<ModelEntry>,
    /// Default model currently used by the CLI (value format). If None, CLI's own default.
    pub default_model: Option<String>,
}

/// Background query thread -> UI channel payload.
#[derive(Debug)]
pub enum ModelsResult {
    /// Query succeeded (updates cache).
    Ready(ProfileModels),
    /// Query skipped due to version gate (keeps cache).
    Skipped,
    /// Query failed or unavailable (reason). Keeps the existing cache.
    Unavailable(String),
}

/// Cache of model lists per profile (key = profile id), persisted in models.json.
#[derive(Debug, Default)]
pub struct ModelCatalog {
    entries: HashMap<String, ProfileModels>,
}

/// Serialization structure for models.json.
#[derive(Debug, Default, Serialize, Deserialize)]
struct ModelsFile {
    version: u32,
    profiles: HashMap<String, ProfileModels>,
}

impl ModelCatalog {
    /// Loads models.json. Returns an empty catalog if missing or corrupted.
    pub fn load() -> Self {
        Self::load_from(&models_file_path())
    }

    fn load_from(path: &Path) -> Self {
        let entries = std::fs::read_to_string(path)
            .ok()
            .and_then(|data| serde_json::from_str::<ModelsFile>(&data).ok())
            .map(|f| f.profiles)
            .unwrap_or_default();
        ModelCatalog { entries }
    }

    /// Saves to models.json (failures are non-fatal and can be ignored by callers).
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&models_file_path())
    }

    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = ModelsFile {
            version: 1,
            profiles: self.entries.clone(),
        };
        let data = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
        std::fs::write(path, data)
    }

    pub fn insert(&mut self, profile_id: String, models: ProfileModels) {
        self.entries.insert(profile_id, models);
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

/// Built-in fallback list for the first execution when cache is missing.
///
/// Only provided for Claude: aliases remain valid even after CLI upgrades, preventing the list from becoming stale.
/// Since Codex/Agy use fast subprocesses for enumeration, they will be filled quickly in the first background query.
/// Hardcoding model names (slug/display name) is avoided as they quickly become outdated, so no fallbacks are provided.
pub fn fallback_models(agent: Agent) -> Vec<ModelEntry> {
    match agent {
        Agent::Claude => ["Fable", "Opus", "Sonnet", "Haiku"]
            .iter()
            .map(|name| ModelEntry {
                value: name.to_ascii_lowercase(),
                label: (*name).to_string(),
                note: String::new(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// models.json path: `~/.config/s7s/models.json`.
fn models_file_path() -> PathBuf {
    config_base_dir().join("models.json")
}

/// Static demo-mode model list per agent for the New Session dropdown.
fn demo_models(agent: Agent) -> ProfileModels {
    let entry = |value: &str, label: &str, note: &str| ModelEntry {
        value: value.to_string(),
        label: label.to_string(),
        note: note.to_string(),
    };
    let (models, default_model) = match agent {
        Agent::Claude => (
            vec![
                entry("fable", "Fable", "Most intelligent for complex work"),
                entry("opus", "Opus", "Powerful all-round model"),
                entry("sonnet", "Sonnet", "Balanced speed and capability"),
                entry("haiku", "Haiku", "Fastest for simple tasks"),
            ],
            Some("fable"),
        ),
        Agent::Codex => (
            vec![
                entry("gpt-5.3-codex", "gpt-5.3-codex", "default"),
                entry("gpt-5.3-codex-mini", "gpt-5.3-codex-mini", "faster, lighter"),
            ],
            Some("gpt-5.3-codex"),
        ),
        Agent::Antigravity => (
            vec![
                entry("Gemini 3.1 Pro (High)", "Gemini 3.1 Pro (High)", ""),
                entry("Gemini 3.1 Pro (Low)", "Gemini 3.1 Pro (Low)", ""),
                entry("Gemini 3 Flash", "Gemini 3 Flash", ""),
            ],
            Some("Gemini 3.1 Pro (High)"),
        ),
    };
    ProfileModels {
        agent,
        cli_version: Some("demo".to_string()),
        models,
        default_model: default_model.map(String::from),
    }
}

/// Starts concurrent querying of model lists for profiles and returns a receiver channel of (profile_id, ModelsResult).
///
/// Profiles where `cached_versions` (profile ID -> cached CLI version) matches the current `--version`
/// will skip querying and send `Skipped` unless `force` is true. Since every target sends exactly one result,
/// the receiver can use this to track loading status.
pub fn spawn_fetch(
    targets: Vec<Profile>,
    cached_versions: HashMap<String, Option<String>>,
    force: bool,
) -> Receiver<(String, ModelsResult)> {
    let (tx, rx) = mpsc::channel();
    // Demo mode: static plausible model lists; never spawn CLI processes
    // (`--version` checks included) so the sandbox stays side-effect free.
    if crate::config::is_demo_mode() {
        for profile in targets {
            let models = demo_models(profile.agent);
            let _ = tx.send((profile.id, ModelsResult::Ready(models)));
        }
        return rx;
    }
    thread::spawn(move || {
        // Query the CLI version for each agent only once (low cost, but avoids repeating for each profile).
        let mut versions: HashMap<Agent, Option<String>> = HashMap::new();
        for agent in Agent::all() {
            if targets.iter().any(|p| p.agent == agent) {
                versions.insert(agent, cli_version(agent));
            }
        }
        let mut workers = Vec::new();
        for profile in targets {
            let version = versions.get(&profile.agent).cloned().flatten();
            // Version gate: skip query if cached version matches (Ctrl+U handles plan updates).
            if !force && version.is_some() && cached_versions.get(&profile.id) == Some(&version) {
                let _ = tx.send((profile.id, ModelsResult::Skipped));
                continue;
            }
            let tx: Sender<(String, ModelsResult)> = tx.clone();
            workers.push(thread::spawn(move || {
                let res = fetch(&profile, version);
                let _ = tx.send((profile.id, res));
            }));
        }
        for w in workers {
            let _ = w.join();
        }
    });
    rx
}

/// Debug option (`--model-probe`): Force queries and print model lists for all profiles.
/// Used for manual verification against actual CLI screen outputs (e.g. `/model`) and does not update the cache.
pub fn probe() {
    let store = crate::profile::ProfileStore::load();
    let targets: Vec<Profile> = store.profiles.to_vec();
    let names: HashMap<String, String> = targets
        .iter()
        .map(|p| (p.id.clone(), format!("{} [{}]", p.name, p.agent.label())))
        .collect();
    let count = targets.len();
    let rx = spawn_fetch(targets, HashMap::new(), true);
    for _ in 0..count {
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok((id, res)) => {
                let name = names.get(&id).cloned().unwrap_or(id);
                match res {
                    ModelsResult::Ready(pm) => {
                        println!(
                            "{name}: cli_version={} default={}",
                            pm.cli_version.as_deref().unwrap_or("?"),
                            pm.default_model.as_deref().unwrap_or("(CLI default)")
                        );
                        for m in &pm.models {
                            println!("  - {} ({})", m.value, m.note);
                        }
                    }
                    ModelsResult::Skipped => println!("{name}: skipped"),
                    ModelsResult::Unavailable(reason) => println!("{name}: unavailable — {reason}"),
                }
            }
            Err(e) => {
                println!("recv error: {e}");
                break;
            }
        }
    }
    // Give some time for the PTY cleanup thread (child CLI termination) before exiting the process.
    thread::sleep(Duration::from_secs(5));
}

/// `<bin> --version` first line. Returns None on execution failure (always queries without version gate).
fn cli_version(agent: Agent) -> Option<String> {
    let bin = agent_bin(agent);
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    (!line.is_empty()).then_some(line)
}

fn agent_bin(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Antigravity => "agy",
        Agent::Codex => "codex",
    }
}

/// Synchronously queries the model list for a single profile.
fn fetch(profile: &Profile, cli_version: Option<String>) -> ModelsResult {
    if !profile.path.is_dir() {
        return ModelsResult::Unavailable("config folder not found".to_string());
    }
    // Extra profiles of Antigravity cannot have env variables injected; they share results of the default path profile.
    if profile.agent == Agent::Antigravity && !profile.is_default_root() {
        return ModelsResult::Unavailable(
            "env injection not supported — shares the default profile's list".to_string(),
        );
    }
    let bin = agent_bin(profile.agent);
    if !usage::installed(bin) {
        return ModelsResult::Unavailable(format!("{bin} not installed"));
    }
    let envs: Vec<(&str, &Path)> = profile.env_var().into_iter().collect();
    let outcome = match profile.agent {
        Agent::Claude => fetch_claude(&envs),
        Agent::Codex => fetch_codex(&envs, &profile.path),
        Agent::Antigravity => fetch_agy(&profile.path),
    };
    match outcome {
        Ok((models, default_model)) if !models.is_empty() => ModelsResult::Ready(ProfileModels {
            agent: profile.agent,
            cli_version,
            models,
            default_model,
        }),
        // Treat empty lists as parsing failures to avoid overwriting the cache.
        Ok(_) => ModelsResult::Unavailable(format!("{bin}: empty model list")),
        Err(e) => ModelsResult::Unavailable(e.to_string()),
    }
}

/// claude: Spawns the `/model` screen via PTY to parse the list and the current default (✔).
fn fetch_claude(envs: &[(&str, &Path)]) -> Result<(Vec<ModelEntry>, Option<String>)> {
    if usage::claude_logged_in(envs) == Some(false) {
        return Err(anyhow!("claude: not logged in"));
    }
    match usage::drive_screen(
        "claude",
        "/model",
        usage::CLAUDE_READY_MARKERS,
        &["Select model"],
        &[],
        Duration::from_secs(2),
        Duration::from_millis(800),
        envs,
    )? {
        usage::DriveOutcome::Screen(text) => parse_claude_model_screen(&text)
            .ok_or_else(|| anyhow!("claude: failed to parse /model screen")),
        usage::DriveOutcome::NotLoggedIn => Err(anyhow!("claude: not logged in")),
    }
}

/// Parser for the claude `/model` screen. Reads `N. Name  Description` lines after "Select model".
///
/// - `✔` following the name denotes the current default model.
/// - The `Default (recommended)` line is excluded from the list as it overlaps with s7s's own Default entry.
///   If ✔ is on this line, default_model is None (CLI Default).
///   Value is the lowercase of the name (= `--model` alias, e.g. "Fable" -> "fable").
fn parse_claude_model_screen(text: &str) -> Option<(Vec<ModelEntry>, Option<String>)> {
    let mut models = Vec::new();
    let mut default_model = None;
    let mut seen_header = false;
    for line in text.lines() {
        let t = line.trim();
        if !seen_header {
            seen_header = t.starts_with("Select model");
            continue;
        }
        // Strip cursor prefix `❯` and check for `N. ` prefix.
        let t = t.strip_prefix('❯').map(str::trim_start).unwrap_or(t);
        let Some(dot) = t.find(". ") else { continue };
        if dot == 0 || !t[..dot].bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let rest = t[dot + 2..].trim_start();
        // Name and description are separated by 2 or more spaces.
        let (name_part, note) = match rest.find("  ") {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        let is_default = name_part.contains('✔');
        let name = name_part.replace('✔', "").trim().to_string();
        if name.is_empty() {
            continue;
        }
        if name.to_ascii_lowercase().starts_with("default") {
            continue; // CLI's own Default line - keep default_model as None even if checked.
        }
        let value = name.to_ascii_lowercase();
        if is_default {
            default_model = Some(value.clone());
        }
        models.push(ModelEntry {
            value,
            label: name,
            note: note.to_string(),
        });
    }
    (!models.is_empty()).then_some((models, default_model))
}

/// codex: Filters visibility=="list" entries from `codex debug models` JSON catalog.
/// Default model is the top-level `model` key in `<CODEX_HOME>/config.toml`.
fn fetch_codex(
    envs: &[(&str, &Path)],
    config_root: &Path,
) -> Result<(Vec<ModelEntry>, Option<String>)> {
    let mut cmd = std::process::Command::new("codex");
    cmd.args(["debug", "models"]);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "codex debug models failed (exit {})",
            out.status.code().unwrap_or(-1)
        ));
    }
    let models = parse_codex_models(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| anyhow!("codex: failed to parse debug models JSON"))?;
    let default_model = codex_default_model(&config_root.join("config.toml"));
    Ok((models, default_model))
}

/// JSON parser for codex model catalog (`{"models":[{slug, display_name, ...}]}`).
fn parse_codex_models(json: &str) -> Option<Vec<ModelEntry>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let models = v.get("models")?.as_array()?;
    Some(
        models
            .iter()
            .filter(|m| m.get("visibility").and_then(|s| s.as_str()) == Some("list"))
            .filter_map(|m| {
                let slug = m.get("slug")?.as_str()?;
                let label = m
                    .get("display_name")
                    .and_then(|s| s.as_str())
                    .unwrap_or(slug);
                let note = m
                    .get("description")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default();
                Some(ModelEntry {
                    value: slug.to_string(),
                    label: label.to_string(),
                    note: note.to_string(),
                })
            })
            .collect(),
    )
}

/// Read top-level `model = "..."` value in config.toml. Parses lines before section `[`
/// entries without a toml dependency (excludes prefix-similar keys like `model_reasoning_effort` via `=` validation).
fn codex_default_model(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            break;
        }
        if let Some(rest) = t.strip_prefix("model") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim().trim_matches('"').to_string();
                return (!value.is_empty()).then_some(value);
            }
        }
    }
    None
}

/// agy: Line-separated display names from `agy models`. Default model is the top-level
/// `model` key in settings.json (verified to share the same format as display name).
fn fetch_agy(config_root: &Path) -> Result<(Vec<ModelEntry>, Option<String>)> {
    let out = std::process::Command::new("agy").arg("models").output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "agy models failed (exit {})",
            out.status.code().unwrap_or(-1)
        ));
    }
    let models: Vec<ModelEntry> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| ModelEntry {
            value: l.to_string(),
            label: l.to_string(),
            note: String::new(),
        })
        .collect();
    let default_model = agy_default_model(&config_root.join("settings.json"));
    Ok((models, default_model))
}

/// Read top-level `model` key from agy settings.json.
fn agy_default_model(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("model")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured vt100 output of claude 2.1.207 `/model` screen from 2026-07-14.
    const CLAUDE_MODEL_SCREEN: &str = "\
uu
 ▐▛███▜▌   Claude Code v2.1.207
▝▜█████▛▘  Fable 5 · Claude Team
  ▘▘ ▝▝    ~/projects/my-app

   Select model
   Switch between Claude models. Your pick becomes the default for new sessions. For other/previous model names, specify with --model.

     1. Default (recommended)  Opus 4.8 with 1M context · Best for everyday, complex tasks
     2. Opus                   Opus 4.8 with 1M context · Best for everyday, complex tasks
   ❯ 3. Fable ✔                Fable 5 · Most capable for your hardest and longest-running tasks
     4. Sonnet                 Sonnet 5 · Efficient for routine tasks
     5. Haiku                  Haiku 4.5 · Fastest for quick answers

   ● High effort (default) ←/→ to adjust

   Enter to set as default · s to use this session only · Esc to cancel
";

    #[test]
    fn parse_claude_model_screen_extracts_aliases_and_default() {
        let (models, default_model) = parse_claude_model_screen(CLAUDE_MODEL_SCREEN).unwrap();
        let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
        // Excludes "Default (recommended)" row; others are lowercase aliases.
        assert_eq!(values, ["opus", "fable", "sonnet", "haiku"]);
        assert_eq!(default_model.as_deref(), Some("fable"));
        let fable = models.iter().find(|m| m.value == "fable").unwrap();
        assert_eq!(fable.label, "Fable");
        assert!(fable.note.starts_with("Fable 5"));
    }

    #[test]
    fn parse_claude_model_screen_default_row_checked_means_cli_default() {
        // If ✔ is on the Default row, default_model is None (relying on CLI Default).
        let screen = "\
   Select model

   ❯ 1. Default (recommended) ✔  Opus 4.8 · Best for everyday tasks
     2. Sonnet                   Sonnet 5 · Efficient for routine tasks
";
        let (models, default_model) = parse_claude_model_screen(screen).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].value, "sonnet");
        assert_eq!(default_model, None);
    }

    #[test]
    fn parse_claude_model_screen_requires_header() {
        // Numbered lines before "Select model" header (e.g. boot banners) are ignored.
        assert!(parse_claude_model_screen("  1. Fable  Fable 5\n").is_none());
    }

    #[test]
    fn parse_codex_models_filters_hidden_entries() {
        let json = r#"{"models":[
            {"slug":"gpt-5.6-sol","display_name":"GPT-5.6-Sol","description":"Latest frontier agentic coding model.","visibility":"list"},
            {"slug":"gpt-5.4","display_name":"GPT-5.4","description":"","visibility":"list"},
            {"slug":"codex-auto-review","display_name":"Codex Auto Review","visibility":"hide"}
        ]}"#;
        let models = parse_codex_models(json).unwrap();
        let values: Vec<&str> = models.iter().map(|m| m.value.as_str()).collect();
        assert_eq!(values, ["gpt-5.6-sol", "gpt-5.4"]);
        assert_eq!(models[0].label, "GPT-5.6-Sol");
        assert!(models[0].note.starts_with("Latest frontier"));
    }

    #[test]
    fn codex_default_model_reads_top_level_key_only() {
        let dir = std::env::temp_dir().join(format!("ular-models-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // Skips model_reasoning_effort (similar prefix key) and ignores model key below section headers.
        std::fs::write(
            &path,
            "model_reasoning_effort = \"medium\"\nmodel = \"gpt-5.6-sol\"\n\n[features]\nmodel = \"ignored\"\n",
        )
        .unwrap();
        assert_eq!(codex_default_model(&path).as_deref(), Some("gpt-5.6-sol"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn catalog_falls_back_to_default_antigravity_entry() {
        let mut catalog = ModelCatalog::default();
        catalog.insert(
            "builtin-antigravity".to_string(),
            ProfileModels {
                agent: Agent::Antigravity,
                cli_version: Some("1.1.2".to_string()),
                models: vec![ModelEntry {
                    value: "Gemini 3.1 Pro (High)".to_string(),
                    label: "Gemini 3.1 Pro (High)".to_string(),
                    note: String::new(),
                }],
                default_model: Some("Gemini 3.5 Flash (High)".to_string()),
            },
        );
        // Extra profiles of agy that cannot have env injected share the default profile's model list.
        let extra = Profile {
            id: "profile-agy2".to_string(),
            agent: Agent::Antigravity,
            name: "Agy2".to_string(),
            path: PathBuf::from("/tmp/agy2"),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        };
        let pm = catalog.for_profile(&extra).unwrap();
        assert_eq!(pm.models.len(), 1);

        // Other agents do not fallback.
        let claude = Profile {
            id: "profile-claude2".to_string(),
            agent: Agent::Claude,
            path: PathBuf::from("/tmp/claude2"),
            ..extra.clone()
        };
        assert!(catalog.for_profile(&claude).is_none());
    }

    #[test]
    fn catalog_roundtrip_save_load() {
        let dir = std::env::temp_dir().join(format!("ular-models-cache-{}", std::process::id()));
        let path = dir.join("models.json");
        let mut catalog = ModelCatalog::default();
        catalog.insert(
            "builtin-codex".to_string(),
            ProfileModels {
                agent: Agent::Codex,
                cli_version: Some("codex-cli 0.144.3".to_string()),
                models: vec![ModelEntry {
                    value: "gpt-5.6-sol".to_string(),
                    label: "GPT-5.6-Sol".to_string(),
                    note: String::new(),
                }],
                default_model: Some("gpt-5.6-sol".to_string()),
            },
        );
        catalog.save_to(&path).unwrap();
        let loaded = ModelCatalog::load_from(&path);
        assert_eq!(
            loaded.cached_version("builtin-codex"),
            Some(Some("codex-cli 0.144.3".to_string()))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fallback_models_only_for_claude() {
        let claude = fallback_models(Agent::Claude);
        assert!(claude.iter().any(|m| m.value == "fable"));
        assert!(claude
            .iter()
            .all(|m| m.value == m.label.to_ascii_lowercase()));
        assert!(fallback_models(Agent::Codex).is_empty());
        assert!(fallback_models(Agent::Antigravity).is_empty());
    }
}
