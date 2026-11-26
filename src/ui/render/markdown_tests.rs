//! Detail Work & Answer pane: final answers render as Markdown.

use super::test_support::*;
use crate::ui::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

fn markdown_detail_app() -> App {
    let mut app = detail_app();
    app.detail.as_mut().expect("detail").turns[0].final_answer =
        Some("## Result\n\n**bold** and `code`\n".to_string());
    app
}

fn draw(app: &App, width: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, 32)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, app))
        .expect("draw");
    terminal
}

#[test]
fn final_answer_renders_markdown_without_markers() {
    let mut app = markdown_detail_app();
    app.on_key_detail(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    let terminal = draw(&app, 100);
    let text = buffer_text(&terminal);
    assert!(text.contains("Result"));
    assert!(!text.contains("## Result"));
    assert!(!text.contains("**bold**"));
    let (x, y) = find_cell(&terminal, "Result");
    assert_eq!(
        terminal.backend().buffer()[(x, y)].bg,
        app.theme.md.heading_bg
    );
}

#[test]
fn faded_work_pane_keeps_markdown_styles() {
    // Entering Detail focuses the Prompt pane, so Work & Answer is faded.
    let app = markdown_detail_app();
    let terminal = draw(&app, 100);
    let (x, y) = find_cell(&terminal, "Result");
    let cell = &terminal.backend().buffer()[(x, y)];
    assert_eq!(cell.bg, app.theme.md.heading_bg);
    assert_eq!(cell.fg, app.theme.md.heading_fg);
    let (x, y) = find_cell(&terminal, "● Final Answer");
    assert_eq!(terminal.backend().buffer()[(x, y)].fg, app.theme.muted);
}

#[test]
fn work_rows_are_cached_per_width() {
    let app = markdown_detail_app();
    let rows_at = |width| {
        draw(&app, width);
        let cache = app.detail.as_ref().expect("detail").work_lines.borrow();
        let (key, _) = cache.as_ref().expect("cached after draw");
        key.width
    };
    let wide = rows_at(100);
    assert_ne!(rows_at(80), wide);
}
