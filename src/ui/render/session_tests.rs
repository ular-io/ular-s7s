//! Full-frame Session screen tests: focus fading, status bar, Prompt preview.

use super::test_support::*;
use super::{preview_turn_lines, PreviewTurnLine};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn backdrop_fades_cells_behind_open_dialog() {
    let mut app = session_app();
    app.theme = crate::theme::default_theme();
    let backend = TestBackend::new(200, 50);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::ui::render::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let (before_fg, before_bg) = (buffer[(0, 0)].fg, buffer[(0, 0)].bg);
    // 'a' opens the agent filter dialog; the frame behind it must fade.
    app.on_key_table(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    terminal.draw(|f| crate::ui::render::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let (after_fg, after_bg) = (buffer[(0, 0)].fg, buffer[(0, 0)].bg);
    let target = app.theme.bg_rgb();
    assert_eq!(
        after_fg,
        crate::ui::components::modal::fade_toward(crate::theme::color_rgb(before_fg), target)
    );
    assert_eq!(
        after_bg,
        crate::ui::components::modal::fade_toward(crate::theme::color_rgb(before_bg), target)
    );
    assert_ne!(before_fg, after_fg);
}

#[test]
fn unfocused_pane_frames_and_titles_fade() {
    let mut app = crate::ui::test_support::app_with_session();
    app.theme = crate::theme::default_theme();
    let (accent, muted) = (app.theme.accent, app.theme.muted);
    let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
    // The body's top border sits right under the five-row header.
    terminal.draw(|f| crate::ui::render::draw(f, &app)).unwrap();
    assert_eq!(title_fg(&terminal, 5, "Session"), accent);
    assert_eq!(title_fg(&terminal, 5, "Prompt"), muted);

    app.on_key_table(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    terminal.draw(|f| crate::ui::render::draw(f, &app)).unwrap();
    assert_eq!(title_fg(&terminal, 5, "Session"), muted);
    assert_eq!(title_fg(&terminal, 5, "Prompt"), accent);

    // The search prompt (three rows, no backdrop) leaves no pane focused
    // and none faded.
    app.on_key_table(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    terminal.draw(|f| crate::ui::render::draw(f, &app)).unwrap();
    for title in ["Session", "Prompt"] {
        let fg = title_fg(&terminal, 8, title);
        assert!(fg != muted && fg != accent, "{title}: {fg:?}");
    }
}

#[test]
fn status_bar_keeps_one_cell_horizontal_margins() {
    let mut app = session_app();
    let mut terminal = Terminal::new(TestBackend::new(12, 1)).expect("test terminal");

    terminal
        .draw(|f| {
            let area = f.area();
            super::draw_status_bar(f, &app, area);
        })
        .expect("draw status bar");

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(1, 0)].symbol(), "q");
    assert_eq!(buffer[(11, 0)].symbol(), " ");

    app.status_msg = Some("x".repeat(20));
    terminal
        .draw(|f| {
            let area = f.area();
            super::draw_status_bar(f, &app, area);
        })
        .expect("draw long status message");

    let buffer = terminal.backend().buffer();
    assert_ne!(buffer[(0, 0)].bg, app.theme.accent);
    assert_eq!(buffer[(1, 0)].bg, app.theme.accent);
    assert_eq!(buffer[(10, 0)].bg, app.theme.accent);
    assert_ne!(buffer[(11, 0)].bg, app.theme.accent);
}

/// The `● Context Source` block is the affordance for `ctrl+o`: it only exists
/// for a context-derived session, and the hint only when the jump can succeed.
#[test]
fn context_source_heading_hints_the_jump_key_only_when_reachable() {
    use crate::model::{Agent, ContextSource, Session};
    use std::path::PathBuf;
    use unicode_width::UnicodeWidthStr;
    let th = crate::theme::default_theme();
    let src = ContextSource {
        id: "source-1".to_string(),
        agent: Agent::Codex,
        profile: "p1".to_string(),
    };
    let source_session = Session {
        agent: Agent::Codex,
        profile_id: "p1".to_string(),
        id: "source-1".to_string(),
        source_path: None,
        cwd: PathBuf::from("/tmp/demo"),
        folder: "demo".to_string(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["question".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: Some("source title".to_string()),
        title_fixed: false,
        context_source: None,
    };
    let heading_of = |lines: &[ratatui::text::Line<'static>]| -> String {
        lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    };

    let resolved = super::context_source_lines(
        &src,
        Some(&source_session),
        Default::default(),
        60,
        &th,
        false,
    );
    let heading = heading_of(&resolved);
    assert!(heading.starts_with("● Context Source"));
    assert!(heading.ends_with("<ctrl+o>"), "heading was {heading:?}");
    assert_eq!(heading.width(), 60, "hint is flush with the pane edge");

    // Unresolved source: the key would report "unavailable", so no hint.
    let missing = super::context_source_lines(&src, None, Default::default(), 60, &th, false);
    assert!(!heading_of(&missing).contains("ctrl+o"));

    // Narrow pane: the hint is dropped rather than crowding the heading.
    let narrow = super::context_source_lines(
        &src,
        Some(&source_session),
        Default::default(),
        20,
        &th,
        false,
    );
    assert_eq!(heading_of(&narrow), "● Context Source");
}

#[test]
fn workspace_pane_shrinks_the_body_and_hides_a_narrow_prompt() {
    use ratatui::layout::Rect;
    let area = |w| Rect::new(0, 0, w, 20);
    let (pane, table, prompt) = super::body_layout(area(140), true);
    assert_eq!(pane.map(|p| p.width), Some(24));
    assert_eq!(table.x, 24);
    assert!(prompt.is_some_and(|p| p.width >= super::PROMPT_MIN_W));

    // 76 cells left: a 42% Prompt would be under 40, so the table takes all.
    let (pane, table, prompt) = super::body_layout(area(100), true);
    assert_eq!(pane.map(|p| p.width), Some(24));
    assert_eq!((table.x, table.width), (24, 76));
    assert!(prompt.is_none());

    // Without the pane the Prompt always shows.
    let (pane, table, prompt) = super::body_layout(area(80), false);
    assert!(pane.is_none());
    assert_eq!(table.x, 0);
    assert!(prompt.is_some());
}

#[test]
fn session_table_hides_optional_columns_on_narrow_terminals() {
    let wide = session_table_header(200);
    for label in ["UPDATED", "TITLE", " Q ", "SIZE"] {
        assert!(wide.contains(label), "{label} missing: {wide}");
    }
    // 105 columns leave the table 61 cells: SIZE and Q hidden, UPDATED kept.
    let narrow = session_table_header(105);
    assert!(narrow.contains("UPDATED") && narrow.contains("TITLE"));
    assert!(
        !narrow.contains("SIZE") && !narrow.contains(" Q "),
        "{narrow}"
    );
}

#[test]
fn session_prompt_renders_turn_timestamp_in_soft_dim() {
    let app = session_app();
    let backend = TestBackend::new(100, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let text = buffer_text(&terminal);

    assert!(text.contains("● Q1  2026-07-23 14:05:06"));
    let (x, y) = find_cell(&terminal, "2026-07-23 14:05:06");
    assert_eq!(terminal.backend().buffer()[(x, y)].fg, app.theme.muted);
}

#[test]
fn preview_keeps_queries_with_eight_lines() {
    let turn = "1\n2\n3\n4\n5\n6\n7\n8";

    assert_eq!(
        preview_turn_lines(turn),
        vec![
            PreviewTurnLine::Content("1"),
            PreviewTurnLine::Content("2"),
            PreviewTurnLine::Content("3"),
            PreviewTurnLine::Content("4"),
            PreviewTurnLine::Content("5"),
            PreviewTurnLine::Content("6"),
            PreviewTurnLine::Content("7"),
            PreviewTurnLine::Content("8"),
        ]
    );
}

#[test]
fn preview_collapses_queries_over_eight_lines() {
    let turn = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13";

    assert_eq!(
        preview_turn_lines(turn),
        vec![
            PreviewTurnLine::Content("1"),
            PreviewTurnLine::Content("2"),
            PreviewTurnLine::Content("3"),
            PreviewTurnLine::Content("4"),
            PreviewTurnLine::Omission(5),
            PreviewTurnLine::Content("10"),
            PreviewTurnLine::Content("11"),
            PreviewTurnLine::Content("12"),
            PreviewTurnLine::Content("13"),
        ]
    );
}

#[test]
fn preview_does_not_scroll_when_content_fits() {
    let app = preview_app(vec!["short".to_string()]);
    render_at(&app, 100, 40); // Large screen: content fits in viewport -> max scroll is 0
    assert_eq!(app.preview_max_scroll.get(), 0);

    let mut app = app;
    for _ in 0..5 {
        app.on_key_table(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert_eq!(
        app.preview_scroll, 0,
        "Should not scroll when content fits in viewport"
    );
}

#[test]
fn preview_scrolls_when_content_overflows() {
    let turns: Vec<String> = (1..=30).map(|i| format!("question {i}")).collect();
    let mut app = preview_app(turns);
    render_at(&app, 60, 10); // Small screen: content overflows -> scroll is enabled
    assert!(app.preview_max_scroll.get() > 0);

    app.on_key_table(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(
        app.preview_scroll > 0,
        "Should scroll when content overflows"
    );
}
