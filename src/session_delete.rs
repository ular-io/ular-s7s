//! Session artifact deletion: removes the on-disk files a session is made of.
//!
//! Shared by the TUI delete action and `s7s session delete`, so both paths obey
//! the same profile-scoping rule: every auxiliary store written to is derived
//! from the config root of the profile the session belongs to (`Profile.path`).
//! A session whose profile is gone means a stale list, so the auxiliary cleanup
//! is skipped rather than falling back to a default root — that would delete
//! another account's metadata.
//!
//! Deletion is irreversible: the session transcript is removed, not archived.

use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Removes the transcript and agent-specific sidecars of one session.
///
/// The transcript is authoritative: its removal failing is an error, while
/// auxiliary cleanup (Antigravity metadata, sqlite sidecars) is best effort so
/// a partially-written store cannot block the delete.
pub fn delete_session_artifacts(profiles: &ProfileStore, session: &Session) -> Result<()> {
    let Some(source_path) = session.source_path.as_ref() else {
        return Err(anyhow!("source path is missing"));
    };

    remove_file_best_effort(source_path)
        .with_context(|| format!("remove {}", source_path.display()))?;

    if session.agent == Agent::Antigravity {
        // Best-effort cache cleanup; skipped when the owning profile is gone
        // (never touch another profile's metadata store).
        if let Some(root) = session_profile_root(profiles, session) {
            let _ = remove_antigravity_metadata(&root, session.id.as_str());
        }
        remove_sqlite_sidecars(source_path);
        // This s7s-owned fallback is independent of the external profile root,
        // so remove it even if that profile disappeared after the last scan.
        let _ = crate::session_workspace::remove(
            &crate::config::session_workspaces_path(),
            &session.profile_id,
            &session.id,
        );
    }

    Ok(())
}

/// Config root (`Profile.path`) of the profile a session belongs to.
/// Sessions are re-stamped with live profile ids on every scan, so a miss
/// means a stale list — callers must not fall back to the default root.
pub fn session_profile_root(profiles: &ProfileStore, session: &Session) -> Option<PathBuf> {
    profiles.find(&session.profile_id).map(|p| p.path.clone())
}

fn remove_file_best_effort(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn remove_sqlite_sidecars(db_path: &Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = db_path.to_path_buf();
        let name = match db_path.file_name().and_then(|s| s.to_str()) {
            Some(name) => format!("{name}{suffix}"),
            None => continue,
        };
        sidecar.set_file_name(name);
        let _ = fs::remove_file(sidecar);
    }
}

fn remove_antigravity_metadata(profile_root: &Path, id: &str) -> Result<()> {
    let path = profile_root.join("cache/conversation_metadata.json");
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let mut root: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(conversations) = root
        .get_mut("conversations")
        .and_then(serde_json::Value::as_object_mut)
    {
        conversations.remove(id);
        let bytes = serde_json::to_vec_pretty(&root)?;
        fs::write(&path, bytes)?;
    }
    Ok(())
}
