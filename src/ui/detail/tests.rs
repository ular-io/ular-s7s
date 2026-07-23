//! Detail screen key-handling tests: question/work column focus and scrolling,
//! tool-log toggle, resume, in-detail rename, and delete returning to the list.

use crate::ui::effect::AppEffect;
use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn detail_question_selection_and_work_scroll() {
    let mut app = app_with_session();
    app.sessions[0]
        .user_turns
        .push("second question".to_string());
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().turns.len(), 2);

    // Left column (questions): Down key moves selection.
    app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().selected, 1);

    // Right key focuses right details column; Up/Down keys scroll details panel.
    app.on_key_detail(key(KeyCode::Right, KeyModifiers::NONE));
    {
        let d = app.detail.as_ref().unwrap();
        assert_eq!(d.focus, DetailFocus::Work);
        d.right_max_scroll.set(10); // Simulate calculations performed during render frame.
    }
    app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().right_scroll.get(), 1);

    // Right column focus -> Left key -> focuses left panel; left panel focus -> Left key -> returns to search view (table focus).
    app.on_key_detail(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().focus, DetailFocus::Questions);
    app.on_key_detail(key(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Session);
    assert!(app.detail.is_none());
    assert_eq!(app.focus, Focus::Table);
}

/// A user turn long enough (> 8 lines) that `preview_turn_lines` omits its middle,
/// making it expandable via `.`.
fn long_turn() -> String {
    (1..=20)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn detail_dot_expands_selected_prompt_when_questions_focused() {
    let mut app = app_with_session();
    app.sessions[0].user_turns = vec![long_turn()];
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    // Entering detail focuses the Prompt (Questions) column.
    assert_eq!(app.detail.as_ref().unwrap().focus, DetailFocus::Questions);
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, None);
    assert!(!app.detail_show_tools);

    // In the Prompt column, `.` toggles expansion of the selected turn only,
    // leaving the Work-panel tool visibility untouched.
    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, Some(0));
    assert!(!app.detail_show_tools);

    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, None);
}

#[test]
fn detail_dot_ignored_for_short_prompt() {
    // The fixture turn "hello" is a single line — nothing to expand, so `.` is a no-op.
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, None);
}

#[test]
fn detail_expanding_another_prompt_collapses_the_previous() {
    let mut app = app_with_session();
    app.sessions[0].user_turns = vec![long_turn(), long_turn()];
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));

    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, Some(0));

    // Selecting and expanding another turn collapses the first (only one at a time).
    app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().selected, 1);
    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, Some(1));
}

#[test]
fn detail_dot_toggles_tool_visibility_when_work_focused() {
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert!(!app.detail_show_tools); // Hidden by default.

    // Move focus to the Work & Answer column, where `.` reveals tools / full length.
    app.on_key_detail(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().focus, DetailFocus::Work);

    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert!(app.detail_show_tools);
    // The prompt-expansion state stays untouched from the Work column.
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, None);

    app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
    assert!(!app.detail_show_tools);
}

#[test]
fn detail_enter_requests_resume() {
    // Since cwd "/tmp" exists in app_with_session, resume request should be configured.
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));

    app.on_key_detail(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.resume_request, Some(0));
}

#[test]
fn detail_ctrl_r_opens_rename_for_detail_session() {
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));

    app.on_key_detail(key(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, UiMode::Rename);
    assert_eq!(app.rename_target, Some(0));
    // Esc cancels and returns back to details view (table mode), clearing target session index.
    app.on_key_rename_modal(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.screen, Screen::Detail);
    assert_eq!(app.rename_target, None);
}

#[test]
fn detail_delete_returns_to_session_screen_with_next_selected() {
    let (mut app, root) = app_with_two_deletable_sessions();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);

    // Ctrl+D -> opens session deletion confirmation modal -> moves focus to Delete button -> confirm.
    app.on_key_detail(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::DeleteConfirm);
    app.on_key_delete_confirm(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_delete_confirm(key(KeyCode::Enter, KeyModifiers::NONE));

    // Confirm only enqueues the effect; the filesystem removal and screen
    // return happen when the boundary runs it.
    assert_eq!(
        app.pending_effect,
        Some(AppEffect::DeleteSession { idx: 0 })
    );
    app.apply_effect();

    // Returns to search screen, selecting the next session (s2).
    assert_eq!(app.screen, Screen::Session);
    assert!(app.detail.is_none());
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.sessions[0].id, "s2.jsonl");
    assert_eq!(app.selected, 0);
    assert_eq!(app.focus, Focus::Table);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn detail_selection_change_resets_work_scroll() {
    let mut app = app_with_session();
    app.sessions[0]
        .user_turns
        .push("second question".to_string());
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    {
        let d = app.detail.as_ref().unwrap();
        d.right_max_scroll.set(10);
        d.right_scroll.set(5);
    }

    app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));

    let d = app.detail.as_ref().unwrap();
    assert_eq!(d.selected, 1);
    assert_eq!(d.right_scroll.get(), 0);
}

#[test]
fn detail_esc_stays_on_detail_screen() {
    let mut app = app_with_session();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);

    app.on_key_detail(key(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(app.screen, Screen::Detail);
    assert!(app.detail.is_some());
}
