//! Quick Command App-integration tests: opening the palette (`:`) and terminal
//! (`!`) windows from the session table, empty-input mode switching, terminal
//! history recall, the Edit Config command, and the folder-existence guard.

use super::QuickMode;
use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

#[test]
fn bang_opens_terminal_mode_and_enter_requests_command() {
    let mut app = app_with_session(); // session cwd /tmp exists on disk
    app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::QuickCommand);
    assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Terminal);

    for c in "echo hi".chars() {
        app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Table);
    let req = app.terminal_request.take().expect("terminal request");
    assert_eq!(req.cwd, PathBuf::from("/tmp"));
    assert_eq!(req.command, "echo hi");
    // `!` commands keep the post-exit keypress wait (output must stay readable).
    assert_eq!(req.kind, TerminalKind::Command);
    assert_eq!(
        app.terminal_history.first().map(String::as_str),
        Some("echo hi")
    );
}

#[test]
fn edit_config_requests_editor_without_pause() {
    let mut app = empty_app();
    app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::QuickCommand);
    for c in "config.toml".chars() {
        app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Table);
    let req = app.terminal_request.take().expect("terminal request");
    assert_eq!(req.cwd, crate::config::config_base_dir());
    assert!(req.command.ends_with("config.toml'"), "{}", req.command);
    // Interactive editors leave no output to read: return without a keypress;
    // failures offer the vim fallback in the handover.
    assert_eq!(req.kind, TerminalKind::EditConfig);
}

#[test]
fn bang_requires_selected_session_with_existing_folder() {
    let mut app = empty_app();
    app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.quick.is_none());
    assert!(app.status_msg.is_some());
}

#[test]
fn quick_mode_switch_only_on_empty_input() {
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Palette);

    // Empty-input `!` switches palette -> terminal.
    app.on_key_quick(key(KeyCode::Char('!'), KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Terminal);

    // With text present, `:` is an ordinary character (no switch).
    app.on_key_quick(key(KeyCode::Char('x'), KeyModifiers::NONE));
    app.on_key_quick(key(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Terminal);
    assert_eq!(app.quick.as_ref().unwrap().input.value, "x:");

    // Clearing the input re-enables switching terminal -> palette.
    app.on_key_quick(key(KeyCode::Backspace, KeyModifiers::NONE));
    app.on_key_quick(key(KeyCode::Backspace, KeyModifiers::NONE));
    app.on_key_quick(key(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Palette);
}

#[test]
fn terminal_history_recall_fills_input_and_restores_typed_text() {
    let mut app = app_with_session();
    app.terminal_history = vec!["git status".to_string(), "ls".to_string()];
    app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));

    // Down recalls the most recent command into the editable input.
    app.on_key_quick(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().term_selected, Some(0));
    assert_eq!(app.quick.as_ref().unwrap().input.value, "git status");
    app.on_key_quick(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().input.value, "ls");

    // Moving back above the list restores the (empty) typed text.
    app.on_key_quick(key(KeyCode::Up, KeyModifiers::NONE));
    app.on_key_quick(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.quick.as_ref().unwrap().term_selected, None);
    assert_eq!(app.quick.as_ref().unwrap().input.value, "");

    // Typing filters the history list and detaches the selection.
    app.on_key_quick(key(KeyCode::Char('g'), KeyModifiers::NONE));
    let state = app.quick.as_ref().unwrap();
    assert_eq!(state.term_items, vec!["git status".to_string()]);
    assert_eq!(state.term_selected, None);
}

#[test]
fn palette_refresh_all_uses_the_shared_refresh_effect() {
    // "Refresh Usage & Sessions" enqueues the same AppEffect::RefreshAll as
    // Ctrl+U on the Session/Profile/Detail screens (one shared two-phase path).
    let mut app = empty_app();
    app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
    for c in "refresh".chars() {
        app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(
        app.pending_effect,
        Some(crate::ui::effect::AppEffect::RefreshAll)
    );
    app.apply_effect();
    assert!(app.refresh_scan_scheduled());
    assert_eq!(
        app.status_msg.as_deref(),
        Some("updating sessions and usage…")
    );
}

#[test]
fn colon_opens_quick_command() {
    let mut app = app_with_profiles();

    app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::QuickCommand);
    assert!(app.quick.is_some());

    // Esc closes the palette and restores prior table mode.
    app.on_key_quick(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.quick.is_none());

    // Screen transitions are driven by ←/→ keys; Esc does not change the active screen.
    app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Profile);
    app.on_key_profile_table(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Profile);
    app.on_key_profile_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Session);
}
