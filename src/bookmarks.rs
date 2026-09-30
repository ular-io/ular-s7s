//! Durable, s7s-owned bookmarks. Agent transcripts and the disposable index are
//! not modified; identity includes the agent, profile, and session ID.

use crate::model::Session;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;

const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct BookmarkKey {
    agent: String,
    profile: String,
    session: String,
}

impl From<&Session> for BookmarkKey {
    fn from(session: &Session) -> Self {
        Self {
            agent: session.agent.key().to_string(),
            profile: session.profile_id.clone(),
            session: session.id.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BookmarkStore {
    version: u32,
    entries: BTreeSet<BookmarkKey>,
}

impl Default for BookmarkStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            entries: BTreeSet::new(),
        }
    }
}

impl BookmarkStore {
    /// Read errors are surfaced rather than allowing a later toggle to overwrite
    /// an unreadable or newer store with an empty one.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err.into()),
        };
        let store: Self =
            serde_json::from_slice(&data).with_context(|| format!("parse {}", path.display()))?;
        if store.version != STORE_VERSION {
            bail!("unsupported bookmark version {}", store.version);
        }
        Ok(store)
    }

    pub(crate) fn contains(&self, session: &Session) -> bool {
        self.entries.contains(&BookmarkKey::from(session))
    }

    pub(crate) fn set(&mut self, session: &Session, bookmarked: bool) {
        let key = BookmarkKey::from(session);
        if bookmarked {
            self.entries.insert(key);
        } else {
            self.entries.remove(&key);
        }
    }

    /// Re-read before writing so another app instance's completed changes are
    /// retained. Return the updated state only after the atomic write succeeds.
    pub(crate) fn toggle(path: &Path, session: &Session) -> Result<Self> {
        let mut store = Self::load(path)?;
        store.set(session, !store.contains(session));
        store.save(path)?;
        Ok(store)
    }

    /// Remove only this identity, retaining other instances' completed changes.
    /// An absent entry does not create or rewrite a store.
    pub(crate) fn remove(path: &Path, session: &Session) -> Result<Self> {
        let mut store = Self::load(path)?;
        if store.contains(session) {
            store.set(session, false);
            store.save(path)?;
        }
        Ok(store)
    }

    fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("bookmark store has no parent")?;
        fs::create_dir_all(parent)?;
        let data = serde_json::to_vec_pretty(self)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temp = parent.join(format!(".bookmarks.tmp-{}-{nonce}", std::process::id()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        let result = (|| -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Agent;
    use crate::ui::test_support::app_with_session;

    #[test]
    fn bookmarks_survive_reload_and_are_scoped_to_agent_profile_and_id() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let session = app_with_session().sessions.remove(0);
        let mut other_profile = session.clone();
        other_profile.profile_id = "other".to_string();
        let mut other_agent = session.clone();
        other_agent.agent = Agent::Claude;

        BookmarkStore::toggle(&root.path, &session).unwrap();
        let loaded = BookmarkStore::load(&root.path).unwrap();
        assert!(loaded.contains(&session));
        assert!(!loaded.contains(&other_profile));
        assert!(!loaded.contains(&other_agent));

        BookmarkStore::toggle(&root.path, &other_profile).unwrap();
        BookmarkStore::toggle(&root.path, &other_agent).unwrap();
        BookmarkStore::toggle(&root.path, &session).unwrap();
        let loaded = BookmarkStore::load(&root.path).unwrap();
        assert!(!loaded.contains(&session));
        assert!(loaded.contains(&other_profile));
        assert!(loaded.contains(&other_agent));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn unreadable_or_newer_store_is_never_overwritten() {
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let session = app_with_session().sessions.remove(0);
        for data in ["not json", r#"{"version":999,"entries":[]}"#] {
            fs::write(&root.path, data).unwrap();
            assert!(BookmarkStore::toggle(&root.path, &session).is_err());
            assert_eq!(fs::read_to_string(&root.path).unwrap(), data);
        }
    }
}
