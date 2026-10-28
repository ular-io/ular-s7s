use super::*;
use crate::model::Agent;
use crate::ui::test_support::TempBookmarkStore;
use crate::usage::UsageWindow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn profile(path: &str) -> Profile {
    Profile {
        id: "builtin-claude".to_string(),
        agent: Agent::Claude,
        name: "Claude".to_string(),
        path: PathBuf::from(path),
        oauth_token: None,
        active: true,
        shortcut: Some(1),
        builtin: true,
    }
}

fn snapshot(pct_left: u8) -> UsageSnapshot {
    UsageSnapshot {
        current: Some(UsageWindow {
            pct_left,
            reset: None,
        }),
        weekly: None,
    }
}

struct Fixture {
    _root: TempBookmarkStore,
    store: PathBuf,
}

fn fixture() -> Fixture {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("usage.json");
    Fixture { _root: root, store }
}

/// Probe stand-in that counts its calls.
fn counting(result: UsageResult) -> (Arc<AtomicUsize>, impl FnOnce(&Profile) -> UsageResult) {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    (calls, move |_: &Profile| {
        seen.fetch_add(1, Ordering::SeqCst);
        result
    })
}

const LONG: Duration = Duration::from_secs(5);

#[test]
fn a_fresh_success_is_reused_without_probing() {
    let f = fixture();
    let p = profile("/tmp/claude");
    let (_, probe) = counting(UsageResult::Ready(snapshot(70)));
    let first = fetch_shared_with(&p, &f.store, now_ms(), LONG, probe);
    assert_eq!(first.origin, Origin::Probed);

    let (calls, probe) = counting(UsageResult::Ready(snapshot(10)));
    let second = fetch_shared_with(&p, &f.store, now_ms(), LONG, probe);
    assert_eq!(second.origin, Origin::Reused);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(second.entry.success.map(|s| s.snapshot), Some(snapshot(70)));
}

#[test]
fn failures_and_stale_successes_are_not_reused() {
    let p = profile("/tmp/claude");
    let now = now_ms();
    let ok = record(None, &p, &UsageResult::Ready(snapshot(70)), now);
    assert!(reusable(&ok, now + REUSE_MS));
    assert!(!reusable(&ok, now + REUSE_MS + 1));

    // A later failure keeps the value but blocks reuse, so a retry probes.
    let failed = record(Some(ok), &p, &UsageResult::NotLoggedIn, now + 10);
    assert_eq!(failed.success.map(|s| s.snapshot), Some(snapshot(70)));
    assert_eq!(
        failed.attempt.map(|a| a.outcome),
        Some(Outcome::NotLoggedIn)
    );
    assert!(!reusable(&failed, now + 20));
}

#[test]
fn an_entry_for_another_folder_does_not_apply() {
    let f = fixture();
    let (_, probe) = counting(UsageResult::Ready(snapshot(70)));
    fetch_shared_with(&profile("/tmp/old"), &f.store, now_ms(), LONG, probe);

    let moved = profile("/tmp/new");
    let (calls, probe) = counting(UsageResult::Ready(snapshot(40)));
    let fetched = fetch_shared_with(&moved, &f.store, now_ms(), LONG, probe);
    assert_eq!(fetched.origin, Origin::Probed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(fetched.entry.path, PathBuf::from("/tmp/new"));
}

#[test]
fn a_waiter_uses_the_holders_attempt_instead_of_probing() {
    let f = fixture();
    let p = profile("/tmp/claude");
    let requested = now_ms();
    let guard = try_lock(&lock_path(&f.store, &p.id)).unwrap().unwrap();

    let (store, waiter_profile) = (f.store.clone(), p.clone());
    let (calls, probe) = counting(UsageResult::Ready(snapshot(10)));
    let waiter =
        thread::spawn(move || fetch_shared_with(&waiter_profile, &store, requested, LONG, probe));
    thread::sleep(Duration::from_millis(300));
    // The holder's query fails; the waiter must report that, not query again.
    let entry = record(None, &p, &UsageResult::Failed("x".into()), now_ms());
    store_entry(&f.store, &p.id, &entry).unwrap();
    drop(guard);

    let fetched = waiter.join().unwrap();
    assert_eq!(fetched.origin, Origin::Waited);
    assert_eq!(
        fetched.entry.attempt.map(|a| a.outcome),
        Some(Outcome::Failed)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_waiter_probes_when_the_holder_left_nothing() {
    let f = fixture();
    let p = profile("/tmp/claude");
    let guard = try_lock(&lock_path(&f.store, &p.id)).unwrap().unwrap();
    let (store, waiter_profile) = (f.store.clone(), p.clone());
    let (calls, probe) = counting(UsageResult::Ready(snapshot(55)));
    let waiter =
        thread::spawn(move || fetch_shared_with(&waiter_profile, &store, now_ms(), LONG, probe));
    thread::sleep(Duration::from_millis(300));
    drop(guard); // e.g. the holder crashed mid-query

    let fetched = waiter.join().unwrap();
    assert_eq!(fetched.origin, Origin::Probed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(stored(&f.store, &p).is_some_and(|e| e.success.is_some()));
}

#[test]
fn waiting_gives_up_as_failed_after_the_limit() {
    let f = fixture();
    let p = profile("/tmp/claude");
    let _guard = try_lock(&lock_path(&f.store, &p.id)).unwrap().unwrap();
    let (calls, probe) = counting(UsageResult::Ready(snapshot(55)));
    let fetched = fetch_shared_with(&p, &f.store, now_ms(), Duration::from_millis(500), probe);
    assert_eq!(fetched.origin, Origin::Waited);
    assert_eq!(
        fetched.entry.attempt.map(|a| a.outcome),
        Some(Outcome::Failed)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn storing_keeps_other_profiles_and_never_overwrites_a_newer_file() {
    let f = fixture();
    let a = profile("/tmp/a");
    let mut b = profile("/tmp/b");
    b.id = "profile-b".to_string();
    store_entry(
        &f.store,
        &a.id,
        &record(None, &a, &UsageResult::NotInstalled, 1),
    )
    .unwrap();
    store_entry(
        &f.store,
        &b.id,
        &record(None, &b, &UsageResult::MissingDir, 2),
    )
    .unwrap();
    let all = load(&f.store).unwrap();
    assert!(entry_for(&all, &a).is_some() && entry_for(&all, &b).is_some());

    std::fs::write(&f.store, br#"{"version": 99, "profiles": {}}"#).unwrap();
    assert!(load(&f.store).is_err());
    assert!(store_entry(
        &f.store,
        &a.id,
        &record(None, &a, &UsageResult::NotInstalled, 3)
    )
    .is_err());
    assert!(std::fs::read_to_string(&f.store).unwrap().contains("99"));

    // A corrupt cache is simply replaced.
    std::fs::write(&f.store, b"{not json").unwrap();
    store_entry(
        &f.store,
        &a.id,
        &record(None, &a, &UsageResult::NotInstalled, 4),
    )
    .unwrap();
    assert!(load(&f.store).is_ok());
}

#[test]
fn a_loading_entry_keeps_its_phase_and_only_takes_a_newer_value() {
    use crate::usage::{UsageEntry, UsagePhase};
    let p = profile("/tmp/claude");
    let shared = record(None, &p, &UsageResult::Ready(snapshot(70)), 2_000);
    let mut loading = UsageEntry {
        phase: UsagePhase::Loading,
        last: Some(snapshot(90)),
        last_at: Some(1_000),
    };
    assert!(loading.apply(&shared));
    assert_eq!(loading.phase, UsagePhase::Loading);
    assert_eq!(
        (loading.last, loading.last_at),
        (Some(snapshot(70)), Some(2_000))
    );
    // An older shared value never replaces what this instance already shows.
    let older = record(None, &p, &UsageResult::Ready(snapshot(10)), 500);
    assert!(!loading.apply(&older));

    // Settled entries follow the latest attempt; a logout keeps the value.
    let mut settled = UsageEntry::idle();
    let logged_out = record(Some(shared), &p, &UsageResult::NotLoggedIn, 3_000);
    assert!(settled.apply(&logged_out));
    assert_eq!(settled.phase, UsagePhase::NotLoggedIn);
    assert_eq!(settled.last, Some(snapshot(70)));
}

#[test]
fn countdowns_run_from_the_read_time_and_end_at_the_reset() {
    use crate::usage::ResetCountdown;
    let r = ResetCountdown {
        days: 0,
        hours: 1,
        minutes: 0,
    };
    let at = 1_000_000;
    assert_eq!(r.remaining(at, at), Some(r));
    assert_eq!(
        r.remaining(at, at + 2 * 60_000),
        Some(ResetCountdown {
            days: 0,
            hours: 0,
            minutes: 58
        })
    );
    assert_eq!(r.remaining(at, at + 60 * 60_000), None);
}
