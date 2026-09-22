//! Session folder change dialog (`Change Folder`, palette-only).
//!
//! Changes the folder a stored session is opened in next time. Nothing on disk
//! moves: no file is relocated and the agent's transcript is never rewritten.
//! The chosen folder is kept in the s7s-owned store
//! (`config::session_workspaces_path`) and wins over the folder read from the
//! transcript at scan time (`scan::apply_workspace_cwd`), so the list column and
//! the resume launch directory follow it together.
//!
//! Antigravity is refused. A resumed agy conversation keeps working in the
//! folder captured when it was created — launching it elsewhere changes only the
//! header — so an override would make the list disagree with where the work
//! happens. See [session-folder.md](../../../docs/session-folder.md).

use crate::model::Agent;
use crate::ui::components::modal::{button_styles, modal_block, render_modal};
use crate::ui::components::text::truncate_w;
use crate::ui::new_session::input::resolve_input_path;
use crate::ui::new_session::state::is_bare_project_name;
use crate::ui::render::{centered_fixed_rect, input_view};
use crate::ui::{App, TextInput, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
    Frame,
};
use std::path::PathBuf;

// ---- State ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeFolderFocus {
    Input,
    Buttons,
}

pub struct ChangeFolderState {
    pub input: TextInput,
    /// Every folder already known from the session list, sorted and de-duplicated.
    pub folders: Vec<PathBuf>,
    /// Indices into `folders`; the first `match_count` entries match the input.
    pub ordered: Vec<usize>,
    pub match_count: usize,
    /// Highlighted pick-list row, or `None` while the text input owns the
    /// highlight — which is also the state in which Enter commits.
    pub cursor: Option<usize>,
    pub focus: ChangeFolderFocus,
    pub ok_focused: bool,
    pub error: Option<String>,
}

impl ChangeFolderState {
    /// Reorders the pick list so input matches lead. Non-matching folders are
    /// appended rather than hidden, mirroring the New Session folder dropdown so
    /// the list never appears to lose entries while typing.
    pub(crate) fn rank(&mut self) {
        let q = crate::normalize::nfc_lower(self.input.value.trim());
        let mut matched = Vec::new();
        let mut rest = Vec::new();
        for (i, path) in self.folders.iter().enumerate() {
            let is_match = if q.is_empty() {
                true
            } else {
                let full = crate::normalize::nfc_lower(&path.to_string_lossy());
                let name = path
                    .file_name()
                    .map(|n| crate::normalize::nfc_lower(&n.to_string_lossy()))
                    .unwrap_or_default();
                full.contains(&q) || name.contains(&q)
            };
            if is_match {
                matched.push(i);
            } else {
                rest.push(i);
            }
        }
        self.match_count = matched.len();
        matched.extend(rest);
        self.ordered = matched;
        if let Some(c) = self.cursor {
            match self.ordered.len() {
                0 => self.cursor = None,
                len => self.cursor = Some(c.min(len - 1)),
            }
        }
    }

    /// Moves the highlight between the text input (`None`) and the pick list.
    fn move_cursor(&mut self, delta: isize) {
        if self.ordered.is_empty() {
            self.cursor = None;
            return;
        }
        let last = self.ordered.len() - 1;
        self.cursor = match (self.cursor, delta) {
            (None, d) if d > 0 => Some(0),
            (None, _) => None,
            (Some(0), d) if d < 0 => None,
            (Some(c), d) if d < 0 => Some(c - 1),
            (Some(c), _) => Some((c + 1).min(last)),
        };
    }

    /// Writes the highlighted folder into the input and returns the highlight to
    /// the input, so the next Enter commits the value the user just picked.
    fn apply_cursor_row(&mut self) {
        let Some(path) = self
            .cursor
            .and_then(|row| self.ordered.get(row))
            .and_then(|&i| self.folders.get(i))
        else {
            return;
        };
        self.input = TextInput::selected(path.to_string_lossy().into_owned());
        self.error = None;
        self.rank();
        self.cursor = None;
    }

    /// Moves between the two controls. Tab and Shift+Tab do the same thing here
    /// because the dialog has only the input and the button row.
    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            ChangeFolderFocus::Input => ChangeFolderFocus::Buttons,
            ChangeFolderFocus::Buttons => ChangeFolderFocus::Input,
        };
    }

    fn after_edit(&mut self) {
        self.error = None;
        self.cursor = None;
        self.rank();
    }
}

// ---- Input ----

impl App {
    /// Opens the folder change dialog for the session at `idx`.
    ///
    /// Antigravity sessions are refused here rather than in the palette alone,
    /// so every entry point states the same reason.
    pub(crate) fn open_change_folder_at(&mut self, idx: usize) {
        let Some(session) = self.sessions.get(idx) else {
            self.status_msg = Some("No session selected".to_string());
            return;
        };
        if session.agent == Agent::Antigravity {
            self.status_msg = Some(
                "Antigravity keeps a resumed session in its original folder — changing it here would not move the work"
                    .to_string(),
            );
            return;
        }
        let current = session.cwd.to_string_lossy().into_owned();
        let mut folders: Vec<PathBuf> = self
            .sessions
            .iter()
            .map(|s| s.cwd.clone())
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        folders.push(crate::scratch::dir());
        folders.sort_unstable();
        folders.dedup();

        let mut state = ChangeFolderState {
            // Prefilled and selected: typing replaces the current folder outright,
            // which is the common case when the session landed in the wrong one.
            input: TextInput::selected(current),
            folders,
            ordered: Vec::new(),
            match_count: 0,
            cursor: None,
            focus: ChangeFolderFocus::Input,
            ok_focused: true,
            error: None,
        };
        state.rank();
        self.change_folder = Some(state);
        self.change_folder_target = Some(idx);
        self.mode = UiMode::ChangeFolder;
        self.status_msg = None;
    }

    pub(crate) fn cancel_change_folder(&mut self) {
        self.change_folder = None;
        self.change_folder_target = None;
        self.mode = UiMode::Table;
    }

    /// Inserts a bracketed paste into the folder input. A pasted newline becomes
    /// a space instead of an Enter key, so a paste can never commit the change.
    pub(crate) fn paste_into_change_folder(&mut self, text: &str) {
        let outcome = {
            let Some(state) = self.change_folder.as_mut() else {
                return;
            };
            if state.focus != ChangeFolderFocus::Input {
                return;
            }
            let outcome = state.input.insert_paste(text);
            state.after_edit();
            outcome
        };
        self.note_paste_outcome(outcome);
    }

    /// Validates the typed folder and enqueues [`crate::ui::effect::AppEffect::ChangeSessionFolder`].
    ///
    /// The folder must already exist: this dialog only re-points a session, so
    /// creating a project folder belongs to New Session, not here.
    pub(crate) fn confirm_change_folder(&mut self) {
        let Some(state) = self.change_folder.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        let raw = state.input.value.trim().to_string();
        if raw.is_empty() {
            state.error = Some("Enter a folder path".to_string());
            return;
        }
        let path = if is_bare_project_name(&raw) {
            crate::config::projects_dir().join(&raw)
        } else {
            resolve_input_path(&raw)
        };
        let folder = match std::fs::canonicalize(&path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                state.error = Some("Path is not a directory".to_string());
                return;
            }
            Err(err) => {
                state.error = Some(format!("Cannot open path: {err}"));
                return;
            }
        };
        let Some(idx) = self
            .change_folder_target
            .filter(|&idx| self.sessions.get(idx).is_some())
        else {
            self.status_msg = Some("No session selected".to_string());
            self.cancel_change_folder();
            return;
        };
        self.pending_effect =
            Some(crate::ui::effect::AppEffect::ChangeSessionFolder { idx, folder });
    }

    pub fn on_key_change_folder(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(state) = self.change_folder.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => self.cancel_change_folder(),
            KeyCode::Tab | KeyCode::BackTab => state.toggle_focus(),
            KeyCode::Enter => match state.focus {
                ChangeFolderFocus::Input => {
                    if state.cursor.is_some() {
                        state.apply_cursor_row();
                    } else {
                        self.confirm_change_folder();
                    }
                }
                ChangeFolderFocus::Buttons => {
                    if state.ok_focused {
                        self.confirm_change_folder();
                    } else {
                        self.cancel_change_folder();
                    }
                }
            },
            KeyCode::Down if state.focus == ChangeFolderFocus::Input => state.move_cursor(1),
            KeyCode::Up if state.focus == ChangeFolderFocus::Input => state.move_cursor(-1),
            KeyCode::Left | KeyCode::Right if state.focus == ChangeFolderFocus::Buttons => {
                state.ok_focused = !state.ok_focused;
            }
            _ if state.focus == ChangeFolderFocus::Input => {
                let edited = match key.code {
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        state.input.insert_char(c);
                        true
                    }
                    KeyCode::Backspace => {
                        state.input.backspace();
                        true
                    }
                    KeyCode::Delete => {
                        state.input.delete();
                        true
                    }
                    KeyCode::Left => {
                        state.input.move_left();
                        false
                    }
                    KeyCode::Right => {
                        state.input.move_right();
                        false
                    }
                    KeyCode::Home => {
                        state.input.home();
                        false
                    }
                    KeyCode::End => {
                        state.input.end();
                        false
                    }
                    _ => false,
                };
                if edited {
                    state.after_edit();
                }
            }
            _ => {}
        }
    }
}

// ---- Render ----

pub(crate) fn draw_change_folder_modal(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let Some(state) = app.change_folder.as_ref() else {
        return;
    };
    let (agent, id) = app
        .change_folder_target
        .and_then(|idx| app.sessions.get(idx))
        .map(|s| (s.agent.label().to_string(), s.id.clone()))
        .unwrap_or_else(|| ("?".to_string(), "?".to_string()));

    let area = centered_fixed_rect(86, 20, f.area());
    let block = modal_block(" Change Folder ", th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Agent / ID row
            Constraint::Length(1), // Spacer
            Constraint::Length(3), // Input box
            Constraint::Length(1), // Error or hint
            Constraint::Min(3),    // Folder pick list
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Buttons
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("[{}] {}", agent, id),
            Style::default().fg(th.dim),
        ))),
        rows[0],
    );
    f.render_widget(Paragraph::new(""), rows[1]);

    let input_focused = state.focus == ChangeFolderFocus::Input;
    let (border_type, style) = if input_focused {
        (
            BorderType::Thick,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )
    } else {
        (BorderType::Plain, Style::default().fg(th.dim))
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(style)
        .padding(Padding::horizontal(1));
    let input_inner = input_block.inner(rows[2]);
    f.render_widget(input_block, rows[2]);
    let (visible, cursor_x) = input_view(&state.input, input_inner.width as usize);
    let value_style = if input_focused && state.input.select_all {
        Style::default().fg(th.selection_fg).bg(th.selection_bg)
    } else {
        Style::default()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(visible, value_style))),
        input_inner,
    );
    if input_focused && state.cursor.is_none() {
        f.set_cursor_position((input_inner.x.saturating_add(cursor_x), input_inner.y));
    }

    let note = match state.error.as_deref() {
        Some(err) => Span::styled(err.to_string(), Style::default().fg(th.error)),
        None => Span::styled(
            "files are not moved · only where this session opens next".to_string(),
            th.soft_dim(),
        ),
    };
    f.render_widget(Paragraph::new(Line::from(note)), rows[3]);

    // Pick list: scrolled so the highlighted row stays visible.
    let list_area = rows[4];
    let height = list_area.height as usize;
    let start = match state.cursor {
        Some(c) if c >= height => c + 1 - height,
        _ => 0,
    };
    let lines: Vec<Line> = state
        .ordered
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .filter_map(|(row, &folder_i)| {
            let path = state.folders.get(folder_i)?;
            let text = truncate_w(&path.to_string_lossy(), list_area.width as usize);
            let style = if state.cursor == Some(row) {
                Style::default().fg(th.selection_fg).bg(th.selection_bg)
            } else if row < state.match_count {
                Style::default()
            } else {
                th.soft_dim()
            };
            Some(Line::from(Span::styled(text, style)))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list_area);

    let buttons_focused = state.focus == ChangeFolderFocus::Buttons;
    let (focused_style, unfocused) = button_styles(th);
    let (ok_style, cancel_style) = if !buttons_focused {
        (unfocused, unfocused)
    } else if state.ok_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("    OK    ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[6],
    );
}
