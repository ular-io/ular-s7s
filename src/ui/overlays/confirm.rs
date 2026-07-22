//! Session rename and delete confirmation dialogs.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R9). The
//! file owns the rename dialog state (`RenameFocus` / `RenameModalState`), the
//! `App` open/cancel/confirm and key handling for both dialogs, and their
//! rendering. The rename state types are re-exported from `ui` so the existing
//! `crate::ui::{RenameFocus, RenameModalState}` paths stay stable. The
//! session-deletion filesystem work (`delete_session_artifacts` and its
//! helpers) stays in `ui::mod` as session manipulation, not overlay logic;
//! `confirm_delete` reaches it descendant-to-ancestor without widening.

use crate::ui::components::modal::{button_styles, modal_block, render_modal};
use crate::ui::components::text::truncate_w;
use crate::ui::render::{centered_fixed_rect, input_view};
use crate::ui::{App, TextInput, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

// ---- State ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameFocus {
    Input,
    Buttons,
}

pub struct RenameModalState {
    pub input: TextInput,
    pub focus: RenameFocus,
    pub ok_focused: bool,
}

// ---- Input ----

impl App {
    pub(crate) fn open_delete_confirm(&mut self) {
        if let Some(&idx) = self.filtered.get(self.selected) {
            self.open_delete_confirm_at(idx);
        } else {
            self.status_msg = Some("No session selected".to_string());
        }
    }

    /// Opens session deletion confirmation modal for specified sessions index (helper for details screen direct trigger).
    pub(crate) fn open_delete_confirm_at(&mut self, idx: usize) {
        self.pending_delete = Some(idx);
        self.delete_ok_focused = false; // Default focus to Cancel (safer fallback).
        self.mode = UiMode::DeleteConfirm;
        self.status_msg = None;
    }

    pub(crate) fn open_rename_modal(&mut self) {
        if let Some(&idx) = self.filtered.get(self.selected) {
            self.open_rename_modal_at(idx);
        } else {
            self.status_msg = Some("No session selected".to_string());
        }
    }

    /// Opens session rename modal for specified sessions index (helper for details screen direct trigger).
    pub(crate) fn open_rename_modal_at(&mut self, idx: usize) {
        let Some(session) = self.sessions.get(idx) else {
            self.status_msg = Some("No session selected".to_string());
            return;
        };
        self.rename_modal = Some(RenameModalState {
            input: TextInput::new(session.title()),
            focus: RenameFocus::Input,
            ok_focused: true,
        });
        self.rename_target = Some(idx);
        self.mode = UiMode::Rename;
        self.status_msg = None;
    }

    fn cancel_delete_confirm(&mut self) {
        self.pending_delete = None;
        self.mode = UiMode::Table;
    }

    fn cancel_rename(&mut self) {
        self.rename_modal = None;
        self.rename_target = None;
        self.mode = UiMode::Table;
    }

    /// Validates the rename request and enqueues the [`AppEffect::RenameSession`]
    /// effect. The actual metadata rename and rescan run at the `App` boundary
    /// (`apply_effect`), which also decides whether to close the modal (success)
    /// or keep it open with an error (failure). Pre-flight validation stays here
    /// as a pure decision so an invalid request never emits an effect.
    pub(crate) fn confirm_rename(&mut self) {
        let Some(state) = self.rename_modal.as_ref() else {
            self.mode = UiMode::Table;
            return;
        };
        // Session captured when opening modal (independent of active table cursor).
        let Some(idx) = self
            .rename_target
            .filter(|&idx| self.sessions.get(idx).is_some())
        else {
            self.status_msg = Some("No session selected".to_string());
            return;
        };

        let title = state.input.value.trim().to_string();
        if title.is_empty() {
            self.status_msg = Some("Title cannot be empty".to_string());
            return;
        }

        // Metadata paths and CLI env derive from the owning profile; never fall
        // back to the default root (wrong account store for extra profiles).
        // Reject up front so a missing profile never emits an effect.
        if self.profiles.find(&self.sessions[idx].profile_id).is_none() {
            self.status_msg =
                Some("Rename failed: session profile not found — refresh with ctrl+u".to_string());
            return;
        }

        self.pending_effect = Some(crate::ui::effect::AppEffect::RenameSession { idx, title });
    }

    /// Confirms deletion and enqueues the [`AppEffect::DeleteSession`] effect.
    /// The filesystem removal, list rebuild, and detail-screen return run at the
    /// `App` boundary (`apply_effect`); the handler only resolves the target and
    /// returns to table mode.
    fn confirm_delete(&mut self) {
        let Some(idx) = self.pending_delete.take() else {
            self.mode = UiMode::Table;
            return;
        };
        self.mode = UiMode::Table;

        if self.sessions.get(idx).is_none() {
            self.status_msg = Some("Delete target no longer exists".to_string());
            return;
        }

        self.pending_effect = Some(crate::ui::effect::AppEffect::DeleteSession { idx });
    }

    /// Handles key inputs in the session deletion confirmation modal.
    pub fn on_key_delete_confirm(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            // Tab/Arrows: Moves focus between Cancel and Delete buttons.
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.delete_ok_focused = !self.delete_ok_focused;
            }
            // Enter: Executes the action of the currently focused button.
            KeyCode::Enter => {
                if self.delete_ok_focused {
                    self.confirm_delete();
                } else {
                    self.cancel_delete_confirm();
                }
            }
            KeyCode::Esc => self.cancel_delete_confirm(),
            _ => {}
        }
    }

    pub fn on_key_rename_modal(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(m) = &mut self.rename_modal else {
            self.mode = UiMode::Table;
            return;
        };

        match key.code {
            KeyCode::Esc => self.cancel_rename(),
            KeyCode::Tab | KeyCode::Down => {
                m.focus = match m.focus {
                    RenameFocus::Input => RenameFocus::Buttons,
                    RenameFocus::Buttons => RenameFocus::Input,
                };
            }
            KeyCode::BackTab | KeyCode::Up => {
                m.focus = match m.focus {
                    RenameFocus::Input => RenameFocus::Buttons,
                    RenameFocus::Buttons => RenameFocus::Input,
                };
            }
            KeyCode::Enter => {
                if m.focus == RenameFocus::Buttons {
                    if m.ok_focused {
                        self.confirm_rename();
                    } else {
                        self.cancel_rename();
                    }
                }
            }
            _ => match m.focus {
                RenameFocus::Input => match key.code {
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                            && !key.modifiers.contains(KeyModifiers::SUPER) =>
                    {
                        m.input.insert_char(c);
                    }
                    KeyCode::Backspace => m.input.backspace(),
                    KeyCode::Delete => m.input.delete(),
                    KeyCode::Left => m.input.move_left(),
                    KeyCode::Right => m.input.move_right(),
                    KeyCode::Home => m.input.home(),
                    KeyCode::End => m.input.end(),
                    _ => {}
                },
                RenameFocus::Buttons => {
                    if matches!(
                        key.code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
                    ) {
                        m.ok_focused = !m.ok_focused;
                    }
                }
            },
        }
    }
}

// ---- Render ----

/// Deletion confirmation modal. Renders centered title + 1 space margins/padding,
/// fixed height based on content lines, and Tab-navigated centered button row.
pub(crate) fn draw_delete_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let (agent, title, cwd, id) = app
        .pending_delete
        .and_then(|idx| app.sessions.get(idx))
        .map(|s| {
            (
                s.agent.label().to_string(),
                s.title(),
                s.cwd.to_string_lossy().to_string(),
                s.id.clone(),
            )
        })
        .unwrap_or_else(|| {
            (
                "?".to_string(),
                "?".to_string(),
                "?".to_string(),
                "?".to_string(),
            )
        });

    // Content rows: title/cwd/id + spacer + notice = 5 lines; spacer + buttons = 2 lines.
    // Height calculation: borders (2) + padding (2) + 5 + 2 = 11.
    let area = centered_fixed_rect(70, 10, f.area());
    let block = modal_block(" Delete Session ", th.error).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    // Truncates text by inner width boundaries to prevent double-width characters from clipping the border.
    let prefix_w = 1 + agent.width() + 2; // "[" + agent + "] "
    let content = vec![
        Line::from(vec![
            Span::styled("[", Style::default().fg(th.dim)),
            Span::styled(
                agent,
                Style::default().fg(th.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled("] ", Style::default().fg(th.dim)),
            Span::raw(truncate_w(&title, inner_w.saturating_sub(prefix_w))),
        ]),
        Line::from(Span::styled(
            truncate_w(&format!("cwd: {}", cwd), inner_w),
            Style::default().fg(th.dim),
        )),
        Line::from(Span::styled(
            truncate_w(&format!("id: {}", id), inner_w),
            Style::default().fg(th.dim),
        )),
        Line::from(""),
        Line::from(Span::styled("This action cannot be undone.", th.soft_dim())),
    ];

    // Splits inner layout: content (Min(0)) + spacer (1 row) + buttons (1 row).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content), rows[0]);

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

/// Session rename modal dialog.
pub(crate) fn draw_rename_modal(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let Some(state) = app.rename_modal.as_ref() else {
        return;
    };
    // Target session captured when opening the modal (independent of search selection, shared by details screen).
    let (agent, id) = app
        .rename_target
        .and_then(|idx| app.sessions.get(idx))
        .map(|s| (s.agent.label().to_string(), s.id.clone()))
        .unwrap_or_else(|| ("?".to_string(), "?".to_string()));

    let area = centered_fixed_rect(72, 11, f.area());
    let block = modal_block(" Rename Session ", th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Agent / ID row
            Constraint::Length(1), // Spacer
            Constraint::Length(3), // Input box
            Constraint::Length(2), // Margin padding
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

    let input_focused = state.focus == RenameFocus::Input;
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
    f.render_widget(Paragraph::new(visible), input_inner);
    if input_focused {
        f.set_cursor_position((input_inner.x.saturating_add(cursor_x), input_inner.y));
    }

    // Buttons
    let buttons_focused = state.focus == RenameFocus::Buttons;
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
        rows[4],
    );
}
