//! TUI rendering: k9s-style header (logo + hotkeys) / session search table / preview /
//! session details (questions list + work/answers) / status bar / modal windows.

use super::components::modal::{
    backdrop_dimmed, button_styles, dim_backdrop, modal_block, render_modal,
};
use super::components::text::truncate_w;
use super::{next_grapheme_boundary, App, Focus, Screen, TextInput, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

mod header;
mod meta;
mod usage;

use header::draw_header;
pub(crate) use meta::*;
pub(crate) use usage::*;

/// Renders the entire application frame.
pub fn draw(f: &mut Frame, app: &App) {
    // Paint the theme background/foreground under the whole frame first. Widgets that
    // set only fg patch on top of this, so every panel inherits the theme background.
    f.render_widget(Block::default().style(app.theme.base_style()), f.area());

    // Header content is capped at five rows (matching the logo and numbered profile limit).
    let header_h = 5.min(f.area().height.saturating_sub(4));
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h), // Header (hotkeys left, logo right)
            Constraint::Min(3),           // Body (table + preview)
            Constraint::Length(1),        // Status bar
        ])
        .split(f.area());

    draw_header(f, app, root[0]);

    if app.screen == Screen::Profile {
        // Profile view: full-width table without preview panel.
        super::profile::render::draw_profile_table(f, app, root[1]);
    } else if app.screen == Screen::Detail {
        super::detail::render::draw_detail(f, app, root[1]);
    } else if app.mode == UiMode::Keyword || app.full_search_active() {
        // Keyword or full-search prompt above the main body (k9s-style). The
        // full-search prompt stays while its results are browsed.
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(root[1]);
        super::session::prompt::draw_search_prompt(f, app, body[0]);
        draw_body(f, app, body[1]);
    } else {
        draw_body(f, app, root[1]);
    }

    draw_status_bar(f, app, root[2]);

    if backdrop_dimmed(app.mode) {
        dim_backdrop(f, &app.theme);
    }

    match app.mode {
        UiMode::AgentModal => super::overlays::filters::draw_agent_modal(f, app),
        UiMode::FolderModal => super::overlays::filters::draw_folder_modal(f, app),
        UiMode::DeleteConfirm => super::overlays::confirm::draw_delete_confirm(f, app),
        UiMode::AttachConfirm => super::overlays::attach::draw_attach_confirm(f, app),
        UiMode::Rename => super::overlays::confirm::draw_rename_modal(f, app),
        UiMode::ChangeFolder => super::overlays::change_folder::draw_change_folder_modal(f, app),
        UiMode::ProfileForm => super::profile::render::draw_profile_form(f, app),
        UiMode::ProfileDeleteConfirm => super::profile::render::draw_profile_delete_confirm(f, app),
        UiMode::ProfileDirConfirm => super::profile::render::draw_profile_dir_confirm(f, app),
        UiMode::NewSession => super::new_session::render::draw_new_session_modal(f, app),
        UiMode::ProjectDirConfirm => draw_project_dir_confirm(f, app),
        UiMode::QuickCommand => super::quick::draw_quick_command(f, app),
        UiMode::ThemeSelect => super::overlays::theme::draw_theme_select(f, app),
        UiMode::Help => super::overlays::help::draw_help(f, app),
        UiMode::Message => super::overlays::message::draw_message_modal(f, app),
        UiMode::WorkspaceEdit => super::workspace::render::draw_workspace_dialog(f, app),
        UiMode::WorkspaceDeleteConfirm => {
            super::workspace::render::draw_workspace_delete_confirm(f, app)
        }
        _ => {}
    }
}

/// Narrowest Prompt panel kept beside the session table while the workspace
/// pane is open; below it the Prompt is hidden and the table takes its width.
const PROMPT_MIN_W: u16 = 40;

/// Session screen body areas: `(workspace pane, table, prompt)`. The table and
/// Prompt split 58/42 whatever is left of the workspace pane.
pub(crate) fn body_layout(area: Rect, pane_open: bool) -> (Option<Rect>, Rect, Option<Rect>) {
    let (pane, rest) = if pane_open {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(super::workspace::render::LIST_MAX_W),
                Constraint::Min(0),
            ])
            .split(area);
        (Some(cols[0]), cols[1])
    } else {
        (None, area)
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rest);
    if pane_open && cols[1].width < PROMPT_MIN_W {
        return (pane, rest, None);
    }
    (pane, cols[0], Some(cols[1]))
}

/// Body: the workspace pane while it has focus, the session table, and the
/// preview panel.
fn draw_body(f: &mut Frame, app: &App, area: Rect) {
    let (pane, table, prompt) = body_layout(area, app.focus == Focus::Workspaces);
    if let Some(pane) = pane {
        super::workspace::render::draw_workspace_pane(f, app, pane);
    }
    super::session::render::draw_table(f, app, table);
    if let Some(prompt) = prompt {
        super::session::render::draw_preview(f, app, prompt);
    }
}

/// Formats path string, substituting home directory prefix with `~`.
pub(crate) fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.to_string_lossy());
        }
    }
    path.to_string_lossy().into_owned()
}

/// Confirmation modal for creating a new project folder under `config::projects_dir()`
/// when the New Session folder input is a bare name with no existing folder.
/// Create makes the folder and starts the session; Cancel returns to the dialog.
fn draw_project_dir_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let path_str = app
        .project_dir_pending
        .as_ref()
        .map(|p| display_path(p))
        .unwrap_or_else(|| "?".to_string());

    let area = centered_fixed_rect(70, 8, f.area());
    let block =
        modal_block(" Create Project Folder ", th.warning).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let content = vec![
        Line::from(Span::styled(
            truncate_w(&path_str, inner_w),
            Style::default().fg(th.warning).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "This folder does not exist. Create it and start the session?",
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
    let (ok_style, cancel_style) = if app.dir_create_ok_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("  Create  ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

/// Footer status bar rendered inside an explicit one-cell horizontal margin.
fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let dim_style = Style::default().fg(th.dim);
    let left: Line = if app.mode == UiMode::Keyword {
        Line::from(Span::styled("enter confirm  ·  esc cancel", dim_style))
    } else if app.mode == UiMode::Help {
        Line::from(Span::styled("esc/q/? close help", dim_style))
    } else if app.mode == UiMode::ThemeSelect {
        Line::from(Span::styled(
            "↑↓ preview  ·  enter apply  ·  esc cancel",
            dim_style,
        ))
    } else if let Some(msg) = &app.status_msg {
        Line::from(Span::styled(
            format!(" {} ", msg),
            Style::default().fg(th.on_accent).bg(th.accent),
        ))
    } else if app.screen == Screen::Detail {
        Line::from(Span::styled(". toggle tool logs  ·  ← back", dim_style))
    } else if app.mode == UiMode::NewSession {
        Line::from(Span::styled(
            "enter open/select  ·  ↑↓ move focus  ·  tab focus  ·  space select  ·  → complete  ·  esc close",
            dim_style,
        ))
    } else if app.mode == UiMode::Rename {
        Line::from(Span::styled("enter save  ·  esc cancel", dim_style))
    } else if app.mode == UiMode::WorkspaceEdit {
        let folder_list_open = app
            .workspace
            .dialog
            .as_ref()
            .is_some_and(|d| d.folder_list.is_some());
        let keys = if folder_list_open {
            "↑↓ move  ·  space toggle  ·  type filter  ·  enter close  ·  esc clear/close"
        } else {
            "↑↓/tab move  ·  enter open Folders  ·  enter on Save saves  ·  esc cancel"
        };
        Line::from(Span::styled(keys, dim_style))
    } else if app.screen == Screen::Session
        && app.focus == Focus::Workspaces
        && app.mode == UiMode::Table
        && !app.filter.is_active()
    {
        Line::from(Span::styled(
            "↑↓ open workspace  ·  enter edit  ·  + new  ·  →/esc close  ·  ← profiles",
            dim_style,
        ))
    } else if app.filter.is_active() {
        Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(th.on_accent).bg(th.accent)),
            Span::styled(
                format!(" {}", app.filter.describe_with(|id| app.profile_name(id))),
                th.soft_dim(),
            ),
        ])
    } else {
        Line::from(Span::styled("q/ctrl+c quit", dim_style))
    };

    f.render_widget(Paragraph::new(left), area.inner(Margin::new(1, 0)));
}

/// Returns the horizontal viewport containing the cursor without mutating the input text.
///
/// Since the terminal hardware cursor overlays the text, no placeholder characters are
/// inserted, preventing trailing characters from shifting.
pub(crate) fn input_view(state: &TextInput, width: usize) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }

    let cursor = state.cursor.min(state.value.len());
    let mut start = 0;
    let max_cursor_x = width.saturating_sub(1);

    // Scroll the viewport by whole grapheme clusters: starting mid-cluster would
    // both mis-measure the remaining width and render a dangling combining mark.
    while UnicodeWidthStr::width(&state.value[start..cursor]) > max_cursor_x {
        start = next_grapheme_boundary(&state.value, start);
    }

    let visible = truncate_w(&state.value[start..], width);
    let cursor_x = UnicodeWidthStr::width(&state.value[start..cursor]) as u16;
    (visible, cursor_x)
}

/// Centers a fixed size rectangle (clamped if exceeding terminal area).
pub(crate) fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod dialog_tests;
#[cfg(test)]
mod header_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod test_support;
