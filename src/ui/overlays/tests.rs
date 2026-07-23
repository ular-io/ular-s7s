//! Overlay tests: the rename modal (open/validate/confirm), the message dialog,
//! the `?` help screen (kept off the screen rotation), and the theme selector
//! (live preview, Enter/Esc, clamping, and dark/light list swap).

use crate::model::Agent;
use crate::ui::effect::AppEffect;
use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

#[test]
fn ctrl_r_opens_rename_modal() {
    let mut app = app_with_session();

    app.on_key_table(key(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, UiMode::Rename);
}

#[test]
fn rename_fails_without_owning_profile() {
    let mut app = app_with_session(); // profile store is empty -> no owning profile
    app.on_key_table(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::Rename);

    app.confirm_rename();

    // Must abort before touching any metadata store and keep the modal open.
    assert_eq!(app.mode, UiMode::Rename);
    assert!(app.pending_effect.is_none());
    assert!(matches!(
        app.status_msg.as_deref(),
        Some(msg) if msg.starts_with("Rename failed: session profile not found")
    ));
}

#[test]
fn confirm_rename_enqueues_effect_when_valid() {
    let mut app = app_with_session();
    // Give the session an owning profile so pre-flight validation passes.
    app.profiles.profiles.push(crate::profile::Profile {
        id: "p1".to_string(),
        agent: Agent::Codex,
        name: "P1".to_string(),
        path: PathBuf::from("/tmp/codex-p1"),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    });
    app.sessions[0].profile_id = "p1".to_string();

    app.on_key_table(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::Rename);
    // The modal pre-fills the current title; confirm it unchanged.
    let expected_title = app.sessions[0].title();
    // Move focus to the buttons and confirm with the OK button focused.
    app.on_key_rename_modal(key(KeyCode::Tab, KeyModifiers::NONE));
    app.on_key_rename_modal(key(KeyCode::Enter, KeyModifiers::NONE));

    // The handler only enqueues; the dialog stays open until the boundary
    // runs the effect (and closes it only on rename success).
    assert_eq!(
        app.pending_effect,
        Some(AppEffect::RenameSession {
            idx: 0,
            title: expected_title,
        })
    );
    assert_eq!(app.mode, UiMode::Rename);
    assert!(app.rename_modal.is_some());
}

#[test]
fn message_dialog_dismisses_to_previous_mode() {
    let mut app = app_with_session();
    app.show_message("t", vec!["body".to_string()], MessageKind::Error);
    assert_eq!(app.mode, UiMode::Message);

    app.on_key_message(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Table);
    assert!(app.message.is_none());
}

#[test]
fn question_mark_opens_help_without_joining_screen_rotation() {
    let mut app = app_with_profiles();

    app.on_key_table(key(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Help);
    assert_eq!(app.screen, Screen::Session);

    app.on_key_help(key(KeyCode::Char('t'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Help);
    assert_eq!(app.screen, Screen::Session);

    app.on_key_help(key(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.screen, Screen::Session);
}

#[test]
fn profile_screen_question_mark_help_returns_to_profile_table() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;

    app.on_key_profile_table(key(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Help);
    assert_eq!(app.screen, Screen::Profile);

    app.on_key_help(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.screen, Screen::Profile);
}

#[test]
fn theme_select_previews_on_move_and_esc_restores_original() {
    let mut app = empty_app();
    let original = app.theme.key.clone();
    app.open_theme_select();
    assert_eq!(app.mode, UiMode::ThemeSelect);
    // Cursor starts on the active theme within its own category list.
    let state = app.theme_select.as_ref().unwrap();
    assert_eq!(state.themes[state.visible()[state.cursor]].key, original);

    // Down applies the next theme immediately (live preview).
    app.on_key_theme_select(key(KeyCode::Down, KeyModifiers::NONE));
    assert_ne!(app.theme.key, original);

    // Esc restores the theme active at open time and closes the dialog.
    app.on_key_theme_select(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.theme.key, original);
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.theme_select.is_none());
}

#[test]
fn theme_select_enter_keeps_previewed_theme_and_closes() {
    let mut app = empty_app();
    app.open_theme_select();
    app.on_key_theme_select(key(KeyCode::Down, KeyModifiers::NONE));
    let previewed = app.theme.key.clone();

    app.on_key_theme_select(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.theme.key, previewed);
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.theme_select.is_none());
    assert!(app
        .status_msg
        .as_deref()
        .is_some_and(|m| m.starts_with("Theme: ")));
}

#[test]
fn theme_select_up_from_top_stays_clamped() {
    let mut app = empty_app();
    app.open_theme_select();
    // Move to the very top, then Up again must not wrap to the last item.
    app.on_key_theme_select(key(KeyCode::Home, KeyModifiers::NONE));
    app.on_key_theme_select(key(KeyCode::Up, KeyModifiers::NONE));
    let state = app.theme_select.as_ref().unwrap();
    assert_eq!(state.cursor, 0);
    assert_eq!(app.theme.key, state.themes[state.visible()[0]].key);
}

#[test]
fn theme_select_down_from_bottom_stays_clamped() {
    let mut app = empty_app();
    app.open_theme_select();
    let vlen = app.theme_select.as_ref().unwrap().visible().len();
    app.on_key_theme_select(key(KeyCode::End, KeyModifiers::NONE));
    app.on_key_theme_select(key(KeyCode::Down, KeyModifiers::NONE));
    let state = app.theme_select.as_ref().unwrap();
    assert_eq!(state.cursor, vlen - 1);
}

#[test]
fn theme_select_left_right_swaps_the_whole_list() {
    let mut app = empty_app();
    app.open_theme_select();
    // Right shows the Light list: every visible theme is light, cursor resets.
    app.on_key_theme_select(key(KeyCode::Right, KeyModifiers::NONE));
    let state = app.theme_select.as_ref().unwrap();
    assert!(!state.dark_view);
    assert_eq!(state.cursor, 0);
    assert!(state.visible().iter().all(|&i| !state.themes[i].dark));
    assert!(!app.theme.dark, "preview switched to a light theme");

    // Left shows the Dark list again.
    app.on_key_theme_select(key(KeyCode::Left, KeyModifiers::NONE));
    let state = app.theme_select.as_ref().unwrap();
    assert!(state.dark_view);
    assert!(state.visible().iter().all(|&i| state.themes[i].dark));
    assert!(app.theme.dark, "preview switched to a dark theme");
}
