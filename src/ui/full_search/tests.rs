use super::state::RunPhase;
use crate::full_search::{MatchField, SessionMatch, SnippetLine};
use crate::model::{Agent, Session};
use crate::session_context::ContextEntryKind;
use crate::ui::refresh::SessionKey;
use crate::ui::test_support::{key, test_profiles};
use crate::ui::{App, Focus, Screen, UiMode};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};

fn session(i: usize, turns: &[&str]) -> Session {
    Session {
        agent: Agent::Claude,
        profile_id: String::new(),
        id: format!("session-{i}"),
        source_path: None,
        cwd: PathBuf::from("/tmp"),
        folder: "tmp".into(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: turns.iter().map(|t| t.to_string()).collect(),
        user_turn_timestamps_ms: Vec::new(),
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: Some(format!("title {i}")),
        title_fixed: false,
        context_source: None,
    }
}

fn app_with(n: usize) -> App {
    let sessions = (0..n)
        .map(|i| session(i, &["first question", "second question"]))
        .collect();
    App::new(
        crate::config::Config::load(),
        test_profiles(),
        sessions,
        String::new(),
    )
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.on_key_keyword(key(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

/// Submits `query` with the workers replaced by a controlled channel.
fn start_controlled(
    app: &mut App,
    query: &str,
) -> Sender<crate::full_search::SearchResult<SessionKey>> {
    app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::CONTROL));
    type_text(app, query);
    app.on_key_keyword(key(KeyCode::Enter, KeyModifiers::NONE));
    let (tx, rx) = mpsc::channel();
    app.background.set_full_search_receiver(rx);
    tx
}

fn found(app: &App, i: usize, turn: usize) -> (SessionKey, Option<SessionMatch>) {
    let m = SessionMatch {
        turn,
        field: MatchField::Entry(ContextEntryKind::ToolResult),
        lines: vec![SnippetLine {
            text: "error: linker failed".into(),
            highlights: vec![(7, 13)],
        }],
        hits: 3,
    };
    (SessionKey::of(&app.sessions[i]), Some(m))
}

fn selected_id(app: &App) -> &str {
    &app.current().expect("selection").id
}

fn status(app: &App) -> String {
    let run = app.full_search.as_ref().unwrap().run.as_ref().unwrap();
    run.status(true)
}

fn screen_text(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| crate::ui::render::draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn enter_moves_focus_to_the_table_while_the_search_runs() {
    let mut app = app_with(3);
    app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::Keyword);
    assert!(app.full_search_editing());
    type_text(&mut app, "linker");
    // The query goes to the full-search prompt, not the keyword filter.
    assert!(app.filter.keyword.is_empty());
    assert_eq!(app.filtered.len(), 3);

    app.on_key_keyword(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!((app.mode, app.focus), (UiMode::Table, Focus::Table));
    assert!(app.filtered.is_empty());
    assert_eq!(
        status(&app),
        "[Searching 0/3 · 0 matches found · Esc: Stop]"
    );
    assert!(app.background_in_flight());
}

#[test]
fn legacy_ctrl_7_opens_the_prompt() {
    let mut app = app_with(1);
    app.on_key_table(key(KeyCode::Char('7'), KeyModifiers::CONTROL));
    assert!(app.full_search_editing());
}

#[test]
fn streamed_matches_keep_the_cursor_on_the_same_session() {
    let mut app = app_with(5);
    let tx = start_controlled(&mut app, "linker");
    tx.send(found(&app, 3, 0)).unwrap();
    assert!(app.poll_full_search());
    assert_eq!(selected_id(&app), "session-3");

    // Matches inserted above the cursor shift its row, not its session.
    tx.send(found(&app, 1, 0)).unwrap();
    tx.send((SessionKey::of(&app.sessions[2]), None)).unwrap();
    tx.send(found(&app, 0, 0)).unwrap();
    app.poll_full_search();
    let ids: Vec<&str> = app
        .filtered
        .iter()
        .map(|&i| app.sessions[i].id.as_str())
        .collect();
    assert_eq!(ids, ["session-0", "session-1", "session-3"]);
    assert_eq!(selected_id(&app), "session-3");
    assert_eq!(
        status(&app),
        "[Searching 4/5 · 3 matches found · Esc: Stop]"
    );

    drop(tx);
    app.poll_full_search();
    assert_eq!(
        status(&app),
        "[Done · 4/5 searched · 3 matches found · Esc: Back]"
    );
}

#[test]
fn esc_stops_then_restores_the_full_list() {
    let mut app = app_with(4);
    let tx = start_controlled(&mut app, "linker");
    tx.send(found(&app, 1, 0)).unwrap();
    tx.send(found(&app, 2, 0)).unwrap();
    app.poll_full_search();
    app.on_key_table(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(selected_id(&app), "session-2");

    app.on_key_table(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(status(&app), "[Stopped · 2 matches found · Esc: Back]");
    assert!(!app.background_in_flight());
    // Late results of the stopped run are discarded.
    let _ = tx.send(found(&app, 3, 0));
    assert!(!app.poll_full_search());
    assert_eq!(app.filtered.len(), 2);

    app.on_key_table(key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.full_search.is_none());
    assert_eq!(app.filtered.len(), 4);
    assert_eq!(selected_id(&app), "session-2");
}

#[test]
fn prompt_and_match_box_render_while_the_table_has_focus() {
    let mut app = app_with(2);
    let tx = start_controlled(&mut app, "linker");
    tx.send(found(&app, 1, 1)).unwrap();
    app.poll_full_search();
    let text = screen_text(&app);
    assert!(text.contains("Full Search"), "{text}");
    assert!(
        text.contains("[Searching 1/2 · 1 match found · Esc: Stop]"),
        "{text}"
    );
    assert!(text.contains("Session[full: linker: 1]"), "{text}");
    assert!(text.contains("Match · Q2 · Tool Result"), "{text}");
    assert!(text.contains("3 hits"), "{text}");
    assert!(text.contains("error: linker failed"), "{text}");
}

#[test]
fn detail_opens_on_the_matched_turn() {
    let mut app = app_with(2);
    let tx = start_controlled(&mut app, "linker");
    tx.send(found(&app, 0, 1)).unwrap();
    app.poll_full_search();
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);
    assert_eq!(app.detail.as_ref().unwrap().selected, 1);
    // Enter on the table still resumes the session.
    app.close_session_detail();
    app.on_key_table(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.resume_request, Some(0));
}

#[test]
fn keyword_prompt_narrows_the_matches_and_returns_to_full_search() {
    let mut app = app_with(3);
    let tx = start_controlled(&mut app, "linker");
    tx.send(found(&app, 0, 0)).unwrap();
    tx.send(found(&app, 2, 0)).unwrap();
    app.poll_full_search();
    app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::NONE));
    assert!(!app.full_search_editing());
    type_text(&mut app, "session-2");
    assert_eq!(app.filtered.len(), 1);
    assert!(screen_text(&app).contains("Search"));
    app.on_key_keyword(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(screen_text(&app).contains("Full Search"));
}

#[test]
fn reopening_selects_the_last_query_and_empty_enter_closes() {
    let mut app = app_with(1);
    let _tx = start_controlled(&mut app, "linker");
    app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::CONTROL));
    let state = app.full_search.as_ref().unwrap();
    assert!(state.editing && state.input.select_all);
    app.on_key_keyword(key(KeyCode::Backspace, KeyModifiers::NONE));
    app.on_key_keyword(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.full_search.is_none());
    assert_eq!((app.mode, app.filtered.len()), (UiMode::Table, 1));
}

#[test]
fn palette_entry_opens_the_prompt_from_another_screen() {
    let mut app = app_with(1);
    app.screen = Screen::Profile;
    app.open_full_search();
    assert_eq!((app.screen, app.mode), (Screen::Session, UiMode::Keyword));
}

#[test]
fn real_workers_search_indexed_user_turns() {
    let sessions = vec![
        session(0, &["deploy the worker"]),
        session(1, &["unrelated"]),
        session(2, &["worker pool sizing"]),
    ];
    let mut app = App::new(
        crate::config::Config::load(),
        test_profiles(),
        sessions,
        String::new(),
    );
    app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::CONTROL));
    type_text(&mut app, "WORKER");
    app.on_key_keyword(key(KeyCode::Enter, KeyModifiers::NONE));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.background_in_flight() && std::time::Instant::now() < deadline {
        app.poll_full_search();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let run = app.full_search.as_ref().unwrap().run.as_ref().unwrap();
    assert_eq!((run.phase, run.scanned), (RunPhase::Done, 3));
    let ids: Vec<&str> = app
        .filtered
        .iter()
        .map(|&i| app.sessions[i].id.as_str())
        .collect();
    assert_eq!(ids, ["session-0", "session-2"]);
    let m = app.full_search_match(&app.sessions[0]).unwrap();
    assert_eq!((m.turn, m.field), (0, MatchField::User));
}
