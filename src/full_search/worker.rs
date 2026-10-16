//! Worker pool: searches sessions on background threads in the given order and
//! streams one result per session over an mpsc channel.

use super::{search_session, Query, SessionMatch};
use crate::model::Session;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

/// Upper bound on worker threads. Each one holds at most one session's raw
/// transcript and parsed context in memory, so this also bounds peak memory.
const MAX_WORKERS: usize = 4;

/// One searched session: its caller-chosen key and the match, if any.
pub type SearchResult<K> = (K, Option<SessionMatch>);

/// Starts the workers. Sessions are claimed in `jobs` order, so results arrive
/// roughly in that order. Workers check `cancel` between sessions and stop when
/// it is set or the receiver is dropped; the channel disconnects once every
/// worker has exited. A panicking session is reported as no match.
pub fn spawn<K: Clone + Send + Sync + 'static>(
    jobs: Vec<(K, Session)>,
    query: Query,
    cancel: Arc<AtomicBool>,
) -> Receiver<SearchResult<K>> {
    let (tx, rx) = mpsc::channel();
    let workers = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, MAX_WORKERS)
        .min(jobs.len());
    let jobs = Arc::new(jobs);
    let query = Arc::new(query);
    let next = Arc::new(AtomicUsize::new(0));
    for n in 0..workers {
        let (jobs, query, next, cancel, tx) = (
            jobs.clone(),
            query.clone(),
            next.clone(),
            cancel.clone(),
            tx.clone(),
        );
        let _ = std::thread::Builder::new()
            .name(format!("s7s-search-{n}"))
            .spawn(move || loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let Some((key, session)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) else {
                    break;
                };
                let found = catch_unwind(AssertUnwindSafe(|| search_session(session, &query)))
                    .ok()
                    .flatten();
                if cancel.load(Ordering::Relaxed) || tx.send((key.clone(), found)).is_err() {
                    break;
                }
            });
    }
    rx
}
