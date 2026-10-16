//! The `/` keyword search prompt drawn above the Session screen body.

use crate::ui::components::text::wrap_w;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Overlay text box for search query (k9s-style). Draws the keyword prompt
/// while it owns input, otherwise the open full-search prompt.
pub(crate) fn draw_search_prompt(f: &mut Frame, app: &App, area: Rect) {
    let keyword_owns_input = app.mode == UiMode::Keyword && !app.full_search_editing();
    if app.full_search_active() && !keyword_owns_input {
        return crate::ui::full_search::render::draw_prompt(f, app, area);
    }
    let th = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(th.accent).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            " Search ",
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(th.base_style()), area);
    f.render_widget(block, area);

    let content_w = inner.width.saturating_sub(1) as usize;
    let wrap_width = content_w.saturating_sub(2);
    // "/ " prefix occupies 2 cells; subsequent lines are padded with two spaces ("  ").
    const PREFIX_W: u16 = 2;
    let mut lines = Vec::new();
    let wrapped = wrap_w(&app.filter.keyword, wrap_width);
    if wrapped.is_empty() {
        lines.push(Line::from(Span::styled("/ ", Style::default().fg(th.dim))));
    } else {
        for (i, chunk) in wrapped.iter().enumerate() {
            let prefix = if i == 0 { "/ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(th.dim)),
                Span::raw(chunk.clone()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);

    // Cursor position calculation: wrap text prior to cursor under matching rules to isolate line and column metrics.
    let cursor = app.keyword_cursor.min(app.filter.keyword.len());
    let before = &app.filter.keyword[..cursor];
    let before_lines = wrap_w(before, wrap_width);
    let (line_idx, col) = match before_lines.last() {
        Some(last) => (
            before_lines.len().saturating_sub(1),
            UnicodeWidthStr::width(last.as_str()),
        ),
        None => (0, 0),
    };
    let cursor_x = inner.x.saturating_add(PREFIX_W).saturating_add(col as u16);
    let cursor_y = inner.y.saturating_add(line_idx as u16);
    // Hard cursor rendered only if coordinates stay within the text box boundaries.
    if cursor_x < inner.x.saturating_add(inner.width)
        && cursor_y < inner.y.saturating_add(inner.height)
    {
        f.set_cursor_position((cursor_x, cursor_y));
    }
}
