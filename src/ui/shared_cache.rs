//! Picks up usage and model-catalog results that other running s7s instances
//! store in `usage.json` / `models.json`. Each loop pass checks the two files'
//! modification times at most once per [`CHECK_INTERVAL`] and re-reads a file
//! only when its time changed. The same tick also redraws once a minute, so
//! usage ages and reset countdowns keep counting while the screen is idle.

use super::App;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

const CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Watched shared files (`usage.json`, `models.json`). `None` in unit tests,
/// which never read the user's files and redraw only on their own events.
pub(crate) struct SharedWatch {
    paths: Option<(PathBuf, PathBuf)>,
    due: Instant,
    usage_seen: Option<SystemTime>,
    models_seen: Option<SystemTime>,
    minute: i64,
}

impl SharedWatch {
    pub(crate) fn new(paths: Option<(PathBuf, PathBuf)>) -> Self {
        SharedWatch {
            paths,
            due: Instant::now(),
            usage_seen: None,
            models_seen: None,
            minute: 0,
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl App {
    /// How long the idle event loop may sleep before the next periodic job
    /// (live-session sweep or shared-file check).
    pub(crate) fn idle_wait(&self) -> Duration {
        let status = self.agent_status_wait();
        if self.shared_watch.paths.is_none() {
            return status;
        }
        status.min(
            self.shared_watch
                .due
                .saturating_duration_since(Instant::now()),
        )
    }

    /// Applies changed shared files and the minute tick. Returns true when the
    /// screen should redraw.
    pub(crate) fn poll_shared_caches(&mut self) -> bool {
        let now = Instant::now();
        if self.shared_watch.paths.is_none() || now < self.shared_watch.due {
            return false;
        }
        let Some((usage_path, models_path)) = self.shared_watch.paths.clone() else {
            return false;
        };
        self.shared_watch.due = now + CHECK_INTERVAL;
        let minute = crate::usage::now_ms() / 60_000;
        let mut changed = minute != self.shared_watch.minute;
        self.shared_watch.minute = minute;

        let seen = modified(&usage_path);
        if seen != self.shared_watch.usage_seen {
            self.shared_watch.usage_seen = seen;
            changed |= self.apply_shared_usage(&usage_path);
        }
        let seen = modified(&models_path);
        if seen != self.shared_watch.models_seen {
            self.shared_watch.models_seen = seen;
            self.models.reload_from(&models_path);
            changed = true;
        }
        changed
    }

    /// Applies every profile's shared entry; an unreadable file changes nothing.
    fn apply_shared_usage(&mut self, path: &Path) -> bool {
        let Ok(entries) = crate::usage::load_shared(path) else {
            return false;
        };
        let mut changed = false;
        for profile in &self.profiles.profiles {
            if let Some(shared) = crate::usage::entry_for(&entries, profile) {
                changed |= self.usage.entry_mut(&profile.id).apply(&shared);
            }
        }
        changed
    }
}
