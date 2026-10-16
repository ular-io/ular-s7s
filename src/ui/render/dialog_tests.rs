//! Full-frame Detail screen and dialog rendering tests.

use super::input_view;
use super::test_support::*;
use crate::ui::TextInput;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn detail_screen_renders_two_columns_with_selected_row_background() {
    let app = detail_app();
    assert_eq!(app.screen, crate::ui::Screen::Detail);

    let backend = TestBackend::new(100, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);

    // Left prompt column (focused -> thick borders) and right work/answers column.
    assert!(text.contains(" Prompt "));
    assert!(text.contains(" Q1 Work & Answer "));
    assert!(text.contains("● Q1"));
    assert!(text.contains("● Q2"));
    assert!(text.contains("2026-07-23 14:05:06"));
    // Selected question (Q1) uses the focused selection background across the padded row;
    // unselected Q2 keeps the base background. Thick joint overlays are gone.
    let (x1, y1) = find_cell(&terminal, "● Q1");
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(x1, y1)].bg, app.theme.selection_bg);
    assert_eq!(buffer[(x1 + 10, y1)].bg, app.theme.selection_bg);
    let timestamp_x = x1 + 6;
    assert_eq!(buffer[(timestamp_x, y1)].fg, app.theme.muted);
    let (x2, y2) = find_cell(&terminal, "● Q2");
    assert_ne!(buffer[(x2, y2)].bg, app.theme.selection_bg);
    assert!(!text.contains("┣━"));
    // Fallback banner (empty intermediate work logs warning) and final assistant answer section.
    assert!(text.contains("No intermediate work extracted for this turn."));
    assert!(text.contains("● Final Answer"));
}

#[test]
fn detail_screen_work_focus_uses_inactive_selection_background() {
    let mut app = detail_app();
    app.on_key_detail(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)); // Right focus

    let backend = TestBackend::new(100, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);

    // Left panel loses focus: whole panel dims (same rule as the session table) —
    // selection switches to the inactive reversed highlight and unselected
    // questions render in soft-dim (muted).
    assert!(text.contains(" Q1 Work & Answer "));
    let (x1, y1) = find_cell(&terminal, "● Q1");
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(x1, y1)].bg, app.theme.selection_inactive_bg);
    assert!(buffer[(x1, y1)]
        .modifier
        .contains(ratatui::style::Modifier::REVERSED));
    let (x2, y2) = find_cell(&terminal, "● Q2");
    assert_eq!(buffer[(x2, y2)].fg, app.theme.muted);
    assert!(!text.contains("┝━"));
}

#[test]
fn detail_work_panel_hides_tools_by_default_and_dot_shows_them() {
    let mut app = detail_app();
    // Injects tool execution logs into the first turn (fallback turns are empty by default).
    if let Some(d) = app.detail.as_mut() {
        d.turns[0].work_entries = vec![
            crate::handoff::WorkEntry {
                // Use ASCII text as double-width (Korean) characters split cell columns in the TestBackend buffer.
                kind: crate::handoff::WorkKind::AssistantText,
                text: "starting work now".to_string(),
            },
            crate::handoff::WorkEntry {
                kind: crate::handoff::WorkKind::ToolCall,
                text: "cargo build".to_string(),
            },
            crate::handoff::WorkEntry {
                kind: crate::handoff::WorkKind::ToolResult,
                text: "build ok".to_string(),
            },
        ];
    }

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);

    // Default: Tool Calls / Results collapsed & hidden under placeholder text, Assistant text visible.
    assert!(text.contains("starting work now"));
    assert!(!text.contains("● Tool Call"));
    assert!(!text.contains("cargo build"));
    assert!(text.contains("2 tool call/result hidden"));

    // `.` toggles tools only when the Work & Answer column is focused.
    app.on_key_detail(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    // After toggle char `.`: reveals intermediate tool logs.
    app.on_key_detail(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);
    assert!(text.contains("● Tool Call"));
    assert!(text.contains("cargo build"));
    assert!(!text.contains("hidden"));
}

#[test]
fn detail_prompt_arrows_scroll_expanded_turn_instead_of_moving_selection() {
    let mut app = detail_app();
    // Make the first turn far taller than any prompt-pane viewport.
    if let Some(d) = app.detail.as_mut() {
        d.turns[0].user = (1..=60)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
    }
    // Expand it (the Prompt column is focused on entry).
    app.on_key_detail(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().expanded_prompt, Some(0));

    let backend = TestBackend::new(110, 30);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");

    let base = {
        let d = app.detail.as_ref().unwrap();
        assert!(
            d.left_scrollable.get(),
            "an expanded turn taller than the pane must enable prompt scrolling"
        );
        d.left_scroll.get()
    };

    // ↓ scrolls the pane; the turn selection must not advance to the next turn.
    app.on_key_detail(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let d = app.detail.as_ref().unwrap();
    assert_eq!(d.selected, 0, "selection must stay put while scrolling");
    assert!(
        d.left_scroll.get() > base,
        "↓ should scroll the expanded prompt down"
    );

    // Collapsing restores selection navigation: ↓ then moves to the next turn.
    app.on_key_detail(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    assert!(!app.detail.as_ref().unwrap().left_scrollable.get());
    app.on_key_detail(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.detail.as_ref().unwrap().selected, 1);
}

#[test]
fn rename_input_cursor_does_not_insert_a_display_cell() {
    let state = TextInput {
        value: "앞뒤".to_string(),
        cursor: "앞".len(),
        select_all: false,
    };

    assert_eq!(input_view(&state, 10), ("앞뒤".to_string(), 2));
}

#[test]
fn rename_input_scrolls_horizontally_to_keep_cursor_visible() {
    let state = TextInput {
        value: "가나다라마바사".to_string(),
        cursor: "가나다라마바사".len(),
        select_all: false,
    };

    assert_eq!(input_view(&state, 5), ("바사".to_string(), 4));
}

#[test]
fn folder_modal_moving_up_within_window_does_not_scroll() {
    let mut app = folder_modal_app(30);
    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    for _ in 0..29 {
        app.on_key_folder_modal(down);
    }
    render_at(&app, 100, 30);
    // Dialog is 20 rows -> 14 list rows; cursor at 29 pins the bottom-most window.
    let m = app.folder_modal.as_ref().unwrap();
    assert_eq!(m.scroll.get(), 16);

    for _ in 0..3 {
        app.on_key_folder_modal(up);
    }
    render_at(&app, 100, 30);
    let m = app.folder_modal.as_ref().unwrap();
    assert_eq!(m.cursor, 26);
    assert_eq!(
        m.scroll.get(),
        16,
        "Moving up inside the visible window must not scroll"
    );
}

#[test]
fn folder_modal_scrolls_only_when_cursor_leaves_window() {
    let mut app = folder_modal_app(30);
    let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    for _ in 0..29 {
        app.on_key_folder_modal(down);
    }
    render_at(&app, 100, 30);
    // Move above the window top (offset 16): offset must follow the cursor.
    for _ in 0..14 {
        app.on_key_folder_modal(up);
    }
    render_at(&app, 100, 30);
    let m = app.folder_modal.as_ref().unwrap();
    assert_eq!(m.cursor, 15);
    assert_eq!(m.scroll.get(), 15);
    // Moving back down within the window keeps the offset.
    app.on_key_folder_modal(down);
    render_at(&app, 100, 30);
    let m = app.folder_modal.as_ref().unwrap();
    assert_eq!(m.scroll.get(), 15);
}

/// Folder rows are bare basenames, so two things must hold: the scratch row is
/// marked in its own label, and the footer resolves the focused row's full path
/// before it is selected.
#[test]
fn folder_dropdown_marks_the_scratch_row_and_shows_a_path_footer() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    let mut state = new_session_state(None);
    state.focus = crate::ui::NewSessionFocus::Folder;
    state.dropdown_open = true;
    state.folders = vec![std::path::PathBuf::from("/tmp/work/web")];
    state.reorder_folders();
    state.folder_cursor = Some(1); // The folder row, so the footer resolves its path.
    app.new_session = Some(state);

    let mut terminal = Terminal::new(TestBackend::new(160, 34)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);
    assert!(text.contains("[SCRATCH]"));
    // Rows carry the basename alone; the full path belongs to the footer.
    assert!(text.contains("web"));
    assert!(text.contains("/tmp/work/web"));
    // Divider ends are joined to the popup's thick side borders.
    assert!(text.contains("┠"));
    assert!(text.contains("┨"));

    let (footer_x, footer_y) = find_cell(&terminal, "/tmp/work/web");
    let (label_x, label_y) = find_cell(&terminal, "[SCRATCH]");
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(footer_x, footer_y)].fg, app.theme.muted);
    assert_ne!(buffer[(label_x, label_y)].fg, app.theme.muted);
}

/// The blank column cleared left of the popup erases backdrop CJK glyphs
/// below the dialog, but must not cut the dialog's own bottom border: the
/// `━` beside each corner stays so both sides of the popup match.
#[test]
fn folder_dropdown_keeps_the_dialog_bottom_border_on_both_sides() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    let mut state = new_session_state(None);
    state.focus = crate::ui::NewSessionFocus::Folder;
    state.dropdown_open = true;
    state.folders = (0..12)
        .map(|i| std::path::PathBuf::from(format!("/tmp/work/f{i}")))
        .collect();
    state.reorder_folders();
    app.new_session = Some(state);

    let mut terminal = Terminal::new(TestBackend::new(160, 34)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    // The dialog's corners come before the popup's, which sit further down.
    let (left_x, bottom_y) = find_cell(&terminal, "┗");
    let (right_x, right_y) = find_cell(&terminal, "┛");
    assert_eq!(bottom_y, right_y);
    let buffer = terminal.backend().buffer();
    let row = |from: u16, to: u16| -> String {
        (from..=to)
            .map(|x| buffer[(x, bottom_y)].symbol())
            .collect()
    };
    assert_eq!(row(left_x, left_x + 2), "┗━┃");
    assert_eq!(row(right_x - 2, right_x), "┃━┛");
    // Below the dialog the margin column left of the popup stays blank.
    assert_eq!(buffer[(left_x + 1, bottom_y + 1)].symbol(), " ");
    assert_eq!(buffer[(left_x + 2, bottom_y + 1)].symbol(), "┃");
}

/// Every dropdown row, the fixed scratch row included, ends with a dim `(N)`.
#[test]
fn folder_dropdown_rows_end_with_a_dim_session_count() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    let mut state = new_session_state(None);
    state.focus = crate::ui::NewSessionFocus::Folder;
    state.dropdown_open = true;
    state.folders = vec![std::path::PathBuf::from("/tmp/work/web")];
    state
        .folder_counts
        .insert(std::path::PathBuf::from("/tmp/work/web"), 12);
    state.reorder_folders();
    state.folder_cursor = Some(0);
    app.new_session = Some(state);

    let mut terminal = Terminal::new(TestBackend::new(160, 34)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let (web_x, web_y) = find_cell(&terminal, "(12)");
    let (scratch_x, scratch_y) = find_cell(&terminal, "(0)");
    assert_eq!(
        web_x + 1,
        scratch_x,
        "right-aligned counts share an end column"
    );
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(web_x, web_y)].fg, app.theme.muted);
    // The cursor row keeps the count dim too.
    assert_eq!(buffer[(scratch_x, scratch_y)].fg, app.theme.muted);
}

/// A whole-value selection has to be visible before the first key: the value is
/// painted like a selected row, and the footer names both ways out of it.
#[test]
fn folder_input_paints_a_selected_prefill_and_hints_both_flows() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    let mut state = new_session_state(None);
    state.focus = crate::ui::NewSessionFocus::Folder;
    state.input = TextInput::selected("/tmp/work/web".to_string());
    state.folders = vec![std::path::PathBuf::from("/tmp/work/web")];
    state.reorder_folders();
    state.dropdown_open = true;
    state.folder_cursor = state.opening_cursor();
    app.new_session = Some(state);

    let mut terminal = Terminal::new(TestBackend::new(160, 34)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    assert!(buffer_text(&terminal).contains("type to replace · → to edit"));

    let (value_x, value_y) = find_cell(&terminal, "/tmp/work/web");
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(value_x, value_y)].bg, app.theme.selection_bg);
    assert_eq!(buffer[(value_x, value_y)].fg, app.theme.selection_fg);
}

/// The selection survives a focus move, so only the focused Folder field may
/// paint it: a highlight on an inactive control reads as a permanent state.
#[test]
fn folder_input_drops_the_selection_paint_when_focus_moves_away() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    let mut state = new_session_state(None);
    state.focus = crate::ui::NewSessionFocus::Buttons;
    state.input = TextInput::selected("/tmp/work/web".to_string());
    state.folders = vec![std::path::PathBuf::from("/tmp/work/web")];
    state.reorder_folders();
    app.new_session = Some(state);

    let mut terminal = Terminal::new(TestBackend::new(160, 34)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");

    let (value_x, value_y) = find_cell(&terminal, "/tmp/work/web");
    let buffer = terminal.backend().buffer();
    assert_ne!(buffer[(value_x, value_y)].bg, app.theme.selection_bg);
}

/// The scratch row's footer must state the purge before the session starts,
/// not only inside the policy file the agent reads afterwards.
#[test]
fn folder_dropdown_footer_warns_the_scratch_folder_is_cleared() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    let mut state = new_session_state(None);
    state.focus = crate::ui::NewSessionFocus::Folder;
    state.dropdown_open = true;
    state.folders = vec![std::path::PathBuf::from("/tmp/work/web")];
    state.reorder_folders();
    state.folder_cursor = Some(0); // The scratch row, where the dropdown opens.
    app.new_session = Some(state);

    let mut terminal = Terminal::new(TestBackend::new(160, 34)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);
    assert!(text.contains("shared, cleared on each start"));
    assert!(text.contains("s7s/scratch"));
}

#[test]
fn contextual_source_is_a_dimmed_read_only_control() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    app.mode = crate::ui::UiMode::NewSession;
    app.new_session = Some(new_session_state(Some(crate::ui::SessionContextRef {
        agent: crate::model::Agent::Codex,
        profile_id: "builtin-codex".to_string(),
        session_id: "abc".to_string(),
        title: "source-session-title".to_string(),
    })));

    let mut terminal = Terminal::new(TestBackend::new(160, 30)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let (label_x, label_y) = find_cell(&terminal, "Context Source");
    let (value_x, value_y) = find_cell(&terminal, "source-session-title");
    let buffer = terminal.backend().buffer();
    let muted = app.theme.muted;

    assert_eq!(buffer[(label_x, label_y)].fg, muted);
    assert_eq!(buffer[(value_x, value_y)].fg, muted);
    let border_x = (0..label_x)
        .rev()
        .find(|&x| buffer[(x, label_y)].symbol() == "┌")
        .expect("plain source border");
    assert_eq!(buffer[(border_x, label_y)].fg, muted);
    assert!(!(border_x..=label_x + "Context Source".len() as u16)
        .any(|x| buffer[(x, label_y)].symbol() == "▾"));
}
