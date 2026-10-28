//! Usage results shared by every running s7s instance through
//! `~/.config/s7s/usage.json`, and the per-profile probe lock that keeps two
//! instances from querying the same profile at the same time.
//!
//! Per profile, a query (startup, `ctrl+u`, profile save) runs:
//! 1. Take `.usage-<profile id>.lock` without blocking.
//! 2. Held elsewhere: wait for it (up to [`WAIT_LIMIT`]). If the holder stored
//!    an attempt after this request, use it — a failure included, so a failing
//!    CLI is not retried once per waiting instance. Otherwise (the holder only
//!    reused a value, or exited mid-query) continue with the lock held.
//! 3. A stored success no older than [`REUSE_MS`] whose latest attempt also
//!    succeeded is reused as is. Failures and logouts are never reused.
//! 4. Otherwise probe under the lock, store the result, then release the lock.
//!
//! The file is a cache: an unreadable one is replaced on the next store, but a
//! file written by a newer s7s (`version` above ours) is never overwritten.

use super::{fetch, UsageResult, UsageSnapshot};
use crate::profile::Profile;
use crate::store_lock::{replace_file, try_lock, with_store_lock};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// How long a stored success stands in for a new query.
pub(crate) const REUSE_MS: i64 = 60_000;
/// Longest wait for another instance's query before giving up as failed.
const WAIT_LIMIT: Duration = Duration::from_secs(180);
const WAIT_STEP: Duration = Duration::from_millis(200);
const FILE_VERSION: u32 = 1;

/// Kind of the latest query attempt for a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ready,
    Failed,
    NotLoggedIn,
    NotInstalled,
    MissingDir,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Success {
    pub at_ms: i64,
    pub snapshot: UsageSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub at_ms: i64,
    pub outcome: Outcome,
}

/// One profile's shared state. `path` is the config root the values were read
/// for: after a profile's folder changes, its old entry no longer applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedEntry {
    pub path: PathBuf,
    pub success: Option<Success>,
    pub attempt: Option<Attempt>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UsageFile {
    version: u32,
    profiles: HashMap<String, SharedEntry>,
}

/// How a worker obtained its entry; only `Reused` skipped the CLI entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Probed,
    Reused,
    Waited,
}

/// Background worker payload: the profile's entry after the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub entry: SharedEntry,
    pub origin: Origin,
}

/// Current wall-clock time in Unix milliseconds. Wall clock (not `Instant`) so
/// ages stay right across sleep and across processes.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Reads every shared entry. A missing file is empty; an unreadable or
/// newer-version file is an error, and callers keep what they already show.
pub fn load(store: &Path) -> Result<HashMap<String, SharedEntry>> {
    match read_file(store)? {
        FileState::Usable(file) => Ok(file.profiles),
        FileState::Corrupt => bail!("unreadable {}", store.display()),
        FileState::Newer => bail!("{} was written by a newer s7s", store.display()),
    }
}

/// The entry for `profile` if it was recorded for the profile's current folder.
pub fn entry_for(entries: &HashMap<String, SharedEntry>, profile: &Profile) -> Option<SharedEntry> {
    entries
        .get(&profile.id)
        .filter(|e| e.path == profile.path)
        .cloned()
}

enum FileState {
    Usable(UsageFile),
    Corrupt,
    Newer,
}

fn read_file(store: &Path) -> Result<FileState> {
    let data = match std::fs::read(store) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileState::Usable(UsageFile::default()))
        }
        Err(e) => return Err(e.into()),
    };
    #[derive(Deserialize)]
    struct VersionOnly {
        version: u32,
    }
    match serde_json::from_slice::<VersionOnly>(&data) {
        Ok(v) if v.version > FILE_VERSION => return Ok(FileState::Newer),
        Ok(_) => {}
        Err(_) => return Ok(FileState::Corrupt),
    }
    Ok(serde_json::from_slice::<UsageFile>(&data)
        .map(FileState::Usable)
        .unwrap_or(FileState::Corrupt))
}

/// Writes one profile's entry, keeping every other profile's.
fn store_entry(store: &Path, profile_id: &str, entry: &SharedEntry) -> Result<()> {
    with_store_lock(store, || {
        let mut file = match read_file(store)? {
            FileState::Usable(file) => file,
            FileState::Corrupt => UsageFile::default(),
            FileState::Newer => bail!("{} was written by a newer s7s", store.display()),
        };
        file.version = FILE_VERSION;
        file.profiles.insert(profile_id.to_string(), entry.clone());
        replace_file(store, &serde_json::to_vec_pretty(&file)?)
    })
}

fn lock_path(store: &Path, profile_id: &str) -> PathBuf {
    let safe: String = profile_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    store.with_file_name(format!(".usage-{safe}.lock"))
}

fn stored(store: &Path, profile: &Profile) -> Option<SharedEntry> {
    load(store).ok().and_then(|e| entry_for(&e, profile))
}

/// Whether `entry` can stand in for a new query at `now`.
pub(crate) fn reusable(entry: &SharedEntry, now: i64) -> bool {
    let fresh_success = entry
        .success
        .is_some_and(|s| now.saturating_sub(s.at_ms) <= REUSE_MS);
    let last_ok = entry.attempt.is_some_and(|a| a.outcome == Outcome::Ready);
    fresh_success && last_ok
}

/// Folds a query result into the profile's previous entry.
pub(crate) fn record(
    previous: Option<SharedEntry>,
    profile: &Profile,
    result: &UsageResult,
    at_ms: i64,
) -> SharedEntry {
    let mut entry = previous.unwrap_or_else(|| SharedEntry {
        path: profile.path.clone(),
        success: None,
        attempt: None,
    });
    let outcome = match result {
        UsageResult::Ready(snapshot) => {
            entry.success = Some(Success {
                at_ms,
                snapshot: *snapshot,
            });
            Outcome::Ready
        }
        UsageResult::Failed(_) => Outcome::Failed,
        UsageResult::NotLoggedIn => Outcome::NotLoggedIn,
        UsageResult::NotInstalled => Outcome::NotInstalled,
        UsageResult::MissingDir => Outcome::MissingDir,
        UsageResult::Unavailable => Outcome::Unavailable,
    };
    entry.attempt = Some(Attempt { at_ms, outcome });
    entry
}

/// Runs one profile's query through the shared cache (see the module docs).
/// `requested_ms` is when the caller asked; a holder's attempt from before then
/// does not answer this request.
pub fn fetch_shared(profile: &Profile, store: &Path, requested_ms: i64) -> Fetched {
    fetch_shared_with(profile, store, requested_ms, WAIT_LIMIT, fetch)
}

fn fetch_shared_with(
    profile: &Profile,
    store: &Path,
    requested_ms: i64,
    wait_limit: Duration,
    probe: impl FnOnce(&Profile) -> UsageResult,
) -> Fetched {
    let lock = lock_path(store, &profile.id);
    let guard = match try_lock(&lock) {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => match wait_for_holder(&lock, store, profile, requested_ms, wait_limit) {
            Waited::Answered(entry) => {
                return Fetched {
                    entry,
                    origin: Origin::Waited,
                }
            }
            Waited::Acquired(guard) => Some(guard),
            Waited::TimedOut => {
                let entry = record(
                    stored(store, profile),
                    profile,
                    &UsageResult::Failed("timed out waiting for another s7s".into()),
                    now_ms(),
                );
                return Fetched {
                    entry,
                    origin: Origin::Waited,
                };
            }
        },
        // No lock file (unwritable config folder): query without sharing.
        Err(_) => None,
    };
    let previous = stored(store, profile);
    if let Some(entry) = previous.clone().filter(|e| reusable(e, now_ms())) {
        return Fetched {
            entry,
            origin: Origin::Reused,
        };
    }
    let result = probe(profile);
    let entry = record(previous, profile, &result, now_ms());
    let _ = store_entry(store, &profile.id, &entry);
    drop(guard);
    Fetched {
        entry,
        origin: Origin::Probed,
    }
}

enum Waited {
    /// The holder recorded an attempt after this request: use it.
    Answered(SharedEntry),
    /// The lock came free with no attempt after the request (the holder only
    /// reused a value, or exited mid-query). The caller now holds the lock.
    Acquired(crate::store_lock::LockGuard),
    TimedOut,
}

fn wait_for_holder(
    lock: &Path,
    store: &Path,
    profile: &Profile,
    requested_ms: i64,
    limit: Duration,
) -> Waited {
    let started = Instant::now();
    while started.elapsed() < limit {
        thread::sleep(WAIT_STEP);
        let Ok(Some(guard)) = try_lock(lock) else {
            continue;
        };
        let answered =
            stored(store, profile).filter(|e| e.attempt.is_some_and(|a| a.at_ms >= requested_ms));
        return match answered {
            Some(entry) => Waited::Answered(entry),
            None => Waited::Acquired(guard),
        };
    }
    Waited::TimedOut
}

#[cfg(test)]
mod tests;
