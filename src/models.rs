//! Querying and caching of selectable models per agent CLI (for the New Session model dropdown).
//!
//! Methods of enumeration (verified in 2026-08, see docs/models.md):
//! - claude: Lacks an enumeration command, so we scrape the `/model` screen via PTY (sharing
//!   the same driver `probe::pty::drive_screen` with usage). Since the list depends on plans/accounts,
//!   we query it per profile (injecting `CLAUDE_CONFIG_DIR`) and obtain the current default model
//!   marked with ✔.
//! - codex: `codex debug models` prints a JSON catalog (only `visibility=="list"` is used).
//!   The default model is the top-level `model` key in `<CODEX_HOME>/config.toml`.
//! - agy: `agy models` prints tab-separated slug/display-name rows (legacy versions printed only
//!   display names). Since environment injection is not supported, we only query the default path
//!   profile, and other profiles share its result (fallback in `ModelCatalog::for_profile`). The
//!   default model is the top-level `model` key in `settings.json`.
//!
//! Because CLIs do not validate invalid model names (agy silently falls back to the default model,
//! codex does not validate on startup - verified), this module is responsible for the accuracy of the list.
//! On failure, the existing cache is preserved.
//!
//! Cache is located at `~/.config/s7s/models.json` (keyed by profile ID). Since model lists only change
//! on CLI upgrades or plan changes, we skip querying on startup if the cached CLI version matches the current `--version`
//! (version gate). Ctrl+U triggers a forced re-query to cover plan changes.

use crate::model::Agent;
use crate::probe::process::installed;
use crate::profile::Profile;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

mod agy;
mod catalog;
mod claude;
mod codex;

use agy::fetch_agy;
pub use catalog::ModelCatalog;
use claude::fetch_claude;
use codex::fetch_codex;

/// A single model entry to display in the dropdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    /// String to pass as CLI `--model` argument (claude=ID or alias, codex/agy=slug).
    pub value: String,
    /// Display name on screen (claude uses original notation like "Fable", others use the same as value).
    pub label: String,
    /// Additional description (description text, empty if none).
    #[serde(default)]
    pub note: String,
}

/// The model the user last picked in the New Session dialog for a profile.
///
/// Distinct from `Option<String>` because we must remember an explicit "Default"
/// (no `--model`) pick separately from "never picked" (fall back to the CLI default):
/// `Option::None` on the field means never recorded, `Some(Default)` means the user
/// deliberately chose the CLI's own default, and `Some(Model(v))` a specific model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LastSelection {
    /// User picked the "Default" dropdown entry (no `--model` injected).
    Default,
    /// User picked a specific model (`--model <value>`).
    Model(String),
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
    /// The model the user last launched with for this profile. Takes precedence over
    /// `default_model` as the New Session dialog's initial selection (see `LastSelection`).
    /// Preserved across background re-fetches (see `ModelCatalog::insert`).
    #[serde(default)]
    pub last_selected: Option<LastSelection>,
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
                entry(
                    "gpt-5.3-codex-mini",
                    "gpt-5.3-codex-mini",
                    "faster, lighter",
                ),
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
        last_selected: None,
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
    if !installed(bin) {
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
            last_selected: None,
        }),
        // Treat empty lists as parsing failures to avoid overwriting the cache.
        Ok(_) => ModelsResult::Unavailable(format!("{bin}: empty model list")),
        Err(e) => ModelsResult::Unavailable(e.to_string()),
    }
}

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod claude_tests;
#[cfg(test)]
mod cli_tests;
