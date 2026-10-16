//! Full-search rendering: the prompt drawn above the Session body and the match
//! snippet box at the top of the Prompt pane.

use crate::full_search::{SessionMatch, SnippetLine};
use crate::theme::Theme;
use crate::ui::render::input_view;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Wrapped rows kept per snippet line; the last kept row ends in `…`.
const MAX_ROWS_PER_LINE: usize = 4;
const PREFIX: &str = "/ ";

/// The full-search prompt. While it owns input it is framed like the keyword
/// prompt (thick accent, hardware cursor); while the session table does, it
/// fades like an unfocused pane but keeps the run status readable.
pub(crate) fn draw_prompt(f: &mut Frame, app: &App, area: Rect) {
    let Some(state) = app.full_search.as_ref() else {
        return;
    };
    let th = &app.theme;
    let editing = state.editing && app.mode == UiMode::Keyword;
    let (border_type, frame_style) = if editing {
        (
            BorderType::Thick,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )
    } else {
        (BorderType::Plain, th.soft_dim())
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(frame_style)
        .title(Span::styled(" Full Search ", frame_style));
    if let Some(run) = &state.run {
        let status = Style::default().add_modifier(Modifier::BOLD);
        block = block.title_top(
            Line::from(Span::styled(format!(" {} ", run.status(!editing)), status)).right_aligned(),
        );
    }
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(th.base_style()), area);
    f.render_widget(block, area);

    let text_w = (inner.width as usize).saturating_sub(PREFIX.width());
    let (visible, cursor_x) = input_view(&state.input, text_w);
    let text_style = if editing && state.input.select_all {
        Style::default().fg(th.selection_fg).bg(th.selection_bg)
    } else {
        Style::default()
    };
    let line = Line::from(vec![
        Span::styled(PREFIX, Style::default().fg(th.dim)),
        Span::styled(visible, text_style),
    ]);
    f.render_widget(Paragraph::new(line), inner);
    if editing && inner.width > 0 && inner.height > 0 {
        let x = inner.x + PREFIX.width() as u16 + cursor_x;
        if x < inner.x + inner.width {
            f.set_cursor_position((x, inner.y));
        }
    }
}

/// Lines of the match box shown above the Session block:
///
/// ```text
/// ┌ Match · Q3 · Tool Result ──── 4 hits ┐
/// │ line before                          │
/// │ line with the match highlighted      │
/// │ line after                           │
/// └──────────────────────────────────────┘
/// ```
///
/// The frame and text fade with the pane; the highlights keep their accent
/// chip colors, since they are what the list is being scanned for.
pub(crate) fn match_box_lines(
    m: &SessionMatch,
    inner_w: usize,
    th: &Theme,
    dimmed: bool,
) -> Vec<Line<'static>> {
    if inner_w < 8 {
        return Vec::new();
    }
    let (frame, heading, text) = if dimmed {
        (
            th.soft_dim(),
            th.soft_dim().add_modifier(Modifier::BOLD),
            th.soft_dim(),
        )
    } else {
        (
            Style::default().fg(th.accent),
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            Style::default(),
        )
    };
    let highlight = Style::default()
        .fg(th.on_accent)
        .bg(th.accent)
        .add_modifier(Modifier::BOLD);
    let content_w = inner_w - 4;

    let label = format!(" Match · Q{} · {} ", m.turn + 1, m.field.label());
    let note = if m.hits > 1 {
        format!(" {} hits ", m.hits)
    } else {
        String::new()
    };
    let room = inner_w - 2;
    let label = crate::ui::components::text::truncate_w(&label, room);
    let note = if label.width() + note.width() <= room {
        note
    } else {
        String::new()
    };
    let fill = room - label.width() - note.width();
    let mut lines = vec![Line::from(vec![
        Span::styled("┌", frame),
        Span::styled(label, heading),
        Span::styled("─".repeat(fill), frame),
        Span::styled(note, th.soft_dim()),
        Span::styled("┐", frame),
    ])];
    for snippet in &m.lines {
        for row in wrap_highlighted(snippet, content_w) {
            let used: usize = row.iter().map(|(s, _)| s.width()).sum();
            let mut spans = vec![Span::styled("│ ", frame)];
            spans.extend(
                row.into_iter()
                    .map(|(s, hl)| Span::styled(s, if hl { highlight } else { text })),
            );
            spans.push(Span::raw(" ".repeat(content_w.saturating_sub(used))));
            spans.push(Span::styled(" │", frame));
            lines.push(Line::from(spans));
        }
    }
    lines.push(Line::from(Span::styled(
        format!("└{}┘", "─".repeat(room)),
        frame,
    )));
    lines
}

/// Wraps one snippet line to `width` cells by grapheme, keeping each run of
/// highlighted or plain text as one segment.
fn wrap_highlighted(line: &SnippetLine, width: usize) -> Vec<Vec<(String, bool)>> {
    let mut rows: Vec<Vec<(String, bool)>> = vec![Vec::new()];
    let mut used = 0;
    for (at, g) in line.text.grapheme_indices(true) {
        let w = g.width();
        if used + w > width && used > 0 {
            if rows.len() == MAX_ROWS_PER_LINE {
                mark_cut(rows.last_mut().expect("row"), width);
                return rows;
            }
            rows.push(Vec::new());
            used = 0;
        }
        let hl = line.highlights.iter().any(|&(s, e)| at >= s && at < e);
        let row = rows.last_mut().expect("row");
        match row.last_mut() {
            Some((text, last)) if *last == hl => text.push_str(g),
            _ => row.push((g.to_string(), hl)),
        }
        used += w;
    }
    rows
}

/// Ends a full row with `…`, dropping trailing graphemes to make room.
fn mark_cut(row: &mut Vec<(String, bool)>, width: usize) {
    while row.iter().map(|(s, _)| s.width()).sum::<usize>() + 1 > width {
        let Some((text, _)) = row.last_mut() else {
            return;
        };
        let cut = text
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(at, _)| at);
        text.truncate(cut);
        if text.is_empty() {
            row.pop();
        }
    }
    row.push(("…".to_string(), false));
}
