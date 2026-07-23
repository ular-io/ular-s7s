//! mtime-based incremental cache.
//!
//! Stores mapping of (mtime, sessions) by file path in `~/Library/Caches/s7s/index.bin` on macOS (dirs::cache_dir()-based) (bincode).
//! Minimizes I/O overhead by comparing file mtime during scans to reparse only modified/new files.

use crate::model::Session;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Cache format/parser version. Increment this value to automatically invalidate
/// existing cache when parser logic or the Session structure changes.
pub const CACHE_VERSION: u32 = 13;

/// Cache entry: mtime of the source file (or logical key) and derived sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub mtime_ms: i64,
    pub sessions: Vec<Session>,
}

/// Entire cache. The key is the file absolute path (or antigravity logical key).
#[derive(Debug, Serialize, Deserialize)]
pub struct Cache {
    pub version: u32,
    pub entries: HashMap<String, CacheEntry>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache {
            version: CACHE_VERSION,
            entries: HashMap::new(),
        }
    }
}

impl Cache {
    /// Loads the cache from disk. Returns a default cache if missing, corrupted, or version mismatched.
    pub fn load(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match bincode::deserialize::<Cache>(&bytes) {
                Ok(c) if c.version == CACHE_VERSION => c,
                _ => Cache::default(), // Version mismatch or corruption -> full re-parsing
            },
            Err(_) => Cache::default(),
        }
    }

    /// Saves the cache to disk (parent directory is automatically created).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = bincode::serialize(self)?;
        std::fs::write(path, bytes)?;
        // The cache now holds redacted assistant answers; restrict it to the owner on
        // Unix so other local users cannot read one account's session content.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Returns sessions if the cached mtime matches the provided mtime.
    pub fn get_fresh(&self, key: &str, mtime_ms: i64) -> Option<&Vec<Session>> {
        self.entries.get(key).and_then(|e| {
            if e.mtime_ms == mtime_ms {
                Some(&e.sessions)
            } else {
                None
            }
        })
    }

    /// Stores sessions with their mtime for the given key.
    pub fn put(&mut self, key: String, mtime_ms: i64, sessions: Vec<Session>) {
        self.entries.insert(key, CacheEntry { mtime_ms, sessions });
    }
}
