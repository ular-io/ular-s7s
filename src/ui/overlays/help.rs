//! The `?` keyboard-shortcuts help screen.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R9). The
//! file owns the per-screen shortcut catalogs, the `App` open/close and key
//! handling, and the full-screen help rendering. The help screen has no state
//! struct; it only toggles `UiMode::Help`. The screen-specific `SHORTCUTS_*`
//! hotkey grids stay in `ui::render` with the header (`draw_header`) that owns
//! them.

use crate::theme::Theme;
use crate::ui::components::text::pad_w;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

// ---- Shortcut catalogs ----

const HELP_GLOBAL: &[(&str, &str)] = &[
    ("?", "Open/close help"),
    ("esc/q", "Close help or clear current state"),
    ("q / ctrl+c", "Quit from table mode (press twice)"),
    (":", "Quick command palette"),
    ("!", "Terminal command in session folder"),
    ("ctrl+u", "Update sessions and usage"),
    ("1..5", "Filter by numbered profile"),
    ("0", "Clear profile filter"),
];

const HELP_SESSION: &[(&str, &str)] = &[
    ("enter", "Resume selected session"),
    ("ctrl+n", "New session (profile/folder dialog)"),
    (
        "ctrl+shift+n",
        "New session with context (fallback: `:` palette)",
    ),
    ("/", "Keyword search"),
    (".", "Expand/collapse preview (when focused)"),
    ("c", "Copy session info / all user turns (by focus)"),
    ("a", "Select agent filter"),
    ("f", "Select folder filter"),
    ("g/home", "Go to top"),
    ("G/end", "Go to bottom"),
    ("pageup", "Scroll preview up"),
    ("pagedown", "Scroll preview down"),
    ("ctrl+r", "Rename session"),
    ("ctrl+d/del", "Delete session"),
];

const HELP_DETAIL: &[(&str, &str)] = &[
    ("enter", "Resume session"),
    ("ctrl+n", "New session (profile/folder dialog)"),
    (
        "ctrl+shift+n",
        "New session with context (fallback: `:` palette)",
    ),
    (".", "Prompt: expand turn / Work: show tools & full length"),
    ("c", "Copy selected turn / work & answer (by focus)"),
    ("pageup/pagedown", "Scroll work panel"),
    ("g/home", "First question / scroll top"),
    ("G/end", "Last question / scroll bottom"),
    ("ctrl+r", "Rename session"),
    ("ctrl+d/del", "Delete session"),
    ("←/h", "Back to session list"),
];

const HELP_PROFILE: &[(&str, &str)] = &[
    ("ctrl+n", "New session (profile/folder dialog)"),
    ("1..5", "Insert profile at shortcut position"),
    ("space", "Toggle profile shortcut at end"),
    ("+", "Add profile"),
    ("ctrl+e", "Edit profile"),
    ("ctrl+d", "Delete profile"),
    ("g/home", "Go to top"),
    ("G/end", "Go to bottom"),
    ("→/l", "Return to session list"),
];

// ---- Input ----

impl App {
    pub(crate) fn open_help(&mut self) {
        self.mode = UiMode::Help;
        self.status_msg = None;
        self.quit_armed = false;
    }

    fn close_help(&mut self) {
        self.mode = UiMode::Table;
    }

    /// Handles key inputs in the global help view. Dismissing returns to table mode on the active screen.
    pub fn on_key_help(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.close_help(),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.close_help();
            }
            // Quick Command window can be opened directly from the help screen (closes the help view).
            KeyCode::Char(':') => self.open_quick_command(),
            // Terminal mode keeps help open (with a status message) if no target folder is available.
            KeyCode::Char('!') => self.open_quick_terminal(),
            _ => {}
        }
    }
}

// ---- Render ----

/// `?` Help screen modal. Temporary overlay, excluded from main Screen rotation loops.
pub(crate) fn draw_help(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let area = f.area();
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(th.base_style()), area);

    let title = " Help - Keyboard Shortcuts ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(th.accent).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            title,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    let col1 = help_lines(
        &[("GLOBAL", HELP_GLOBAL), ("SESSION LIST", HELP_SESSION)],
        th,
    );
    let col2 = help_lines(
        &[
            ("SESSION DETAIL", HELP_DETAIL),
            ("PROFILE LIST", HELP_PROFILE),
        ],
        th,
    );
    for (area, lines) in [(cols[0], col1), (cols[1], col2)] {
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("<esc>", th.key_style()),
            Span::styled(" Back  ", th.soft_dim()),
            Span::styled("<?>", th.key_style()),
            Span::styled(" Close help", th.soft_dim()),
        ]))
        .alignment(Alignment::Center),
        rows[1],
    );
}

fn help_lines(sections: &[(&str, &[(&str, &str)])], th: &Theme) -> Vec<Line<'static>> {
    // Key column width: tracks maximum width among all keys rendered.
    // Pads to build two left-aligned columns (table structure) separating keys from their labels.
    let key_w = sections
        .iter()
        .flat_map(|(_, items)| items.iter())
        .map(|(key, _)| format!("<{key}>").width())
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    for (si, (title, items)) in sections.iter().enumerate() {
        if si > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            (*title).to_string(),
            Style::default().fg(th.success).add_modifier(Modifier::BOLD),
        )));
        for (key, action) in *items {
            lines.push(Line::from(vec![
                Span::styled(pad_w(&format!("<{key}>"), key_w), th.key_style()),
                Span::raw("  "),
                Span::styled((*action).to_string(), th.soft_dim()),
            ]));
        }
    }
    lines
}
