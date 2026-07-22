//! The reusable alert message dialog (`show_message`).
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R9). The
//! file owns the dialog state (`MessageKind` / `MessageDialog`), the `App`
//! spawn/dismiss and key handling, and the rendering. `MessageKind` and
//! `MessageDialog` are re-exported from `ui` so the existing
//! `crate::ui::{MessageKind, MessageDialog}` paths stay stable. `show_message`
//! stays `pub` because any screen spawns alerts through it.

use crate::ui::components::modal::{modal_block, render_modal};
use crate::ui::components::text::wrap_w;
use crate::ui::render::centered_fixed_rect;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

// ---- State ----

/// Severity of the alert dialog, determining border and button highlight colors.
/// Info and Warn variants are currently unused, but preserved for dialog API reuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MessageKind {
    Info,
    Warn,
    Error,
}

/// State for the reusable alert message dialog.
///
/// Can be spawned anywhere via `App::show_message`, returning to the prior UI mode
/// when dismissed (via Enter / Esc / Space). Holds purely descriptive variables (title,
/// lines, severity) with no custom actions/callbacks attached.
pub struct MessageDialog {
    pub title: String,
    pub lines: Vec<String>,
    pub kind: MessageKind,
    /// Prior UI mode to restore upon dismissing the dialog.
    return_mode: UiMode,
}

// ---- Input ----

impl App {
    /// Spawns an alert message dialog. Reverts to the prior active mode on dismissal.
    /// Exposed publicly for reuse in displaying warnings or validation errors.
    pub fn show_message(
        &mut self,
        title: impl Into<String>,
        lines: Vec<String>,
        kind: MessageKind,
    ) {
        let return_mode = self.mode;
        self.message = Some(MessageDialog {
            title: title.into(),
            lines,
            kind,
            return_mode,
        });
        self.mode = UiMode::Message;
    }

    /// Dismisses the message dialog and restores the prior UI mode.
    fn dismiss_message(&mut self) {
        let return_mode = self
            .message
            .take()
            .map(|m| m.return_mode)
            .unwrap_or(UiMode::Table);
        self.mode = return_mode;
    }

    /// Handles key inputs in the alert dialog, dismissing and returning to the prior UI mode on commit keys.
    pub fn on_key_message(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char(' ') => self.dismiss_message(),
            _ => {}
        }
    }
}

// ---- Render ----

/// Generic message alert dialog. Adapts highlight borders depending on severity,
/// wraps body strings to inner widths, and draws centered confirmation "OK" button.
pub(crate) fn draw_message_modal(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let Some(m) = &app.message else {
        return;
    };
    let color = match m.kind {
        MessageKind::Info => th.accent,
        MessageKind::Warn => th.warning,
        MessageKind::Error => th.error,
    };

    let full = f.area();
    // Width calculation: longest body/title line + paddings (margins 2 + borders 2 + padding 2 = 6). Clamped to 34..80.
    let content_w = m
        .lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(m.title.width());
    let w = (content_w as u16)
        .saturating_add(6)
        .clamp(34.min(full.width), full.width.min(80));
    // Inner area width = outer width - margins (2) - borders (2) - padding (2).
    let inner_w = (w as usize).saturating_sub(6);

    // Wraps body text based on inner width constraints (handles long path strings). Empty strings remain as empty rows.
    let mut body: Vec<Line> = Vec::new();
    for line in &m.lines {
        if line.is_empty() {
            body.push(Line::from(""));
        } else {
            for wrapped in wrap_w(line, inner_w) {
                body.push(Line::from(Span::raw(wrapped)));
            }
        }
    }

    // Height calculation: borders (2) + padding (2) + body + spacer (1) + button (1).
    let h = (body.len() as u16).saturating_add(6).min(full.height);
    let area = centered_fixed_rect(w, h, full);
    let inner = render_modal(f, area, modal_block(&m.title, color), th);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    f.render_widget(Paragraph::new(body), rows[0]);

    let button = Line::from(Span::styled(
        "  OK  ",
        Style::default()
            .fg(crate::theme::contrast_fg(color))
            .bg(color)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(button).alignment(Alignment::Center), rows[1]);
}
