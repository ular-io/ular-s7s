//! Workspace rendering: the Session screen's workspace pane, the edit dialog
//! (stacked field boxes with the `Folders` combo, the match count, a notice
//! line for a refused Save, the Save/Cancel buttons, and the combo's folder
//! checklist popup), and the deletion confirmation.

use super::state::{folder_display_label, WorkspaceDialog, WorkspaceField, DIALOG_FIELDS};
use crate::theme::Theme;
use crate::ui::components::modal::{
    button_styles, draw_combo, dropdown_divider, dropdown_frame, form_input, modal_block,
    render_modal, titled_block_nav,
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
use unicode_width::UnicodeWidthStr;

/// Width of the Session screen's workspace pane, including borders.
pub(crate) const LIST_MAX_W: u16 = 24;
/// Fixed rows of the workspace list, outside the stored list. Bracketed upper
/// case marks a fixed option (as `[SCRATCH]` does), and a divider separates each
/// from the stored workspaces. Elsewhere "All" keeps its plain name.
pub(crate) const ALL_WORKSPACE_LABEL: &str = "[ALL]";
pub(crate) const UNASSIGNED_WORKSPACE_LABEL: &str = "[NO WORKSPACE]";
pub(crate) const NEW_WORKSPACE_LABEL: &str = "[NEW WORKSPACE]";
/// Fixed first row of the folder checklist: an empty selection, every folder.
/// Bracketed upper case like the other fixed rows, against bare basenames.
pub(crate) const ALL_FOLDERS_LABEL: &str = "[ALL FOLDERS]";
/// Label column of the dialog's unboxed rows (`" Matches "`, `" Search "`).
const LABEL_W: usize = 10;
/// Dialog width including its outer margin, capped at 80% of the terminal.
const DIALOG_MAX_W: u16 = 86;
/// Dialog height: borders (2) and top padding (1); the Name, Folders,
/// Includes, and Excludes boxes (12) and Matches; notice, blank, and buttons
/// (3). A form has no divider above its buttons.
const DIALOG_H: u16 = 19;
/// Checklist popup rows besides the folder rows: borders (2), the search line
/// and its divider (2).
const LIST_CHROME_H: u16 = 4;
/// Footer under the checklist rows: a divider and the cursor row's full path.
const LIST_FOOTER_H: u16 = 2;

/// `(width, height)` of the edit dialog on a `full`-sized terminal. The
/// folder checklist is a popup, so the dialog never changes size.
pub(crate) fn dialog_size(full: Rect) -> (u16, u16) {
    let width = DIALOG_MAX_W.min((u32::from(full.width) * 8 / 10) as u16);
    (width, DIALOG_H.min(full.height))
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
    let mut rows: Vec<Option<(&str, bool)>> = vec![Some((ALL_WORKSPACE_LABEL, true))];
    if !stored.is_empty() {
        rows.push(None);
        rows.extend(stored.iter().map(|w| Some((w.name.as_str(), false))));
    }
    rows.push(Some((UNASSIGNED_WORKSPACE_LABEL, true)));
    rows.push(None);
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

/// The checklist's search line. It stays unboxed like the Select Folders
/// search line: a box would cost two of the folder rows. It always has focus
/// while the checklist is open, so it bolds its label in the accent color and
/// shows the hardware cursor, with no placeholder where the cursor starts.
fn search_row(
    f: &mut Frame,
    (x, y, value_w): (u16, u16, usize),
    input: &TextInput,
    th: &Theme,
) -> Line<'static> {
    let label_style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    let (visible, cursor_x) = input_view(input, value_w);
    f.set_cursor_position((x + LABEL_W as u16 + cursor_x, y));
    Line::from(vec![
        Span::styled(pad_w(" Search", LABEL_W), label_style),
        Span::raw(visible),
    ])
}

/// The workspace edit dialog (`UiMode::WorkspaceEdit`).
pub(crate) fn draw_workspace_dialog(f: &mut Frame, app: &App) {
    let Some(dialog) = &app.workspace.dialog else {
        return;
    };
    let th = &app.theme;
    let full = f.area();
    let (w, h) = dialog_size(full);
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
            Constraint::Length(3), // Name
            Constraint::Length(3), // Folders combo
            Constraint::Length(3), // Includes
            Constraint::Length(3), // Excludes
            Constraint::Length(1), // Matches
            Constraint::Min(0),
            Constraint::Length(1), // notice: a refused Save's reason
            Constraint::Length(1), // blank above the buttons
            Constraint::Length(1), // buttons
        ])
        .split(inner);
    let width = inner.width as usize;
    for (row, field) in DIALOG_FIELDS.iter().enumerate() {
        let focused = !dialog.on_buttons && dialog.cursor == row;
        let (label, placeholder) = match field {
            WorkspaceField::Name => (" Name ", ""),
            // A combo: `draw_combo` adds the padding and the `▾`.
            WorkspaceField::Folders => ("Folders", ""),
            // The titles carry the word rule: every include word must
            // occur, while any one exclude word hides a session.
            WorkspaceField::Includes => (" Includes · all words ", "(none)"),
            WorkspaceField::Excludes => (" Excludes · any word ", "(none)"),
        };
        match dialog.input(*field) {
            Some(input) => form_input(f, rows[row], label, input, focused, placeholder, th),
            None => folder_combo(f, rows[row], label, dialog, focused, th),
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(pad_w(" Matches", LABEL_W), th.soft_dim()),
            Span::raw(format!("{} ", dialog.matching)),
            Span::styled(format!("of {} sessions", app.sessions.len()), th.soft_dim()),
        ])),
        rows[4],
    );

    // The notice line holds only a refused Save's reason. It stays reserved
    // while empty, so a refusal never moves the buttons.
    if let Some(err) = &dialog.error {
        f.render_widget(
            Paragraph::new(truncate_w(&format!(" {err}"), width))
                .style(Style::default().fg(th.error)),
            rows[6],
        );
    }

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
        rows[8],
    );

    // Drawn last: the popup covers the rows below the combo, and past the
    // dialog when the terminal leaves room.
    if let Some(cursor) = dialog.folder_list {
        draw_folder_list(f, app, dialog, area, rows[1], cursor);
    }
}

/// The closed `Folders` combo (`draw_combo`: `Thick` accent while focused or
/// open, `Plain` `dim` otherwise, `▾` at the inner right end), showing the
/// selection from `folder_summary` with its folder count dim at the right edge
/// of the value area, just left of the `▾`.
fn folder_combo(
    f: &mut Frame,
    area: Rect,
    label: &str,
    dialog: &WorkspaceDialog,
    focused: bool,
    th: &Theme,
) {
    let value = draw_combo(f, area, label, focused, th);
    let width = value.width as usize;
    let (names, count) = folder_summary(dialog, width);
    let line = match count {
        Some(count) => Line::from(vec![
            Span::raw(pad_w(&names, width.saturating_sub(count.width()))),
            Span::styled(count, th.soft_dim()),
        ]),
        None => Line::from(names),
    };
    f.render_widget(Paragraph::new(line), value);
}

/// The combo's value as `(names, count)` for a `width`-cell row: `All
/// folders` for an empty selection, a single basename (truncated), or for two
/// or more the basenames in checklist order and a `N folders` count drawn dim
/// at the right edge. Names are cut at name boundaries and end with `…` when
/// some do not fit; the count stays, so a narrow row never hides how many.
pub(crate) fn folder_summary(dialog: &WorkspaceDialog, width: usize) -> (String, Option<String>) {
    let selected = dialog.selected_folders();
    let names: Vec<String> = selected.iter().map(|p| folder_display_label(p)).collect();
    match names.as_slice() {
        [] => ("All folders".to_string(), None),
        [one] => (truncate_w(one, width), None),
        many => {
            let count = format!("{} folders", many.len());
            if width <= count.width() {
                return (String::new(), Some(truncate_w(&count, width)));
            }
            // One cell keeps the names off the count.
            let budget = width - count.width() - 1;
            let mut shown = String::new();
            let mut fitted = 0;
            for (i, name) in many.iter().enumerate() {
                let candidate = if shown.is_empty() {
                    name.clone()
                } else {
                    format!("{shown}, {name}")
                };
                // A name that is not the last must also leave room for `, …`.
                let reserve = if i + 1 < many.len() {
                    ", …".width()
                } else {
                    0
                };
                if candidate.width() + reserve > budget {
                    break;
                }
                shown = candidate;
                fitted += 1;
            }
            if fitted < many.len() {
                shown = if shown.is_empty() {
                    truncate_w("…", budget)
                } else {
                    format!("{shown}, …")
                };
            }
            (shown, Some(count))
        }
    }
}

/// The open folder checklist, a popup joined under the `Folders` combo
/// (`anchor`): the search line, the fixed `[ALL FOLDERS]` row and the folder
/// rows (`[✓]`/`[ ]`, the bare basename, and a right-aligned ` (N)` session
/// count), and a footer with the cursor row's full path. It is sized by every
/// folder rather than the search matches, so typing never resizes it, and it
/// stops at the terminal bottom; rows past that scroll.
fn draw_folder_list(
    f: &mut Frame,
    app: &App,
    dialog: &WorkspaceDialog,
    dialog_area: Rect,
    anchor: Rect,
    cursor: usize,
) {
    let th = &app.theme;
    let full = f.area();
    let popup_y = anchor.bottom().saturating_sub(1);
    let avail = full.bottom().saturating_sub(popup_y);
    // The fixed row plus every folder (one message row when there are none).
    let rows = 1 + dialog.folders.len().max(1) as u16;
    // The footer only stays while two list rows remain beside it.
    let footer_h = if avail >= LIST_CHROME_H + LIST_FOOTER_H + 2 {
        LIST_FOOTER_H
    } else {
        0
    };
    let popup_h = (LIST_CHROME_H + rows + footer_h).min(avail);
    if popup_h < LIST_CHROME_H + 1 {
        return;
    }
    let popup = Rect::new(anchor.x, popup_y, anchor.width, popup_h);
    let border = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    let inner = dropdown_frame(f, popup, dialog_area, border, th);
    let width = inner.width as usize;

    let value_w = width.saturating_sub(LABEL_W + 1);
    let search = search_row(f, (inner.x, inner.y, value_w), &dialog.folder_query, th);
    f.render_widget(Paragraph::new(search), Rect { height: 1, ..inner });
    let line = Style::default().fg(th.dim);
    dropdown_divider(f, popup, inner.y + 1, border, line);

    let view = inner.height.saturating_sub(2 + footer_h);
    let list = Rect::new(inner.x, inner.y + 2, inner.width, view);
    let view = view as usize;
    let scroll = follow(dialog.folder_scroll.get(), cursor, view);
    dialog.folder_scroll.set(scroll);
    // " [✓] " before the label and one trailing space.
    let label_w = width.saturating_sub(6);
    let total = dialog.list_rows();
    let mut lines = Vec::new();
    for i in (0..total).skip(scroll).take(view) {
        let (label, checked, count) = match i.checked_sub(1) {
            None => (
                ALL_FOLDERS_LABEL.to_string(),
                dialog.draft.folders.is_empty(),
                app.sessions.len(),
            ),
            Some(pos) => {
                let folder = &dialog.folders[dialog.visible[pos]];
                (
                    folder_display_label(folder),
                    dialog.draft.has_folder(folder),
                    dialog.folder_counts.get(folder).copied().unwrap_or(0),
                )
            }
        };
        let mark = if checked { "[✓]" } else { "[ ]" };
        let style = row_style(th, i == cursor, true);
        let mark_style = if checked {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            th.soft_dim()
        };
        let note = count_note(count);
        lines.push(Line::from(vec![
            Span::styled(" ", style),
            Span::styled(mark, mark_style.patch(style)),
            Span::styled(" ", style),
            Span::styled(fit_before_note(&label, label_w, &note), style),
            Span::styled(note, style.patch(th.soft_dim())),
            Span::styled(" ", style),
        ]));
    }
    if dialog.visible.is_empty() && lines.len() < view {
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
    if total > view && view > 0 {
        let sb = Rect::new(popup.x, list.y - 1, popup.width, view as u16 + 2);
        draw_vscrollbar(f, sb, true, scroll, total, view, th);
    }

    if footer_h > 0 {
        let divider_y = list.bottom();
        dropdown_divider(f, popup, divider_y, border, line);
        let path = match dialog.cursor_folder() {
            Some(folder) => display_path(folder),
            None => "Every folder: no folder restriction".to_string(),
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!(" {}", truncate_w(&path, width.saturating_sub(1))),
                th.soft_dim(),
            )),
            Rect::new(inner.x, divider_y + 1, inner.width, 1),
        );
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
    fn dialog_has_a_fixed_height_and_eighty_percent_width_cap() {
        let term = |w, h| Rect::new(0, 0, w, h);
        assert_eq!(dialog_size(term(200, 50)), (86, 19));
        // 80% of an 80-column terminal; a short terminal clips the height.
        assert_eq!(dialog_size(term(80, 24)), (64, 19));
        assert_eq!(dialog_size(term(80, 16)), (64, 16));
    }
}
