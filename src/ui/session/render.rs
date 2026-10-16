//! Session search screen rendering: the left session table (composite filter
//! title, agent tag, per-row metadata) and the right per-turn preview panel. The
//! `/` keyword search prompt lives in `prompt`, the table column layout in
//! `table_layout`.
//!
//! Extracted from `ui::render` per the refactoring plan (R8b). The full-frame
//! `draw`/`draw_body` dispatchers stay in `ui::render` and the shared preview
//! helpers (`session_meta_lines`, `preview_turn_lines`, `agent_tag`) in
//! `ui::render::meta`; the full-frame Session render tests are in
//! `ui::render::session_tests`.
//! The pure `table_layout` width tests live with `table_layout`.

use super::table_layout::{table_layout, TableColumn};
use crate::model::format_local_datetime_seconds;
use crate::ui::components::modal::titled_block_nav;
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::{truncate_w, wrap_w};
use crate::ui::render::{
    agent_tag, context_source_lines, preview_turn_display, session_meta_lines, PreviewTurnLine,
};
use crate::ui::{App, Focus, UiMode};
use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Left session table. Title exhibits `sessions[filter: count]`. It fades
/// while a neighbouring panel (the Prompt or the workspace pane) owns focus.
pub(crate) fn draw_table(f: &mut Frame, app: &App, area: Rect) {
    let table_focus = app.focus == Focus::Table && app.mode == UiMode::Table;
    let table_dimmed = app.focus != Focus::Table && app.mode == UiMode::Table;
    let th = &app.theme;
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

    // Measured over every loaded session, not just the filtered ones, so the
    // column does not shift while a search is typed.
    let folder_label_w = app
        .sessions
        .iter()
        .map(|s| UnicodeWidthStr::width(crate::scratch::folder_label(&s.cwd, &s.folder)))
        .max()
        .unwrap_or(0);
    let layout = table_layout(area.width, folder_label_w);
    let header_cell = |col: TableColumn| match col {
        TableColumn::Agent => Cell::from("A"),
        TableColumn::Folder => Cell::from("FOLDER"),
        TableColumn::Updated => Cell::from("UPDATED"),
        TableColumn::Title => Cell::from("TITLE"),
        TableColumn::Q => Cell::from(layout.pad_right(col, format!("{:>4}", "Q"))),
        TableColumn::Size => Cell::from(format!("{:>5} ", "SIZE")),
    };
    let header = Row::new(layout.columns.iter().map(|&col| header_cell(col))).style(header_style);

    let rows: Vec<Row> = app
        .filtered
        .iter()
        .map(|&i| {
            let s = &app.sessions[i];
            let marks = app.title_marks(s);
            let bookmarked = marks.bookmarked;
            let (tag, color) = agent_tag(s.agent, th);
            let tag_style = if table_dimmed {
                th.soft_dim()
            } else {
                Style::default().fg(color)
            };
            Row::new(layout.columns.iter().map(|&col| match col {
                TableColumn::Agent => Cell::from(Span::styled(tag, tag_style)),
                TableColumn::Folder => Cell::from(Span::styled(
                    truncate_w(
                        crate::scratch::folder_label(&s.cwd, &s.folder),
                        layout.folder_w,
                    ),
                    text_style,
                )),
                TableColumn::Updated => Cell::from(Span::styled(s.date_str(), text_style)),
                TableColumn::Title => Cell::from(crate::ui::bookmarks::styled_title(
                    truncate_w(&app.session_display_title(s), layout.title_w),
                    marks,
                    text_style,
                )),
                TableColumn::Q => Cell::from(Span::styled(
                    layout.pad_right(col, format!("{:>4}", s.user_turns.len())),
                    text_style,
                )),
                TableColumn::Size => {
                    Cell::from(Span::styled(format!("{:>5} ", s.size_str()), text_style))
                }
            }))
            .style(if bookmarked {
                Style::default().bg(th.bookmark_bg)
            } else {
                Style::default()
            })
        })
        .collect();

    let widths: Vec<Constraint> = layout
        .columns
        .iter()
        .map(|&col| match col {
            TableColumn::Title => Constraint::Min(10),
            _ => Constraint::Length(layout.width(col) as u16),
        })
        .collect();

    // Session[workspace, filter: count] / Session[count]
    let scope: Vec<String> = app
        .active_workspace_name()
        .map(str::to_string)
        .into_iter()
        .chain(app.full_search_query().map(|q| format!("full: {q}")))
        .chain(
            app.filter
                .is_active()
                .then(|| app.filter.describe_with(|id| app.profile_name(id))),
        )
        .collect();
    let title = if scope.is_empty() {
        format!(" Session[{}] ", app.filtered.len())
    } else {
        format!(" Session[{}: {}] ", scope.join(", "), app.filtered.len())
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
        .block(titled_block_nav(
            &title,
            table_focus,
            table_dimmed,
            true,
            true,
            th,
        ))
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
    // Every pane without focus fades, as the table does when this one has it;
    // an overlay or the search prompt leaves both undimmed.
    let dimmed = app.focus != Focus::Preview && app.mode == UiMode::Table;
    let block = titled_block_nav(" Prompt ", preview_focus, dimmed, true, true, th);
    let inner_w = area.width.saturating_sub(2) as usize;
    let (heading_style, content_style) = if dimmed {
        (th.soft_dim().add_modifier(Modifier::BOLD), th.soft_dim())
    } else {
        (
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            Style::default(),
        )
    };

    let mut lines: Vec<Line> = Vec::new();
    if let Some(s) = app.current() {
        if let Some(m) = app.full_search_match(s) {
            lines.extend(crate::ui::full_search::render::match_box_lines(
                m, inner_w, th, dimmed,
            ));
        }
        lines.extend(session_meta_lines(
            s,
            app.title_marks(s),
            true,
            inner_w,
            th,
            dimmed,
        ));
        lines.push(Line::from(Span::styled(
            "─".repeat(inner_w.max(1)),
            Style::default().fg(th.dim),
        )));

        if let Some(src) = &s.context_source {
            let resolved = app.context_source_index(s).map(|i| &app.sessions[i]);
            lines.extend(context_source_lines(
                src,
                resolved,
                resolved.map(|s| app.title_marks(s)).unwrap_or_default(),
                inner_w,
                th,
                dimmed,
            ));
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w.max(1)),
                Style::default().fg(th.dim),
            )));
        }

        for (idx, turn) in s.user_turns.iter().enumerate() {
            let mut title = vec![Span::styled(format!("● Q{}", idx + 1), heading_style)];
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
                    PreviewTurnLine::Content(line) => (line.to_string(), content_style),
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
            app.full_search_empty_note()
                .unwrap_or("No sessions to show."),
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
