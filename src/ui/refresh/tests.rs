use super::*;
use crate::ui::{effect::AppEffect, test_support::*, Screen};
use crossterm::event::{KeyCode, KeyModifiers};
use std::sync::mpsc::{channel, Sender};

fn two_sessions() -> App {
    let mut app = app_with_session();
    let mut second = app.sessions[0].clone();
    second.id = "session-2".into();
    app.sessions.push(second);
    app.recompute();
    app
}

fn pending(app: &mut App) -> Sender<Result<RefreshResult, String>> {
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    app.start_scheduled_refresh_scan();
    let (tx, rx) = channel();
    app.background.set_refresh_receiver(rx);
    tx
}

#[test]
fn input_and_repeat_refresh_work_before_scan_completion() {
    let mut app = two_sessions();
    let tx = pending(&mut app);
    assert!(app.background_in_flight());
    assert!(!app.poll_background());
    app.finish_refresh_cycle();
    assert_eq!(app.refresh_all, RefreshAllPhase::Scanning);

    app.on_key_table(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.current().unwrap().id, "session-2");
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(!app.refresh_scan_scheduled());
    assert_eq!(app.refresh_all, RefreshAllPhase::Scanning);

    app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::NONE));
    app.on_key_keyword(key(KeyCode::Char('h'), KeyModifiers::NONE));
    assert_eq!(app.filter.keyword, "h");
    // Cursor changes made during the scan win over its starting selection.
    app.selected = 1;
    let mut sessions = app.sessions.clone();
    sessions.reverse();
    tx.send(Ok(RefreshResult::fixture(sessions))).ok().unwrap();
    assert!(app.poll_background());
    assert_eq!(app.filter.keyword, "h");
    assert_eq!(app.mode, UiMode::Keyword);
    assert_eq!(app.current().unwrap().id, "session-2");
    assert_eq!(app.selected, 0);
    assert!(!app.background_in_flight());
}

#[test]
fn completed_scan_waits_until_index_based_dialogs_close() {
    for mode in [
        UiMode::Rename,
        UiMode::DeleteConfirm,
        UiMode::FolderModal,
        UiMode::NewSession,
    ] {
        let mut app = two_sessions();
        let tx = pending(&mut app);
        let mut sessions = app.sessions.clone();
        sessions.reverse();
        tx.send(Ok(RefreshResult::fixture(sessions))).ok().unwrap();
        app.mode = mode;
        assert!(!app.poll_background());
        assert_eq!(app.sessions[0].id, "session-1");
        assert!(app.background_in_flight());
        app.mode = UiMode::Table;
        assert!(app.poll_background());
        assert_eq!(app.sessions[0].id, "session-2");
        assert_eq!(app.current().unwrap().id, "session-1");
    }
}

#[test]
fn handover_request_keeps_its_original_session_index() {
    let mut app = two_sessions();
    let tx = pending(&mut app);
    let mut sessions = app.sessions.clone();
    sessions.reverse();
    tx.send(Ok(RefreshResult::fixture(sessions))).ok().unwrap();
    app.resume_request = Some(0);
    assert!(!app.poll_background());
    assert_eq!(app.sessions[app.resume_request.unwrap()].id, "session-1");
}

#[test]
fn detail_refresh_keeps_current_turn_selection_and_profile_identity() {
    let mut app = two_sessions();
    // Identical agent/id under different profiles must remain distinct.
    app.sessions[1].id = app.sessions[0].id.clone();
    app.sessions[0].profile_id = "first".into();
    app.sessions[1].profile_id = "second".into();
    app.open_session_detail();
    let tx = pending(&mut app);
    let mut result = RefreshResult::fixture(app.sessions.iter().rev().cloned().collect());
    result.detail = Some((
        SessionKey::of(&app.sessions[0]),
        vec![
            HandoffTurn {
                user: "updated first".into(),
                ..Default::default()
            },
            HandoffTurn {
                user: "updated second".into(),
                ..Default::default()
            },
        ],
    ));
    app.detail.as_mut().unwrap().selected = 1;
    tx.send(Ok(result)).ok().unwrap();
    assert!(app.poll_background());
    let detail = app.detail.as_ref().unwrap();
    assert_eq!(detail.session_idx, 1);
    assert_eq!(detail.selected, 1);
    assert_eq!(detail.turns[1].user, "updated second");
    assert_eq!(app.sessions[detail.session_idx].profile_id, "first");
}

#[test]
fn navigation_to_another_detail_does_not_apply_old_turns() {
    let mut app = two_sessions();
    app.open_session_detail();
    let tx = pending(&mut app);
    let mut result = RefreshResult::fixture(app.sessions.iter().rev().cloned().collect());
    result.detail = Some((SessionKey::of(&app.sessions[0]), Vec::new()));
    app.close_session_detail();
    app.selected = 1;
    app.open_session_detail();
    tx.send(Ok(result)).ok().unwrap();
    assert!(app.poll_background());
    assert_eq!(app.screen, Screen::Detail);
    let detail = app.detail.as_ref().unwrap();
    assert_eq!(detail.session_idx, 0);
    assert_eq!(detail.turns[0].user, "hello");
}

#[test]
fn deletion_discards_a_ready_snapshot_instead_of_restoring_the_session() {
    let (mut app, root) = app_with_two_deletable_sessions();
    let tx = pending(&mut app);
    tx.send(Ok(RefreshResult::fixture(app.sessions.clone())))
        .ok()
        .unwrap();
    app.pending_effect = Some(AppEffect::DeleteSession { idx: 0 });
    app.apply_effect();
    assert!(!app.poll_background());
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.sessions[0].id, "s2.jsonl");
    assert_eq!(app.refresh_all, RefreshAllPhase::Idle);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_worker_keeps_the_old_index_and_allows_retry() {
    let mut app = two_sessions();
    let tx = pending(&mut app);
    drop(tx);
    assert!(app.poll_background());
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(
        app.status_msg.as_deref(),
        Some("Session refresh failed: worker disconnected")
    );
    app.finish_refresh_cycle();
    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    app.apply_effect();
    assert!(app.refresh_scan_scheduled());
}

#[test]
fn cache_is_published_only_when_the_snapshot_is_accepted() {
    let temp = TempBookmarkStore::new();
    let root = temp.path.parent().unwrap();
    let cache = root.join("index.bin");
    std::fs::write(&cache, b"newer index").unwrap();
    let mut result = RefreshResult::collect(&[], None, &cache, &root.join("workspaces.json"));
    let staging = result.cache.as_ref().unwrap().staging.clone();
    assert_eq!(std::fs::read(&cache).unwrap(), b"newer index");
    assert!(staging.exists());
    result.publish_cache();
    assert!(!staging.exists());
    assert!(crate::cache::Cache::load(&cache).entries.is_empty());

    let mut app = two_sessions();
    let tx = pending(&mut app);
    let result = RefreshResult::collect(&[], None, &cache, &root.join("workspaces.json"));
    let staging = result.cache.as_ref().unwrap().staging.clone();
    app.cancel_refresh_scan();
    std::fs::write(&cache, b"newer index").unwrap();
    // A worker finishing after invalidation must clean up its staged file.
    assert!(tx.send(Ok(result)).is_err());
    assert!(!staging.exists());
    assert_eq!(std::fs::read(&cache).unwrap(), b"newer index");
}

#[test]
fn worker_collects_index_and_detail_without_touching_the_published_cache() {
    let temp = TempBookmarkStore::new();
    let root = temp.path.parent().unwrap();
    crate::demo::ensure_demo_sandbox(root).unwrap();
    let profile = crate::profile::Profile {
        id: "demo-claude".into(),
        agent: crate::model::Agent::Claude,
        name: "demo".into(),
        path: root.join("sessions/claude"),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    };
    let cache = root.join("cache/index.bin");
    let workspaces = root.join("config/session_workspaces.json");
    let initial = RefreshResult::collect(std::slice::from_ref(&profile), None, &cache, &workspaces);
    let detail = &initial.scan.sessions[0];
    let expected_turns = crate::handoff::load_turns(detail);
    let detail = SessionKey::of(detail);
    let worker = std::thread::spawn(move || {
        RefreshResult::collect(&[profile], Some(detail), &cache, &workspaces)
    });
    let result = worker.join().unwrap();
    assert!(!root.join("cache/index.bin").exists());
    assert!(!result.scan.sessions.is_empty());
    assert_eq!(
        result.detail.as_ref().unwrap().1.len(),
        expected_turns.len()
    );
    assert_eq!(
        result.detail.as_ref().unwrap().1[0].user,
        expected_turns[0].user
    );
}
