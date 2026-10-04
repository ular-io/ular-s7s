//! Shared folder-change validation and persistence for the TUI and session CLI.

use crate::model::{Agent, Session};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub(crate) fn validate_agent(agent: Agent) -> Result<()> {
    if agent == Agent::Antigravity {
        bail!("Antigravity keeps a resumed session in its original folder — changing it here would not move the work");
    }
    Ok(())
}

/// Resolve an existing directory; callers own any UI-specific name expansion.
pub(crate) fn validate_folder(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("Enter a folder path");
    }
    let folder = std::fs::canonicalize(path).context("Cannot open path")?;
    if !folder.is_dir() {
        bail!("Path is not a directory");
    }
    Ok(folder)
}

/// Validate the entire batch before touching the store. Running sessions keep
/// their current cwd; only future s7s launches consume the saved override.
pub(crate) fn change(sessions: &[&Session], folder: &Path, store: &Path) -> Result<()> {
    if sessions.is_empty() {
        bail!("No sessions selected");
    }
    for session in sessions {
        validate_agent(session.agent)?;
    }
    let folder = validate_folder(folder)?;
    let targets: Vec<_> = sessions
        .iter()
        .map(|s| (s.profile_id.as_str(), s.id.as_str()))
        .collect();
    crate::session_workspace::record_many(store, &targets, &folder)
}
