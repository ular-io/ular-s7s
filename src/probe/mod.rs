//! Neutral PTY/process probe layer shared by the usage and model queries.
//!
//! Owns only the generic mechanics of driving an agent CLI: PTY lifecycle and
//! screen capture (`pty`), process discovery/termination helpers (`process`),
//! and the CLI helpers shared by more than one probe client (this module).
//! It must not know usage labels or model syntax — screen-to-domain parsing,
//! fallback/cache policy, and demo-mode guards stay in the feature modules
//! (`usage.rs`, `models.rs`), which are independent clients of this layer.

pub(crate) mod process;
pub(crate) mod pty;

use std::path::{Path, PathBuf};

/// Marker for Claude boot completion. Prefers stable, always-visible markers (mode toggles/footer)
/// over ephemeral welcome messages. Shared by `/usage` and `/model` queries.
pub(crate) const CLAUDE_READY_MARKERS: &[&str] = &[
    "shift+tab to cycle",
    "auto mode",
    "for shortcuts",
    "Tips for getting started",
];

/// Working directory for a probe child process, plus whether s7s owns it.
pub(crate) struct ProbeCwd {
    pub(crate) path: PathBuf,
    /// Whether `path` is the dedicated probe folder s7s created for itself.
    /// Only there may a folder-trust dialog be auto-confirmed — the fallback is
    /// the user's own working directory, where that decision is theirs to make.
    pub(crate) dedicated: bool,
}

/// Working directory handed to every probe child, created on first use.
///
/// Never the s7s launch directory: a folder that pre-approves permissions in
/// `.claude/settings.json` makes claude open its trust dialog instead of booting,
/// failing the query for as long as s7s is started there (see `config::probe_dir`).
/// Falls back to the process directory if the fixed folder cannot be created, so a
/// probe stays best-effort rather than dead.
pub(crate) fn probe_cwd() -> Option<ProbeCwd> {
    let dir = crate::config::probe_dir();
    match std::fs::create_dir_all(&dir) {
        Ok(()) => Some(ProbeCwd {
            path: dir,
            dedicated: true,
        }),
        Err(_) => std::env::current_dir().ok().map(|path| ProbeCwd {
            path,
            dedicated: false,
        }),
    }
}

/// JSON `loggedIn` value from `claude auth status`. Returns None on execution failure or parsing error.
pub(crate) fn claude_logged_in(envs: &[(&str, &Path)]) -> Option<bool> {
    let mut cmd = std::process::Command::new("claude");
    cmd.args(["auth", "status"]);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let out = cmd.output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("loggedIn")?.as_bool()
}
