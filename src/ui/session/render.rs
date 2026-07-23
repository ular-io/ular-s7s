//! Session search screen rendering: the left session table (composite filter
//! title, agent tag, per-row metadata), the right per-turn preview panel, and the
//! `/` keyword search prompt overlay.
//!
//! Extracted from `ui::render` per the refactoring plan (R8b). The full-frame
//! `draw`/`draw_header`/`draw_body` dispatchers and the shared preview helpers
//! (`session_meta_lines`, `preview_turn_lines`, `agent_tag`) stay in `ui::render`;
//! the Session render tests are full-frame (`super::draw`) and stay there too.

use crate::model::format_local_datetime_seconds;
use crate::ui::components::modal::titled_block_nav;
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::{truncate_w, wrap_w};
use crate::ui::render::{agent_tag, preview_turn_display, session_meta_lines, PreviewTurnLine};
use crate::ui::{App, Focus, UiMode};
use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Overlay text box for search query (k9s-style). Only rendered during Keyword filter mode.
pub(crate) fn draw_search_prompt(f: &mut Frame, app: &App, area: Rect) {
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

/// Left session table. Title exhibits `sessions[filter: count]`.
pub(crate) fn draw_table(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let table_focus = app.focus == Focus::Table && app.mode == UiMode::Table;
    let table_dimmed = app.focus == Focus::Preview && app.mode == UiMode::Table;
    let header_style = if table_dimmed {
        th.soft_dim().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
    };
    let text_style = if table_dimmed {
        th.soft_dim()
    } else {
        Style::default()
    };
    let table_style = if table_dimmed {
        th.soft_dim()
    } else {
        Style::default()
    };

    let header = Row::new(vec![
        Cell::from("A"),
        Cell::from("FOLDER"),
        Cell::from("UPDATED"),
        Cell::from("TITLE"),
        Cell::from(format!("{:>4}", "Q")),
        Cell::from(format!("{:>5} ", "SIZE")),
    ])
    .style(header_style);

    // Truncates the TITLE column to fill the remaining width.
    // inner area = area.width - borders (2). Subtracting highlight_symbol (1) +
    // fixed columns (A 4 + FOLDER 18 + UPDATED 11 + Q 4 + SIZE 6 = 43) +
    // spacing (5 gaps for 6 columns) yields TITLE width.
    // UPDATED consists of 10 characters + 1 right padding to add space before TITLE.
    // SIZE consists of 5 right-aligned characters + 1 right margin before the border.
    // Leaves 1 cell padding to prevent right border double-width character overflow.
    let title_w = (area.width as usize).saturating_sub(2 + 1 + 43 + 5 + 1);

    let rows: Vec<Row> = app
        .filtered
        .iter()
        .map(|&i| {
            let s = &app.sessions[i];
            let (tag, color) = agent_tag(s.agent, th);
            let tag_style = if table_dimmed {
                th.soft_dim()
            } else {
                Style::default().fg(color)
            };
            Row::new(vec![
                Cell::from(Span::styled(tag, tag_style)),
                Cell::from(Span::styled(truncate_w(&s.folder, 18), text_style)),
                Cell::from(Span::styled(s.date_str(), text_style)),
                Cell::from(Span::styled(truncate_w(&s.title(), title_w), text_style)),
                Cell::from(Span::styled(
                    format!("{:>4}", s.user_turns.len()),
                    text_style,
                )),
                Cell::from(Span::styled(format!("{:>5} ", s.size_str()), text_style)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(18),
        Constraint::Length(11),
        Constraint::Min(10),
        Constraint::Length(4),
        Constraint::Length(6),
    ];

    // sessions[filter: count] / sessions[count]
    let title = if app.filter.is_active() {
        format!(
            " Session[{}: {}] ",
            app.filter.describe_with(|id| app.profile_name(id)),
            app.filtered.len()
        )
    } else {
        format!(" Session[{}] ", app.filtered.len())
    };

    let row_highlight_bg = if table_focus {
        th.selection_bg
    } else {
        th.selection_inactive_bg
    };
    let row_highlight_style = if table_dimmed {
        th.soft_dim()
            .bg(row_highlight_bg)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .bg(row_highlight_bg)
            .fg(th.selection_fg)
            .add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, widths)
        .style(table_style)
        .header(header)
        .column_spacing(1)
        .block(titled_block_nav(&title, table_focus, true, true, th.accent))
        .row_highlight_style(row_highlight_style)
        // Reserves 1 space to the left of all rows (padding). The highlighted row spans across this space.
        .highlight_symbol(" ");

    // Persists viewport state across frames: updating selection instructs ratatui to scroll minimally,
    // keeping selection visible (cursor moves inside viewport; scrolls only at bounds).
    let mut state: std::cell::RefMut<TableState> = app.table_state.borrow_mut();
    if app.filtered.is_empty() {
        state.select(None);
    } else {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(table, area, &mut state);
    let offset = state.offset();
    drop(state);
    // viewport = inner height - header (1 row) (area.height - 3 including top/bottom borders).
    let viewport = (area.height as usize).saturating_sub(3);
    draw_vscrollbar(
        f,
        area,
        table_focus,
        offset,
        app.filtered.len(),
        viewport,
        th,
    );
}

/// Right preview panel: lists sanitized user questions from the selected session.
pub(crate) fn draw_preview(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let preview_focus = app.focus == Focus::Preview && app.mode == UiMode::Table;
    let block = titled_block_nav(" Prompt ", preview_focus, true, true, th.accent);
    let inner_w = area.width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();
    if let Some(s) = app.current() {
        lines.extend(session_meta_lines(s, inner_w, th, false));
        lines.push(Line::from(Span::styled(
            "─".repeat(inner_w.max(1)),
            Style::default().fg(th.dim),
        )));

        for (idx, turn) in s.user_turns.iter().enumerate() {
            let mut title = vec![Span::styled(
                format!("● Q{}", idx + 1),
                Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            )];
            if let Some(timestamp) = s
                .user_turn_timestamp_ms(idx)
                .and_then(format_local_datetime_seconds)
            {
                title.push(Span::styled(format!("  {timestamp}"), th.soft_dim()));
            }
            lines.push(Line::from(title));
            // When expanded, show every user-turn line in full; otherwise keep the omission.
            for display_line in preview_turn_display(turn, app.preview_expanded) {
                let (raw_line, style) = match display_line {
                    PreviewTurnLine::Content(line) => (line.to_string(), Style::default()),
                    PreviewTurnLine::Omission(count) => (
                        format!("────── ⋯ {count} lines omitted ⋯ ──────"),
                        th.soft_dim().add_modifier(Modifier::DIM),
                    ),
                };
                // Pre-wrap content using width constraints (handles long contiguous words and tab tabstops).
                // Disables Paragraph's native wrapping; uses this pre-computed result as the sole line sequence.
                for wrapped in wrap_w(&raw_line, inner_w) {
                    lines.push(Line::from(Span::styled(wrapped, style)));
                }
            }
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w.max(1)),
                Style::default().fg(th.dim),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No sessions to show.",
            Style::default().fg(th.dim),
        )));
    }

    // Computes and sets the maximum scroll boundary based on total lines and viewport height.
    // Limits scrolling bounds; if content fits within viewport (lines <= viewport), defaults to 0,
    // which disables scrolling via Up/Down keys.
    let total = lines.len();
    let viewport = (area.height as usize).saturating_sub(2);
    app.preview_max_scroll
        .set(total.saturating_sub(viewport).min(u16::MAX as usize) as u16);

    // Native Paragraph wrapping is disabled as `wrap_w` already handled formatting.
    // (Combining with native wrapping disrupts width metrics, leaking long lines past borders
    // and causing discrepancies in scroll boundaries.) Each `Line` maps exactly to one terminal row.
    let para = Paragraph::new(lines)
        .block(block)
        .scroll((app.preview_scroll, 0));
    f.render_widget(para, area);
    draw_vscrollbar(
        f,
        area,
        preview_focus,
        app.preview_scroll as usize,
        total,
        viewport,
        th,
    );
}
