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

use std::path::Path;

/// Marker for Claude boot completion. Prefers stable, always-visible markers (mode toggles/footer)
/// over ephemeral welcome messages. Shared by `/usage` and `/model` queries.
pub(crate) const CLAUDE_READY_MARKERS: &[&str] = &[
    "shift+tab to cycle",
    "auto mode",
    "for shortcuts",
    "Tips for getting started",
];

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
