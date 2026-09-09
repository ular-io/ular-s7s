//! s7s-owned working-directory metadata for sessions whose agent store omits it.
//!
//! Antigravity `--print` runs in the requested directory but does not persist
//! that cwd in the conversation DB. For handoffs created by s7s, the launch
//! path is known exactly, so this store keeps the `(profile, conversation) ->
//! cwd` fact independently of Antigravity's disposable caches.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const STORE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WorkspaceStore {
    version: u32,
    profiles: BTreeMap<String, BTreeMap<String, PathBuf>>,
}

impl Default for WorkspaceStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            profiles: BTreeMap::new(),
        }
    }
}

impl WorkspaceStore {
    /// Loads the store for scan-time fallback. Missing, malformed, or newer
    /// files are ignored so session scanning remains available.
    pub(crate) fn load(path: &Path) -> Self {
        Self::load_checked(path).unwrap_or_default()
    }

    pub(crate) fn cwd(&self, profile_id: &str, session_id: &str) -> Option<&Path> {
        self.profiles
            .get(profile_id)?
            .get(session_id)
            .map(PathBuf::as_path)
    }

    fn load_checked(path: &Path) -> Result<Self> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err.into()),
        };
        let store: Self =
            serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
        if store.version != STORE_VERSION {
            return Err(anyhow!(
                "unsupported session workspace version {}",
                store.version
            ));
        }
        Ok(store)
    }

    fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("workspace store has no parent: {}", path.display()))?;
        fs::create_dir_all(parent)?;
        let data = serde_json::to_vec_pretty(self)?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("session_workspaces.json");
        let temp = parent.join(format!(".{name}.tmp-{}", std::process::id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| -> Result<()> {
            let mut file = options.open(&temp)?;
            file.write_all(&data)?;
            file.sync_all()?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

/// Persists one cwd without discarding mappings captured by earlier handoffs.
pub(crate) fn record(path: &Path, profile_id: &str, session_id: &str, cwd: &Path) -> Result<()> {
    let mut store = WorkspaceStore::load_checked(path)?;
    store
        .profiles
        .entry(profile_id.to_string())
        .or_default()
        .insert(session_id.to_string(), cwd.to_path_buf());
    store.save(path)
}

/// Removes one mapping after an explicit s7s session deletion.
pub(crate) fn remove(path: &Path, profile_id: &str, session_id: &str) -> Result<()> {
    let mut store = WorkspaceStore::load_checked(path)?;
    let mut changed = false;
    let mut remove_profile = false;
    if let Some(sessions) = store.profiles.get_mut(profile_id) {
        changed = sessions.remove(session_id).is_some();
        remove_profile = sessions.is_empty();
    }
    if remove_profile {
        store.profiles.remove(profile_id);
    }
    if changed {
        if store.profiles.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        } else {
            store.save(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "s7s-session-workspace-{label}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn records_multiple_sessions_in_the_same_folder_and_removes_one() {
        let path = temp_path("roundtrip");
        record(&path, "agy-a", "session-1", Path::new("/tmp/project")).expect("record first");
        record(&path, "agy-a", "session-2", Path::new("/tmp/project")).expect("record second");

        let store = WorkspaceStore::load(&path);
        assert_eq!(
            store.cwd("agy-a", "session-1"),
            Some(Path::new("/tmp/project"))
        );
        assert_eq!(
            store.cwd("agy-a", "session-2"),
            Some(Path::new("/tmp/project"))
        );

        remove(&path, "agy-a", "session-1").expect("remove first");
        let store = WorkspaceStore::load(&path);
        assert!(store.cwd("agy-a", "session-1").is_none());
        assert_eq!(
            store.cwd("agy-a", "session-2"),
            Some(Path::new("/tmp/project"))
        );

        remove(&path, "agy-a", "session-2").expect("remove final mapping");
        assert!(!path.exists());

        let _ = fs::remove_file(path);
    }
}
