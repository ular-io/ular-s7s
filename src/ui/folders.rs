//! Folder keys, per-folder session counts, and latest-activity folder ordering.

use crate::model::Session;
use std::collections::HashMap;
use std::path::PathBuf;

/// Distinct `key` values over `sessions`, newest latest activity first and ties
/// by key: the order every folder list shows. Sessions mapped to `None` are
/// skipped.
pub(crate) fn by_latest_activity<K: Ord + Clone + std::hash::Hash>(
    sessions: &[Session],
    key: impl Fn(&Session) -> Option<K>,
) -> Vec<K> {
    let mut latest: HashMap<K, i64> = HashMap::new();
    for s in sessions {
        if let Some(k) = key(s) {
            let at = latest.entry(k).or_insert(i64::MIN);
            *at = (*at).max(s.updated_at_ms);
        }
    }
    let mut keys: Vec<(K, i64)> = latest.into_iter().collect();
    keys.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    keys.into_iter().map(|(k, _)| k).collect()
}

/// Sessions per `key`; sessions mapped to `None` are not counted.
pub(crate) fn count_by<'a, K: Eq + std::hash::Hash>(
    sessions: impl IntoIterator<Item = &'a Session>,
    key: impl Fn(&Session) -> Option<K>,
) -> HashMap<K, usize> {
    let mut counts = HashMap::new();
    for s in sessions {
        if let Some(k) = key(s) {
            *counts.entry(k).or_insert(0) += 1;
        }
    }
    counts
}

/// Full-path folder key of a session; `None` when no cwd was recorded.
fn cwd_key(s: &Session) -> Option<PathBuf> {
    (!s.cwd.as_os_str().is_empty()).then(|| s.cwd.clone())
}

/// Folder-name key (the folder filter's unit); `None` when the name is empty.
pub(crate) fn folder_name_key(s: &Session) -> Option<String> {
    (!s.folder.is_empty()).then(|| s.folder.clone())
}

/// Sessions per cwd, the count every full-path folder list shows.
pub(crate) fn cwd_counts(sessions: &[Session]) -> HashMap<PathBuf, usize> {
    count_by(sessions, cwd_key)
}

/// Session cwds by latest activity, empty cwds skipped.
pub(crate) fn cwds_by_latest(sessions: &[Session]) -> Vec<PathBuf> {
    by_latest_activity(sessions, cwd_key)
}

/// Folder names by latest activity.
pub(super) fn folder_names_by_latest(sessions: &[Session]) -> Vec<String> {
    by_latest_activity(sessions, folder_name_key)
}
