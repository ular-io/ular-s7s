//! Workspace rendering: the Session screen's workspace pane, the edit dialog
//! (field boxes and the match count beside the folder search and checklist, a
//! footer resolving the cursor row, and the Save/Cancel buttons), and the
//! deletion confirmation.

use super::state::{
    folder_display_label, WorkspaceDialog, WorkspaceField, DIALOG_FIELDS, FIRST_FOLDER_ROW,
};
use crate::theme::Theme;
use crate::ui::components::modal::{
    button_styles, form_input, joined_divider, modal_block, render_modal, titled_block_nav,
};
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::{count_note, fit_before_note, pad_w, truncate_w};
use crate::ui::render::{centered_fixed_rect, display_path, input_view};
use crate::ui::{App, TextInput, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Padding, Paragraph, Wrap},
    Frame,
};

/// Width of the Session screen's workspace pane, including borders.
pub(crate) const LIST_MAX_W: u16 = 24;
/// Fixed rows of the workspace list, outside the stored list. Bracketed upper
/// case marks a fixed option (as `[SCRATCH]` does), and a divider separates each
/// from the stored workspaces. Elsewhere "All" keeps its plain name.
pub(crate) const ALL_WORKSPACE_LABEL: &str = "[ALL]";
pub(crate) const UNASSIGNED_WORKSPACE_LABEL: &str = "[NO WORKSPACE]";
pub(crate) const NEW_WORKSPACE_LABEL: &str = "[NEW WORKSPACE]";
/// Label column of the dialog's unboxed rows (`" Matches "`, `" Search "`).
const LABEL_W: usize = 10;
/// Dialog width including its outer margin, capped at 90% of the terminal
/// rather than the usual 80%: the body is split into two columns.
const DIALOG_MAX_W: u16 = 86;
/// Dialog rows besides the body: borders (2) and top padding (1); divider,
/// footer, blank, and buttons (4).
const DIALOG_CHROME_H: u16 = 7;
/// Left column height: the Name, Includes, and Excludes boxes (9) and Matches.
const FIELDS_H: u16 = 10;
/// Right column rows above the folder rows: the Folders heading and Search.
const FOLDER_HEAD_H: u16 = 2;
/// Separator column between the two body columns: a space and `│`.
const COLUMN_GAP_W: u16 = 2;
/// Folder rows kept when there are fewer folders (or none), so the dialog
/// never collapses around an empty list.
const DIALOG_MIN_FOLDER_ROWS: u16 = 3;

/// `(width, height)` of the edit dialog on a `full`-sized terminal: tall
/// enough for the field column and every one of `folders` rows, capped at 90%
/// of the terminal height. Sized by every folder rather than the search
/// matches, so typing a query never moves the buttons.
pub(crate) fn dialog_size(full: Rect, folders: usize) -> (u16, u16) {
    let width = DIALOG_MAX_W.min((u32::from(full.width) * 9 / 10) as u16);
    let rows = (folders as u32).max(u32::from(DIALOG_MIN_FOLDER_ROWS));
    let body = u32::from(FIELDS_H).max(u32::from(FOLDER_HEAD_H) + rows);
    let want = u32::from(DIALOG_CHROME_H) + body;
    let cap = u32::from(full.height) * 9 / 10;
    (width, want.min(cap) as u16)
}

/// The Session screen's workspace pane: fixed All/Unassigned rows, saved
/// workspaces, then New Workspace; focused while it takes keys.
pub(crate) fn draw_workspace_pane(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let focused = app.mode == UiMode::Table;
    let block = titled_block_nav(" Workspaces ", focused, false, true, true, th);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 3 || inner.height == 0 {
        return;
    }
    // Display rows: `Some((label, fixed))` is a cursor row, `None` a divider the
    // cursor never lands on. A list with no stored workspace keeps one divider.
    let stored = &app.workspaces.workspaces;
    let mut rows: Vec<Option<(&str, bool)>> = vec![
        Some((ALL_WORKSPACE_LABEL, true)),
        Some((UNASSIGNED_WORKSPACE_LABEL, true)),
        None,
    ];
    rows.extend(stored.iter().map(|w| Some((w.name.as_str(), false))));
    if !stored.is_empty() {
        rows.push(None);
    }
    rows.push(Some((NEW_WORKSPACE_LABEL, true)));
    // Cursor rows in display order, so the pane cursor maps onto its display row.
    let cursor_row = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.is_some())
        .nth(app.workspace_pane_cursor())
        .map_or(0, |(i, _)| i);
    let view = inner.height as usize;
    let scroll = follow(app.workspace.list_scroll.get(), cursor_row, view);
    app.workspace.list_scroll.set(scroll);

    // One-cell margins on both sides; the selected highlight spans them.
    let inner_w = inner.width as usize;
    let text_w = inner_w.saturating_sub(2);
    let mut lines = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(scroll).take(view) {
        lines.push(match row {
            Some((name, fixed)) => {
                let selected = i == cursor_row;
                let mut style = row_style(th, selected, focused);
                // Fixed rows also take the key-hint color; the selection keeps its own.
                if *fixed && !selected {
                    style = style.fg(th.key_hint);
                }
                Line::from(Span::styled(
                    format!(" {} ", pad_w(&truncate_w(name, text_w), text_w)),
                    style,
                ))
            }
            None => divider(inner_w, th),
        });
    }
    f.render_widget(Paragraph::new(lines), inner);
    draw_vscrollbar(f, area, focused, scroll, rows.len(), view, th);
}

/// Cursor row style, mirroring the session table: a focused selection is
/// `selection_bg` + bold; while a dialog or the search prompt owns input it
/// keeps `selection_inactive_bg`.
fn row_style(th: &Theme, selected: bool, focused: bool) -> Style {
    match (selected, focused) {
        (true, true) => Style::default()
            .bg(th.selection_bg)
            .fg(th.selection_fg)
            .add_modifier(Modifier::BOLD),
        (true, false) => Style::default()
            .bg(th.selection_inactive_bg)
            .fg(th.selection_fg)
            .add_modifier(Modifier::BOLD),
        (false, _) => Style::default(),
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

/// The folder search row. It stays unboxed like the Select Folders search line:
/// a box would cost two of the folder rows. A text input is never painted as a
/// cursor row: a focused row bolds its label in the accent color and shows the
/// hardware cursor, and a whole-value selection paints only the text. The
/// `type to filter` placeholder fills an empty row only while it is not
/// focused, because the hardware cursor sits where it would start.
fn search_row(
    f: &mut Frame,
    (x, y, value_w): (u16, u16, usize),
    input: &TextInput,
    focused: bool,
    th: &Theme,
) -> Line<'static> {
    let label_style = if focused {
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
    } else {
        th.soft_dim()
    };
    let mut spans = vec![Span::styled(pad_w(" Search", LABEL_W), label_style)];
    if input.value.is_empty() {
        if !focused {
            spans.push(Span::styled(
                truncate_w("type to filter", value_w),
                th.soft_dim(),
            ));
        }
    } else {
        let (visible, _) = input_view(input, value_w);
        let style = if focused && input.select_all {
            Style::default().fg(th.selection_fg).bg(th.selection_bg)
        } else {
            Style::default()
        };
        spans.push(Span::styled(visible, style));
    }
    if focused {
        let (_, cursor_x) = input_view(input, value_w);
        f.set_cursor_position((x + LABEL_W as u16 + cursor_x, y));
    }
    Line::from(spans)
}

/// The workspace edit dialog (`UiMode::WorkspaceEdit`).
pub(crate) fn draw_workspace_dialog(f: &mut Frame, app: &App) {
    let Some(dialog) = &app.workspace.dialog else {
        return;
    };
    let th = &app.theme;
    let full = f.area();
    let (w, h) = dialog_size(full, dialog.folders.len());
    let area = centered_fixed_rect(w, h, full);
    let title = if dialog.created {
        " New Workspace "
    } else {
        " Edit Workspace "
    };
    let block = modal_block(title, th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    if inner.width < LABEL_W as u16 + 2 || inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // body: fields | folders
            Constraint::Length(1), // divider
            Constraint::Length(1), // footer: error or the cursor row
            Constraint::Length(1), // blank above the buttons
            Constraint::Length(1), // buttons
        ])
        .split(inner);
    let width = inner.width as usize;
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(COLUMN_GAP_W),
            Constraint::Fill(1),
        ])
        .split(rows[0]);
    draw_dialog_fields(f, app, dialog, columns[0]);

    // Column separator, joined to the divider below the body with `┴`.
    let sep_x = columns[1].x + COLUMN_GAP_W - 1;
    let sep = vec![Line::from("│"); rows[0].height as usize];
    f.render_widget(
        Paragraph::new(sep).style(Style::default().fg(th.dim)),
        Rect::new(sep_x, rows[0].y, 1, rows[0].height),
    );

    let right = columns[2];
    let folder_col = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Folders heading
            Constraint::Length(1), // search
            Constraint::Min(0),    // folder rows
        ])
        .split(right);
    let selected = dialog.draft.folders.len();
    let scope = if selected == 0 {
        "all folders".to_string()
    } else {
        format!("{selected} selected")
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Folders ", th.soft_dim().add_modifier(Modifier::BOLD)),
            Span::styled(format!("· {scope}"), th.soft_dim()),
        ])),
        folder_col[0],
    );
    let value_w = (right.width as usize).saturating_sub(LABEL_W + 1);
    let search = search_row(
        f,
        (right.x, folder_col[1].y, value_w),
        &dialog.folder_query,
        dialog.cursor_on_search(),
        th,
    );
    f.render_widget(Paragraph::new(search), folder_col[1]);
    draw_dialog_folders(f, app, dialog, area, folder_col[2]);

    joined_divider(f, area, rows[1].y, th);
    f.buffer_mut()[(sep_x, rows[1].y)]
        .set_symbol("┴")
        .set_style(Style::default().fg(th.dim));

    let (note, note_style) = match &dialog.error {
        Some(err) => (err.clone(), Style::default().fg(th.error)),
        None => (dialog_footer(dialog), th.soft_dim()),
    };
    f.render_widget(
        Paragraph::new(truncate_w(&format!(" {note}"), width)).style(note_style),
        rows[2],
    );

    // Buttons are highlighted only while the button row has focus.
    let (focused_style, unfocused) = button_styles(th);
    let (save_style, cancel_style) = match (dialog.on_buttons, dialog.save_focused) {
        (false, _) => (unfocused, unfocused),
        (true, true) => (focused_style, unfocused),
        (true, false) => (unfocused, focused_style),
    };
    let buttons = Line::from(vec![
        Span::styled("   Save   ", save_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[4],
    );
}

/// Left body column: the Name, Includes, and Excludes boxes stacked, then the
/// Matches count.
fn draw_dialog_fields(f: &mut Frame, app: &App, dialog: &WorkspaceDialog, area: Rect) {
    let th = &app.theme;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);
    for (row, field) in DIALOG_FIELDS.iter().enumerate() {
        let (label, placeholder) = match field {
            WorkspaceField::Name => (" Name ", ""),
            WorkspaceField::Includes => (" Includes ", "(none)"),
            WorkspaceField::Excludes => (" Excludes ", "(none)"),
        };
        let focused = !dialog.on_buttons && dialog.cursor == row;
        form_input(
            f,
            rows[row],
            label,
            dialog.input(*field),
            focused,
            placeholder,
            th,
        );
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(pad_w(" Matches", LABEL_W), th.soft_dim()),
            Span::raw(format!("{} ", dialog.matching)),
            Span::styled(format!("of {} sessions", app.sessions.len()), th.soft_dim()),
        ])),
        rows[3],
    );
}

/// Folder checklist: `[✓]`/`[ ]`, the bare basename, and a right-aligned
/// ` (N)` session count, scrolled to keep the cursor visible.
fn draw_dialog_folders(
    f: &mut Frame,
    app: &App,
    dialog: &WorkspaceDialog,
    dialog_area: Rect,
    list: Rect,
) {
    let th = &app.theme;
    let width = list.width as usize;
    let view = list.height as usize;
    let folder_cursor = dialog
        .cursor_folder()
        .and(dialog.cursor.checked_sub(FIRST_FOLDER_ROW));
    let scroll = follow(
        dialog.folder_scroll.get(),
        folder_cursor.unwrap_or(dialog.folder_scroll.get()),
        view,
    );
    dialog.folder_scroll.set(scroll);
    // " [✓] " before the label and one trailing space.
    let label_w = width.saturating_sub(6);
    let mut lines = Vec::new();
    for (i, folder) in dialog.visible_folders().enumerate().skip(scroll).take(view) {
        let checked = dialog.draft.has_folder(folder);
        let mark = if checked { "[✓]" } else { "[ ]" };
        let style = row_style(th, folder_cursor == Some(i), true);
        let mark_style = if checked {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            th.soft_dim()
        };
        let note = count_note(dialog.folder_counts.get(folder).copied().unwrap_or(0));
        lines.push(Line::from(vec![
            Span::styled(" ", style),
            Span::styled(mark, mark_style.patch(style)),
            Span::styled(" ", style),
            Span::styled(
                fit_before_note(&folder_display_label(folder), label_w, &note),
                style,
            ),
            Span::styled(note, style.patch(th.soft_dim())),
            Span::styled(" ", style),
        ]));
    }
    if dialog.visible.is_empty() && view > 0 {
        let empty = if dialog.folder_query.value.is_empty() {
            " No session folders"
        } else {
            " No matching folders"
        };
        lines.push(Line::from(Span::styled(
            truncate_w(empty, width),
            th.soft_dim(),
        )));
    }
    f.render_widget(Paragraph::new(lines), list);
    let shown = dialog.visible.len();
    if shown > view && view > 0 {
        // The dialog frame sits one cell inside its outer margin.
        let sb = Rect::new(
            dialog_area.x + 1,
            list.y - 1,
            dialog_area.width.saturating_sub(2),
            view as u16 + 2,
        );
        draw_vscrollbar(f, sb, true, scroll, shown, view, th);
    }
}

/// Footer line resolving the cursor row: the full path of a folder row, what
/// a field does, the folder search state, or what a button does.
fn dialog_footer(dialog: &WorkspaceDialog) -> String {
    if dialog.on_buttons {
        return match (dialog.save_focused, dialog.created) {
            (true, true) => "Add the workspace and open it".to_string(),
            (true, false) => "Save the changes".to_string(),
            (false, _) => "Discard the changes".to_string(),
        };
    }
    if let Some(folder) = dialog.cursor_folder() {
        return display_path(folder);
    }
    if dialog.cursor_on_search() {
        let shown = dialog.visible.len();
        let total = dialog.folders.len();
        let query = &dialog.folder_query;
        return if query.value.is_empty() {
            "Type to filter folders · ↑↓ move".to_string()
        } else if query.select_all {
            format!("{shown}/{total} · type replaces · → edit")
        } else {
            format!("{shown} of {total} folders · esc clear")
        };
    }
    match dialog.cursor_field() {
        Some(WorkspaceField::Name) => "Unique name, listed in the palette".to_string(),
        Some(WorkspaceField::Includes) => "Sessions must contain every word".to_string(),
        Some(WorkspaceField::Excludes) => "Sessions with any word are hidden".to_string(),
        None => String::new(),
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
    use super::dialog_size;
    use ratatui::layout::Rect;

    #[test]
    fn dialog_fits_every_folder_up_to_ninety_percent_of_the_terminal() {
        let term = |w, h| Rect::new(0, 0, w, h);
        // 7 rows of chrome plus the Folders heading, Search, and a row per folder.
        assert_eq!(dialog_size(term(200, 50), 10), (86, 19));
        // The field column (10 rows) sets the floor for a few folders.
        assert_eq!(dialog_size(term(200, 50), 0), (86, 17));
        assert_eq!(dialog_size(term(200, 50), 8), (86, 17));
        // Many folders stop at 90% of the height (45 of 50, 21 of 24), and the
        // width at 90% of a narrow terminal.
        assert_eq!(dialog_size(term(200, 50), 500), (86, 45));
        assert_eq!(dialog_size(term(80, 24), 500), (72, 21));
    }
}
