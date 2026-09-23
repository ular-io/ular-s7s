//! Session search screen rendering: the left session table (composite filter
//! title, agent tag, per-row metadata), the right per-turn preview panel, and the
//! `/` keyword search prompt overlay.
//!
//! Extracted from `ui::render` per the refactoring plan (R8b). The full-frame
//! `draw`/`draw_header`/`draw_body` dispatchers and the shared preview helpers
//! (`session_meta_lines`, `preview_turn_lines`, `agent_tag`) stay in `ui::render`;
//! the Session render tests are full-frame (`super::draw`) and stay there too.
//! Only the pure `table_layout` width tests live here.

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

/// Session table columns in display order. UPDATED, Q, and SIZE are optional and
/// dropped on narrow panes (see `table_layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableColumn {
    Agent,
    Folder,
    Updated,
    Title,
    Q,
    Size,
}

/// TITLE width below which the table gives up space: first FOLDER's extra width,
/// then the optional columns in the order SIZE, Q, UPDATED.
const TITLE_MIN_W: usize = 20;
/// FOLDER width range. The column grows to the longest folder label up to
/// `FOLDER_MAX_PERCENT` of the table width (never below the minimum), but only
/// while every column is shown; it is back at the minimum before any column is
/// hidden, so narrowing the window never widens it.
const FOLDER_MIN_W: usize = 12;
const FOLDER_MAX_PERCENT: usize = 20;

fn folder_max_w(area_width: u16) -> usize {
    (area_width as usize * FOLDER_MAX_PERCENT / 100).max(FOLDER_MIN_W)
}

struct TableLayout {
    columns: Vec<TableColumn>,
    folder_w: usize,
    title_w: usize,
}

impl TableLayout {
    fn is_last(&self, col: TableColumn) -> bool {
        self.columns.last() == Some(&col)
    }

    /// Fixed cell width of a non-TITLE column.
    /// UPDATED is 10 characters + 1 right padding before TITLE. SIZE is 5
    /// right-aligned characters + 1 right margin before the border; Q takes that
    /// margin over when SIZE is hidden.
    fn width(&self, col: TableColumn) -> usize {
        match col {
            TableColumn::Agent => 4,
            TableColumn::Folder => self.folder_w,
            TableColumn::Updated => 11,
            TableColumn::Title => self.title_w,
            TableColumn::Q if self.is_last(col) => 5,
            TableColumn::Q => 4,
            TableColumn::Size => 6,
        }
    }

    fn pad_right(&self, col: TableColumn, text: String) -> String {
        if col == TableColumn::Q && self.is_last(col) {
            text + " "
        } else {
            text
        }
    }

    /// TITLE width left in a table `area_width` wide. inner area = area.width -
    /// borders (2); subtract highlight_symbol (1), the fixed columns, one spacing
    /// cell per gap, and 1 cell padding that keeps a double-width character from
    /// overflowing the right border.
    fn fit_title(&mut self, area_width: u16) {
        let fixed: usize = self
            .columns
            .iter()
            .filter(|&&c| c != TableColumn::Title)
            .map(|&c| self.width(c))
            .sum();
        let gaps = self.columns.len() - 1;
        self.title_w = (area_width as usize).saturating_sub(2 + 1 + fixed + gaps + 1);
    }
}

/// Picks the visible columns and the FOLDER/TITLE widths for a table
/// `area_width` wide whose longest folder label is `folder_label_w` cells.
fn table_layout(area_width: u16, folder_label_w: usize) -> TableLayout {
    let mut layout = TableLayout {
        columns: vec![
            TableColumn::Agent,
            TableColumn::Folder,
            TableColumn::Updated,
            TableColumn::Title,
            TableColumn::Q,
            TableColumn::Size,
        ],
        folder_w: folder_label_w.clamp(FOLDER_MIN_W, folder_max_w(area_width)),
        title_w: 0,
    };
    layout.fit_title(area_width);
    if layout.title_w >= TITLE_MIN_W {
        return layout;
    }
    // Give back FOLDER's extra width before hiding any column.
    let deficit = TITLE_MIN_W - layout.title_w;
    layout.folder_w = layout.folder_w.saturating_sub(deficit).max(FOLDER_MIN_W);
    layout.fit_title(area_width);
    for col in [TableColumn::Size, TableColumn::Q, TableColumn::Updated] {
        if layout.title_w >= TITLE_MIN_W {
            break;
        }
        layout.columns.retain(|&c| c != col);
        layout.fit_title(area_width);
    }
    layout
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
                TableColumn::Title => Cell::from(Span::styled(
                    truncate_w(&s.title(), layout.title_w),
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

        if let Some(src) = &s.context_source {
            let resolved = app.context_source_index(s).map(|i| &app.sessions[i]);
            lines.extend(context_source_lines(src, resolved, inner_w, th, false));
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w.max(1)),
                Style::default().fg(th.dim),
            )));
        }

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

#[cfg(test)]
mod tests {
    use super::{table_layout, TableColumn::*, FOLDER_MIN_W, TITLE_MIN_W};

    #[test]
    fn table_layout_drops_size_then_q_then_updated() {
        // (table width, visible columns, TITLE width) with FOLDER at its minimum.
        let cases = [
            (66, vec![Agent, Folder, Updated, Title, Q, Size], 20),
            (65, vec![Agent, Folder, Updated, Title, Q], 25),
            (60, vec![Agent, Folder, Updated, Title, Q], 20),
            (59, vec![Agent, Folder, Updated, Title], 25),
            (54, vec![Agent, Folder, Updated, Title], 20),
            (53, vec![Agent, Folder, Title], 31),
        ];
        for (width, columns, title_w) in cases {
            let layout = table_layout(width, 0);
            assert_eq!(layout.columns, columns, "width {width}");
            assert_eq!(layout.folder_w, FOLDER_MIN_W, "width {width}");
            assert_eq!(layout.title_w, title_w, "width {width}");
        }
    }

    #[test]
    fn folder_grows_to_the_longest_label_within_bounds() {
        assert_eq!(table_layout(200, 10).folder_w, FOLDER_MIN_W);
        assert_eq!(table_layout(200, 20).folder_w, 20);
        // The maximum is 20% of the table width: 40 at 200, 20 at 100.
        assert_eq!(table_layout(200, 50).folder_w, 40);
        assert_eq!(table_layout(100, 50).folder_w, 20);
    }

    #[test]
    fn folder_shrinks_before_any_column_is_hidden() {
        // At 67 cells the 20% cap (13) still leaves TITLE 20.
        let layout = table_layout(67, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Updated, Title, Q, Size]);
        assert_eq!((layout.folder_w, layout.title_w), (13, 20));
        // At 66 the cap (13) leaves TITLE 19, so FOLDER returns to the minimum.
        let layout = table_layout(66, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Updated, Title, Q, Size]);
        assert_eq!((layout.folder_w, layout.title_w), (FOLDER_MIN_W, 20));
        // Once a column is hidden FOLDER stays at the minimum, never regrowing.
        let layout = table_layout(65, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Updated, Title, Q]);
        assert_eq!((layout.folder_w, layout.title_w), (FOLDER_MIN_W, 25));
    }

    #[test]
    fn table_layout_keeps_agent_folder_title_when_still_narrow() {
        let layout = table_layout(30, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Title]);
        assert_eq!(layout.folder_w, FOLDER_MIN_W);
        assert!(layout.title_w < TITLE_MIN_W);
    }

    #[test]
    fn q_takes_the_right_margin_when_it_is_the_last_column() {
        assert_eq!(table_layout(66, 0).width(Q), 4);
        assert_eq!(table_layout(60, 0).width(Q), 5);
    }
}
