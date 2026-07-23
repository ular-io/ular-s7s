//! Cross-feature `ui` tests: behaviors that span more than one screen, such as
//! global refresh reachable from every screen and left/right screen switching.

use super::effect::AppEffect;
use super::test_support::*;
use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn ctrl_u_is_global_across_profile_and_detail_screens() {
    // Profile view: refreshes both session list and usage statistics.
    let mut app = empty_app();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));
    app.apply_effect();
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));

    // Details view: returning to search view if target session vanishes (e.g. empty profile rescan) post-update.
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);
    app.on_key_detail(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));
    app.apply_effect();
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("session update complete · ")
    ));
    assert_eq!(app.screen, Screen::Session);
    assert!(app.detail.is_none());
}

#[test]
fn left_right_switch_between_profile_and_session_screens() {
    let mut app = app_with_session();

    // Session list (with table focus) -> Left key -> Profile list screen.
    app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Profile);

    // Profile view -> Right key -> Session list screen (independent of selected profile).
    app.on_key_profile_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.focus, Focus::Table);
}
