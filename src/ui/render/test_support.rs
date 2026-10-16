//! Shared fixtures for the full-frame render tests.

use crate::ui::TextInput;
use chrono::TimeZone;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Color;
use ratatui::{backend::TestBackend, Terminal};

/// Constructs a mock App on the session table screen with one 2-turn session.
pub(super) fn session_app() -> crate::ui::App {
    use crate::model::{Agent, Session};
    use std::path::PathBuf;
    let timestamp = chrono::Local
        .with_ymd_and_hms(2026, 7, 23, 14, 5, 6)
        .single()
        .expect("valid local time")
        .timestamp_millis();
    crate::ui::App::new(
        crate::config::Config::load(),
        crate::profile::ProfileStore {
            profiles: Vec::new(),
        },
        vec![Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: "session-1".to_string(),
            source_path: None,
            cwd: PathBuf::from("/tmp"),
            folder: "tmp".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["first question".to_string(), "second question".to_string()],
            user_turn_timestamps_ms: vec![Some(timestamp), Some(timestamp)],
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: Some("demo".to_string()),
            title_fixed: false,
            context_source: None,
        }],
        "1 sessions".to_string(),
    )
}

/// Constructs a mock App initialized down to the details view with a 2-turn session via Right arrow key inputs.
pub(super) fn detail_app() -> crate::ui::App {
    let mut app = session_app();
    let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    app.on_key_table(right);
    app.on_key_table(right);
    app
}

/// Foreground of the first cell of `needle` on the pane border row `y`.
pub(super) fn title_fg(terminal: &Terminal<TestBackend>, y: u16, needle: &str) -> Color {
    let buffer = terminal.backend().buffer();
    let row: Vec<&str> = (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    let x = (0..row.len())
        .find(|&x| row[x..].concat().starts_with(needle))
        .unwrap_or_else(|| panic!("{needle:?} not on the top border"));
    buffer[(x as u16, y)].fg
}

pub(super) fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = *buffer.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

pub(super) fn rendered_text(app: &crate::ui::App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, app))
        .expect("draw");
    buffer_text(&terminal)
}

/// Locates the buffer cell where `needle` starts. Assumes single-width symbols
/// (test data is ASCII plus single-width bullets), so char offset == cell x.
pub(super) fn find_cell(terminal: &Terminal<TestBackend>, needle: &str) -> (u16, u16) {
    let buffer = terminal.backend().buffer();
    let area = *buffer.area();
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buffer[(x, y)].symbol());
        }
        if let Some(pos) = row.find(needle) {
            return (row[..pos].chars().count() as u16, y);
        }
    }
    panic!("needle not found in buffer: {needle}");
}

/// Table header row (the line carrying `FOLDER`) at a given terminal width.
pub(super) fn session_table_header(width: u16) -> String {
    rendered_text(&session_app(), width, 30)
        .lines()
        .find(|line| line.contains("FOLDER"))
        .expect("session table header")
        .to_string()
}

/// Constructs a mock App focused on the preview panel (right panel) using the provided user turns.
pub(super) fn preview_app(turns: Vec<String>) -> crate::ui::App {
    use crate::model::{Agent, Session};
    use std::path::PathBuf;
    let mut app = crate::ui::App::new(
        crate::config::Config::load(),
        crate::profile::ProfileStore {
            profiles: Vec::new(),
        },
        vec![Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: "s1".to_string(),
            source_path: None,
            cwd: PathBuf::from("/tmp"),
            folder: "tmp".to_string(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: turns,
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: Some("demo".to_string()),
            title_fixed: false,
            context_source: None,
        }],
        "1 sessions".to_string(),
    );
    app.on_key_table(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)); // Preview focus
    app
}

pub(super) fn render_at(app: &crate::ui::App, w: u16, h: u16) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| crate::ui::render::draw(f, app)).unwrap();
}

/// Mock App with `n` sessions in distinct folders and the folder modal opened.
pub(super) fn folder_modal_app(n: usize) -> crate::ui::App {
    use crate::model::{Agent, Session};
    use std::path::PathBuf;
    let sessions: Vec<Session> = (0..n)
        .map(|i| Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: format!("session-{i}"),
            source_path: None,
            cwd: PathBuf::from(format!("/tmp/folder{i:02}")),
            folder: format!("folder{i:02}"),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["question".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
            context_source: None,
        })
        .collect();
    let mut app = crate::ui::App::new(
        crate::config::Config::load(),
        crate::profile::ProfileStore {
            profiles: Vec::new(),
        },
        sessions,
        format!("{n} sessions"),
    );
    app.on_key_table(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    app
}

pub(super) fn new_session_state(
    context: Option<crate::ui::SessionContextRef>,
) -> crate::ui::NewSessionState {
    crate::ui::NewSessionState {
        profile_idx: 0,
        focus: crate::ui::NewSessionFocus::Buttons,
        dropdown_open: false,
        profile_cursor: 0,
        model_options: Vec::new(),
        model_idx: 0,
        model_cursor: 0,
        input: TextInput {
            value: String::new(),
            cursor: 0,
            select_all: false,
        },
        folders: Vec::new(),
        folder_counts: Default::default(),
        ordered: Vec::new(),
        match_count: 0,
        folder_cursor: None,
        ok_focused: true,
        error: None,
        context,
    }
}
