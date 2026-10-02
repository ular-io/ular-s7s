//! Workspace rendering: the workspace list (the Session screen's workspace
//! pane, and a display-only copy on the Workspaces screen), the Detail pane
//! (fields, folder search and checklist, and a footer resolving the cursor
//! row), the reused session table, and the deletion confirmation modal.

use super::state::{
    folder_display_label, WorkspaceField, DETAIL_FIELDS, FIRST_FOLDER_ROW, SEARCH_ROW,
};
use crate::theme::Theme;
use crate::ui::components::modal::{button_styles, modal_block, render_modal, titled_block_nav};
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::{count_note, fit_before_note, pad_w, truncate_w};
use crate::ui::render::{centered_fixed_rect, display_path, input_view};
use crate::ui::{App, UiMode};
use crate::workspaces::ALL_WORKSPACE_NAME;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Padding, Paragraph, Wrap},
    Frame,
};

/// Pane widths including borders. The list and Detail panes are capped; the
/// session table takes the rest and hides optional columns on its own. The
/// Session screen's workspace pane uses the same list width.
pub(crate) const LIST_MAX_W: u16 = 24;
/// Fixed last row of the workspace pane, outside the stored list.
pub(crate) const NEW_WORKSPACE_LABEL: &str = "[NEW WORKSPACE]";
const DETAIL_MAX_W: u16 = 40;
/// Session table width protected before the Detail pane may shrink.
const SESSIONS_MIN_W: u16 = 40;
const DETAIL_MIN_W: u16 = 24;
/// Detail field label column (`" Includes "`).
const LABEL_W: usize = 10;

/// `(list, detail, sessions)` widths for a body `total` cells wide. A narrow
/// terminal shrinks the Detail pane first, down to `DETAIL_MIN_W`; past that
/// the session table gives up its optional columns.
pub(crate) fn pane_widths(total: u16) -> (u16, u16, u16) {
    let list = LIST_MAX_W.min(total);
    let rest = total - list;
    let detail = if rest >= DETAIL_MAX_W + SESSIONS_MIN_W {
        DETAIL_MAX_W
    } else {
        rest.saturating_sub(SESSIONS_MIN_W)
            .max(DETAIL_MIN_W)
            .min(rest)
    };
    (list, detail, rest - detail)
}

pub(crate) fn draw_workspace_screen(f: &mut Frame, app: &App, area: Rect) {
    let body = if app.mode == UiMode::Keyword {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(area);
        crate::ui::session::render::draw_search_prompt(f, app, rows[0]);
        rows[1]
    } else {
        area
    };
    let (list_w, detail_w, _) = pane_widths(body.width);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(list_w),
            Constraint::Length(detail_w),
            Constraint::Min(0),
        ])
        .split(body);
    // Only the Detail pane takes keys here. As on the Session and Detail
    // screens, the other panes fade while it owns focus; an overlay or the
    // search prompt leaves none focused or dimmed.
    let active = matches!(app.mode, UiMode::Table | UiMode::WorkspaceEdit);
    let display_only = PaneFocus {
        focused: false,
        dimmed: active,
    };
    draw_list(f, app, cols[0], display_only, false);
    let detail = PaneFocus {
        focused: active,
        dimmed: false,
    };
    draw_detail(f, app, cols[1], detail);
    crate::ui::session::render::draw_table_with(
        f,
        app,
        cols[2],
        false,
        display_only.dimmed,
        (false, false),
    );
}

/// The Session screen's workspace pane: the list with `[NEW WORKSPACE]` last,
/// focused while it takes keys.
pub(crate) fn draw_workspace_pane(f: &mut Frame, app: &App, area: Rect) {
    let pane = PaneFocus {
        focused: app.mode == UiMode::Table,
        dimmed: false,
    };
    draw_list(f, app, area, pane, true);
}

#[derive(Clone, Copy)]
struct PaneFocus {
    focused: bool,
    /// Another pane owns focus, so this one renders `soft_dim()`.
    dimmed: bool,
}

/// Row style shared by the list and Detail panes, mirroring the session table:
/// a focused selection is `selection_bg` + bold, a dimmed pane fades every row
/// and keeps only the weak reversed signal on the selected one.
fn row_style(th: &Theme, selected: bool, pane: PaneFocus) -> Style {
    match (selected, pane.focused, pane.dimmed) {
        (true, true, _) => Style::default()
            .bg(th.selection_bg)
            .fg(th.selection_fg)
            .add_modifier(Modifier::BOLD),
        (true, false, true) => th
            .soft_dim()
            .bg(th.selection_inactive_bg)
            .add_modifier(Modifier::REVERSED),
        (true, false, false) => Style::default()
            .bg(th.selection_inactive_bg)
            .fg(th.selection_fg)
            .add_modifier(Modifier::BOLD),
        (false, _, true) => th.soft_dim(),
        (false, _, false) => Style::default(),
    }
}

/// Keeps `cursor` inside a `view`-row window, returning the new offset.
fn follow(scroll: usize, cursor: usize, view: usize) -> usize {
    if view == 0 {
        0
    } else if cursor < scroll {
        cursor
    } else if cursor >= scroll + view {
        cursor + 1 - view
    } else {
        scroll
    }
}

/// "All" first, then the stored workspaces, then `[NEW WORKSPACE]` when
/// `with_new_row` (the Session screen's pane; the Workspaces screen's copy
/// only shows which workspace is being edited).
fn draw_list(f: &mut Frame, app: &App, area: Rect, pane: PaneFocus, with_new_row: bool) {
    let th = &app.theme;
    let focused = pane.focused;
    let block = titled_block_nav(" Workspaces ", focused, true, true, th.accent);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 3 || inner.height == 0 {
        return;
    }
    let names: Vec<&str> = std::iter::once(ALL_WORKSPACE_NAME)
        .chain(app.workspaces.workspaces.iter().map(|w| w.name.as_str()))
        .chain(with_new_row.then_some(NEW_WORKSPACE_LABEL))
        .collect();
    let cursor = if with_new_row {
        app.workspace_pane_cursor()
    } else {
        app.workspaces.active_index().map_or(0, |i| i + 1)
    };
    let view = inner.height as usize;
    let scroll = follow(app.workspace.list_scroll.get(), cursor, view);
    app.workspace.list_scroll.set(scroll);

    // One-cell margins on both sides; the selected highlight spans them.
    let text_w = (inner.width as usize).saturating_sub(2);
    let mut lines = Vec::new();
    for (row, name) in names.iter().enumerate().skip(scroll).take(view) {
        let style = row_style(th, row == cursor, pane);
        lines.push(Line::from(Span::styled(
            format!(" {} ", pad_w(&truncate_w(name, text_w), text_w)),
            style,
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
    draw_vscrollbar(f, area, focused, scroll, names.len(), view, th);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect, pane: PaneFocus) {
    let th = &app.theme;
    let focused = pane.focused;
    let ws = app.workspaces.active_workspace();
    let locked = ws.is_none();
    // ←/→ do not leave this pane (Esc does), so no arrows on its frame.
    let block = titled_block_nav(" Detail ", focused, false, false, th.accent);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < LABEL_W as u16 + 2 || inner.height == 0 {
        return;
    }
    let state = &app.workspace;
    let w = inner.width as usize;
    let value_w = w.saturating_sub(LABEL_W + 1);
    let edit = state
        .edit
        .as_ref()
        .filter(|_| app.mode == UiMode::WorkspaceEdit);
    // A locked ("All") pane is never focusable, so no cursor row is drawn there.
    let cursor = (!locked).then_some(state.detail_cursor);
    let label_style = th.soft_dim();
    let value_style = if locked || pane.dimmed {
        th.soft_dim()
    } else {
        Style::default()
    };

    let mut lines: Vec<Line> = Vec::new();
    for (row, field) in DETAIL_FIELDS.iter().enumerate() {
        let (label, value) = match field {
            WorkspaceField::Name => ("Name", ws.map_or(ALL_WORKSPACE_NAME, |w| &w.name)),
            WorkspaceField::Includes => ("Includes", ws.map_or("", |w| &w.includes)),
            WorkspaceField::Excludes => ("Excludes", ws.map_or("", |w| &w.excludes)),
        };
        let style = row_style(th, cursor == Some(row), pane);
        let editing = edit.filter(|e| e.field == *field && cursor == Some(row));
        let value_span = match editing {
            Some(edit) => {
                let (visible, cursor_x) = input_view(&edit.input, value_w);
                f.set_cursor_position((inner.x + LABEL_W as u16 + cursor_x, inner.y + row as u16));
                Span::styled(pad_w(&visible, value_w), style)
            }
            None if value.trim().is_empty() => {
                Span::styled(pad_w("(none)", value_w), label_style.patch(style))
            }
            None => Span::styled(
                pad_w(&truncate_w(value, value_w), value_w),
                value_style.patch(style),
            ),
        };
        lines.push(Line::from(vec![
            Span::styled(
                pad_w(&format!(" {label}"), LABEL_W),
                label_style.patch(style),
            ),
            value_span,
            Span::styled(" ", style),
        ]));
    }
    lines.push(divider(w, th));
    let selected = ws.map_or(0, |w| w.folders.len());
    let scope = if selected == 0 {
        "all folders".to_string()
    } else {
        format!("{selected} selected")
    };
    lines.push(Line::from(vec![
        Span::styled(" Folders ", label_style.add_modifier(Modifier::BOLD)),
        Span::styled(format!("· {scope}"), label_style),
    ]));

    // Folder search row: typed into directly while the cursor is on it. It is
    // an input, so the row is never highlighted: the hardware cursor marks it,
    // and a whole-query selection paints only the text.
    let query = &state.folder_query;
    let search_focused = focused && cursor == Some(SEARCH_ROW) && app.mode == UiMode::Table;
    let mut search_line = vec![Span::styled(pad_w(" Search", LABEL_W), label_style)];
    if query.value.is_empty() {
        // The placeholder would sit after the hardware cursor, so it shows
        // only while the row is not focused.
        if !search_focused {
            search_line.push(Span::styled(
                truncate_w("type to filter", value_w),
                label_style,
            ));
        }
    } else {
        let (visible, _) = input_view(query, value_w);
        let text_style = if search_focused && query.select_all {
            Style::default().fg(th.selection_fg).bg(th.selection_bg)
        } else {
            value_style
        };
        search_line.push(Span::styled(visible, text_style));
    }
    if search_focused {
        let (_, cursor_x) = input_view(query, value_w);
        f.set_cursor_position((
            inner.x + LABEL_W as u16 + cursor_x,
            inner.y + lines.len() as u16,
        ));
    }
    lines.push(Line::from(search_line));

    // Folder viewport: the footer (divider + path) is dropped before the list
    // would lose its last usable row.
    let head = lines.len();
    let total_h = inner.height as usize;
    let (view, footer) = match total_h.saturating_sub(head) {
        h if h > 2 => (h - 2, true),
        h => (h, false),
    };
    let folder_cursor = cursor.and_then(|c| c.checked_sub(FIRST_FOLDER_ROW));
    let scroll = follow(
        state.folder_scroll.get(),
        folder_cursor.unwrap_or(state.folder_scroll.get()),
        view,
    );
    state.folder_scroll.set(scroll);
    // " [✓] " before the label and one trailing space.
    let label_w = w.saturating_sub(6);
    let shown = state.visible.len();
    for (i, folder) in state.visible_folders().enumerate().skip(scroll).take(view) {
        let checked = ws.is_some_and(|w| w.has_folder(folder));
        let mark = if checked { "[✓]" } else { "[ ]" };
        let label = folder_display_label(folder);
        let style = row_style(th, folder_cursor == Some(i), pane);
        // A dimmed pane drops the accent, as the session table drops agent
        // colors, but keeps a checked mark bold.
        let mark_style = match (checked && !locked, pane.dimmed) {
            (true, false) => Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            (true, true) => th.soft_dim().add_modifier(Modifier::BOLD),
            (false, _) => th.soft_dim(),
        };
        let note = count_note(state.folder_counts.get(folder).copied().unwrap_or(0));
        lines.push(Line::from(vec![
            Span::styled(" ", style),
            Span::styled(mark, mark_style.patch(style)),
            Span::styled(" ", style),
            Span::styled(
                fit_before_note(&label, label_w, &note),
                value_style.patch(style),
            ),
            Span::styled(note, style.patch(th.soft_dim())),
            Span::styled(" ", style),
        ]));
    }
    if shown == 0 && !query.value.is_empty() && view > 0 {
        lines.push(Line::from(Span::styled(
            truncate_w(" No matching folders", w),
            th.soft_dim(),
        )));
    }
    while lines.len() < head + view {
        lines.push(Line::from(""));
    }
    if footer {
        lines.push(divider(w, th));
        let note = if locked {
            "All sessions · cannot be edited".to_string()
        } else if let Some(folder) = state.cursor_folder().filter(|_| folder_cursor.is_some()) {
            display_path(folder)
        } else if state.cursor_on_search() {
            let total = state.folders.len();
            if query.value.is_empty() {
                "Type to filter folders · ↑↓ move".to_string()
            } else if query.select_all {
                format!("{shown}/{total} · type replaces · → edit")
            } else {
                format!("{shown} of {total} folders · esc clear")
            }
        } else {
            match state.cursor_field() {
                Some(WorkspaceField::Name) => "enter rename".to_string(),
                Some(WorkspaceField::Includes) => "Sessions must contain every word".to_string(),
                Some(WorkspaceField::Excludes) => "Sessions with any word are hidden".to_string(),
                None => String::new(),
            }
        };
        lines.push(Line::from(Span::styled(
            truncate_w(&format!(" {note}"), w),
            th.soft_dim(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
    if shown > view && view > 0 {
        let sb = Rect::new(
            area.x,
            inner.y + head as u16 - 1,
            area.width,
            view as u16 + 2,
        );
        draw_vscrollbar(f, sb, focused, scroll, shown, view, th);
    }
}

fn divider(w: usize, th: &Theme) -> Line<'static> {
    Line::from(Span::styled("─".repeat(w), Style::default().fg(th.dim)))
}

/// Workspace deletion confirmation. Sessions are untouched: a workspace is
/// only a saved filter.
pub(crate) fn draw_workspace_delete_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let name = app
        .pending_workspace_delete
        .and_then(|idx| app.workspaces.workspaces.get(idx))
        .map(|w| w.name.clone())
        .unwrap_or_else(|| "?".to_string());
    let area = centered_fixed_rect(70, 9, f.area());
    let block = modal_block(" Delete Workspace ", th.error).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;
    let content = vec![
        Line::from(Span::styled(
            truncate_w(&name, inner_w),
            Style::default().fg(th.error).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Only the workspace is removed. Its sessions are not deleted.",
            th.soft_dim(),
        )),
    ];
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), rows[0]);
    let (focused_style, unfocused) = button_styles(th);
    let (cancel_style, delete_style) = if app.delete_ok_focused {
        (unfocused, focused_style)
    } else {
        (focused_style, unfocused)
    };
    let buttons = Line::from(vec![
        Span::styled("  Delete  ", delete_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::pane_widths;

    #[test]
    fn panes_keep_their_caps_and_shrink_detail_first() {
        assert_eq!(pane_widths(200), (24, 40, 136));
        assert_eq!(pane_widths(104), (24, 40, 40));
        assert_eq!(pane_widths(100), (24, 36, 40));
        // Detail stops at its minimum; the session table absorbs the rest.
        assert_eq!(pane_widths(70), (24, 24, 22));
        assert_eq!(pane_widths(30), (24, 6, 0));
    }
}
