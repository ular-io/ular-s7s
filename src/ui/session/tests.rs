//! Session screen key-handling tests: table focus/navigation, keyword search,
//! profile-number filtering, resume gating, global refresh/quit from the table,
//! and owning-profile resolution.

use crate::model::Agent;
use crate::ui::effect::AppEffect;
use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

#[test]
fn session_profile_root_resolves_owning_profile_path() {
    let mut app = app_with_session();
    app.profiles.profiles.push(crate::profile::Profile {
        id: "profile-team".to_string(),
        agent: Agent::Codex,
        name: "Team".to_string(),
        path: PathBuf::from("/tmp/codex-team"),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    });
    app.sessions[0].profile_id = "profile-team".to_string();

    assert_eq!(
        app.session_profile_root(&app.sessions[0]),
        Some(PathBuf::from("/tmp/codex-team"))
    );

    // Unknown profile id must resolve to None (no default-root fallback:
    // that would write title metadata into the wrong account store).
    app.sessions[0].profile_id = "ghost".to_string();
    assert_eq!(app.session_profile_root(&app.sessions[0]), None);
}

#[test]
fn ctrl_u_updates_sessions_without_entering_rename_mode() {
    let mut app = empty_app();

    app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));

    // The handler only enqueues the effect; the rescan/status run at the boundary.
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));

    app.apply_effect();
    assert!(app.pending_effect.is_none());
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));
}

#[test]
fn quit_requires_two_presses() {
    let mut app = app_with_session();

    app.on_key_table(key(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(!app.should_quit);
    assert_eq!(
        app.status_msg.as_deref(),
        Some("Press q or ctrl+c again to quit")
    );

    app.on_key_table(key(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(app.should_quit);
}

#[test]
fn quit_keys_are_ignored_during_grace_after_handover() {
    let mut app = app_with_session();
    app.begin_quit_grace();
    let initial_grace = app.quit_grace_until.unwrap();

    // During the grace period, exits do not trigger even on rapid key spam (Ctrl+C x 2), extending grace period.
    app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(!app.quit_armed);
    assert!(!app.should_quit);
    assert!(app.quit_grace_until.unwrap() >= initial_grace);

    // Restores normal "press twice to exit" behavior after grace period expires (user halts spamming).
    app.quit_grace_until = Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
    app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.quit_armed);
    app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.should_quit);
}

#[test]
fn tab_no_longer_toggles_focus_in_session_table() {
    let mut app = app_with_session();

    app.on_key_table(key(KeyCode::Tab, KeyModifiers::NONE));

    assert_eq!(app.focus, Focus::Table);
}

#[test]
fn arrow_keys_move_column_focus_in_session_screen() {
    let mut app = app_with_session();
    assert_eq!(app.focus, Focus::Table);

    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Preview);

    app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Table);
    assert_eq!(app.screen, Screen::Session);
}

#[test]
fn right_key_on_preview_opens_detail_screen() {
    let mut app = app_with_session();

    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE)); // focus preview
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE)); // enter details

    assert_eq!(app.screen, Screen::Detail);
    let d = app.detail.as_ref().expect("detail state");
    assert_eq!(d.focus, DetailFocus::Questions);
    assert_eq!(d.turns.len(), 1);
    assert_eq!(d.turns[0].user, "hello");
}

#[test]
fn search_tab_closes_prompt_and_focuses_preview() {
    let mut app = app_with_session();
    app.mode = UiMode::Keyword;
    app.filter.keyword = "hello".to_string();

    app.on_key_keyword(key(KeyCode::Tab, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.focus, Focus::Preview);
    assert_eq!(app.filter.keyword, "hello");
}

#[test]
fn search_backtab_closes_prompt_and_focuses_table() {
    let mut app = app_with_session();
    app.mode = UiMode::Keyword;
    app.focus = Focus::Preview;
    app.filter.keyword = "hello".to_string();

    app.on_key_keyword(key(KeyCode::BackTab, KeyModifiers::SHIFT));

    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.focus, Focus::Table);
    assert_eq!(app.filter.keyword, "hello");
}

#[test]
fn search_arrow_keys_move_cursor_and_edit_mid_string() {
    let mut app = app_with_session();
    app.mode = UiMode::Keyword;
    app.filter.keyword = "helo".to_string();
    app.keyword_cursor = app.filter.keyword.len();

    // Move cursor left twice, insert missing 'l' -> resolves to "hello".
    app.on_key_keyword(key(KeyCode::Left, KeyModifiers::NONE));
    app.on_key_keyword(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.keyword_cursor, 2);
    app.on_key_keyword(key(KeyCode::Char('l'), KeyModifiers::NONE));
    assert_eq!(app.filter.keyword, "hello");
    assert_eq!(app.keyword_cursor, 3);

    // Verify Home/End navigation and Backspace/Delete actions at current cursor positions.
    app.on_key_keyword(key(KeyCode::Home, KeyModifiers::NONE));
    assert_eq!(app.keyword_cursor, 0);
    app.on_key_keyword(key(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(app.filter.keyword, "ello");
    app.on_key_keyword(key(KeyCode::End, KeyModifiers::NONE));
    app.on_key_keyword(key(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(app.filter.keyword, "ell");
    assert_eq!(app.keyword_cursor, app.filter.keyword.len());
}

#[test]
fn search_cursor_moves_over_multibyte_chars() {
    let mut app = app_with_session();
    app.mode = UiMode::Keyword;
    app.filter.keyword = "한글".to_string();
    app.keyword_cursor = app.filter.keyword.len();

    // Must traverse by character boundary step (3 bytes per Hangul char).
    app.on_key_keyword(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.keyword_cursor, "한".len());
    app.on_key_keyword(key(KeyCode::Char('x'), KeyModifiers::NONE));
    assert_eq!(app.filter.keyword, "한x글");
}

#[test]
fn number_key_filters_by_active_profile() {
    let mut app = app_with_profiles();

    // <2> key: filters only by the second active profile (profile-x).
    app.on_key_table(key(KeyCode::Char('2'), KeyModifiers::NONE));
    assert_eq!(app.filtered.len(), 1);
    assert_eq!(app.sessions[app.filtered[0]].profile_id, "profile-x");

    // <0> key: clears active filters.
    app.on_key_table(key(KeyCode::Char('0'), KeyModifiers::NONE));
    assert_eq!(app.filtered.len(), 2);
}

#[test]
fn resume_blocked_when_folder_missing_shows_message() {
    let mut app = app_with_cwd("/no/such/dir/s7s-xyz");

    app.on_key_table(key(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.resume_request.is_none());
    assert_eq!(app.mode, UiMode::Message);
    assert!(app.message.is_some());
}

#[test]
fn resume_allowed_when_folder_exists() {
    // Since "/tmp" exists on disk, resume request should be configured.
    let mut app = app_with_cwd("/tmp");

    app.on_key_table(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.resume_request, Some(0));
    assert_eq!(app.mode, UiMode::Table);
}
