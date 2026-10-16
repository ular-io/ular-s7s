//! Full-search key handling and run lifecycle: opening the prompt, starting a
//! run over the listed sessions, streaming its results into the session table
//! without moving the cursor, stopping, and leaving the mode.

use super::state::{FullSearchState, RunPhase, SearchRun};
use crate::full_search::{Query, SessionMatch};
use crate::model::Session;
use crate::ui::refresh::SessionKey;
use crate::ui::{App, Focus, Screen, TextInput, UiMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(crate) fn full_search_active(&self) -> bool {
        self.full_search.is_some()
    }

    /// Whether the full-search prompt owns input. It shares `UiMode::Keyword`
    /// with the keyword prompt; this flag tells the two apart.
    pub(crate) fn full_search_editing(&self) -> bool {
        self.mode == UiMode::Keyword && self.full_search.as_ref().is_some_and(|s| s.editing)
    }

    /// The current run's match for `session`.
    pub(crate) fn full_search_match(&self, session: &Session) -> Option<&SessionMatch> {
        let run = self.full_search.as_ref()?.run.as_ref()?;
        run.matches.get(&SessionKey::of(session))
    }

    /// The submitted query, shown in the session table title.
    pub(crate) fn full_search_query(&self) -> Option<&str> {
        let run = self.full_search.as_ref()?.run.as_ref()?;
        Some(run.query.as_str())
    }

    /// Prompt pane text for an empty full-search list.
    pub(crate) fn full_search_empty_note(&self) -> Option<&'static str> {
        let run = self.full_search.as_ref()?.run.as_ref()?;
        Some(if run.phase == RunPhase::Running {
            "No matches yet."
        } else {
            "No matching sessions."
        })
    }

    /// Narrows `filtered` to the current run's matches (none before the first
    /// query is submitted). Part of `rebuild_filtered`, after the workspace scope.
    pub(crate) fn retain_full_search_matches(&self, filtered: &mut Vec<usize>) {
        let Some(run) = self.full_search.as_ref().and_then(|s| s.run.as_ref()) else {
            return;
        };
        filtered.retain(|&i| run.matches.contains_key(&SessionKey::of(&self.sessions[i])));
    }

    /// `ctrl+/` or the palette: opens the prompt, or returns to it with the
    /// last query selected so typing replaces it.
    pub(crate) fn open_full_search(&mut self) {
        if self.screen != Screen::Session {
            self.switch_screen(Screen::Session);
        }
        if self.focus == Focus::Workspaces {
            self.focus = Focus::Table;
        }
        let origin = self.current().map(SessionKey::of);
        let state = self.full_search.get_or_insert_with(|| FullSearchState {
            input: TextInput::new(String::new()),
            editing: false,
            run: None,
            origin,
        });
        state.editing = true;
        state.input.end();
        state.input.select_all = state.run.is_some() && !state.input.value.is_empty();
        self.mode = UiMode::Keyword;
        self.status_msg = None;
    }

    /// Keys while the full-search prompt owns input.
    pub(crate) fn on_key_full_search_prompt(&mut self, key: KeyEvent) {
        let Some(state) = self.full_search.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        let input = &mut state.input;
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                input.insert_char(c)
            }
            KeyCode::Backspace => input.backspace(),
            KeyCode::Delete => input.delete(),
            KeyCode::Left => input.move_left(),
            KeyCode::Right => input.move_right(),
            KeyCode::Home => input.home(),
            KeyCode::End => input.end(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            // Back to the results of the last run, keeping its query.
            KeyCode::Tab | KeyCode::BackTab if state.run.is_some() => {
                state.editing = false;
                input.select_all = false;
                self.mode = UiMode::Table;
                self.focus = if key.code == KeyCode::Tab {
                    Focus::Preview
                } else {
                    Focus::Table
                };
            }
            KeyCode::Enter => {
                let text = input.value.trim().to_string();
                match Query::parse(&text) {
                    Some(query) => self.start_full_search(text, query),
                    None => self.exit_full_search(),
                }
            }
            KeyCode::Esc => self.exit_full_search(),
            _ => {}
        }
    }

    pub(crate) fn paste_into_full_search(&mut self, text: &str) {
        if let Some(state) = self.full_search.as_mut() {
            let outcome = state.input.insert_paste(text);
            self.note_paste_outcome(outcome);
        }
    }

    /// Starts a run over the sessions the list shows without one (the
    /// ordinary filters and the open workspace apply) and hands the session
    /// table focus at once; matches appear as they are found.
    fn start_full_search(&mut self, text: String, query: Query) {
        self.background.cancel_full_search();
        if let Some(state) = self.full_search.as_mut() {
            state.run = None;
            state.editing = false;
        }
        self.rebuild_filtered();
        let jobs: Vec<(SessionKey, Session)> = self
            .filtered
            .iter()
            .map(|&i| {
                let mut session = self.sessions[i].clone();
                // Not read by the search; dropped to keep the job list small.
                session.assistant_blob = String::new();
                (SessionKey::of(&session), session)
            })
            .collect();
        let total = jobs.len();
        if let Some(state) = self.full_search.as_mut() {
            state.run = Some(SearchRun::new(text, total));
        }
        if total > 0 {
            self.background.spawn_full_search(jobs, query);
        }
        self.mode = UiMode::Table;
        self.focus = Focus::Table;
        self.recompute();
    }

    /// `Esc` on the session table: stops a running search, keeping the matches
    /// found so far; a second `Esc` leaves the mode.
    pub(crate) fn on_full_search_escape(&mut self) {
        let run = self.full_search.as_mut().and_then(|s| s.run.as_mut());
        match run {
            Some(run) if run.phase == RunPhase::Running => {
                self.background.cancel_full_search();
                run.phase = RunPhase::Stopped;
            }
            _ => self.exit_full_search(),
        }
    }

    /// Leaves full-search mode and restores the ordinary list, keeping the
    /// cursor on the inspected session (or the one selected on entry).
    pub(crate) fn exit_full_search(&mut self) {
        self.background.cancel_full_search();
        let Some(state) = self.full_search.take() else {
            return;
        };
        let keep: Vec<SessionKey> = self
            .current()
            .map(SessionKey::of)
            .into_iter()
            .chain(state.origin)
            .collect();
        self.mode = UiMode::Table;
        self.focus = Focus::Table;
        self.recompute();
        if let Some(pos) = keep.iter().find_map(|key| self.list_position(key)) {
            self.selected = pos;
        }
    }

    /// Applies streamed results; returns whether a redraw is needed.
    pub(crate) fn poll_full_search(&mut self) -> bool {
        let Some((results, finished)) = self.background.drain_full_search() else {
            return false;
        };
        let Some(run) = self.full_search.as_mut().and_then(|s| s.run.as_mut()) else {
            return false;
        };
        let before = run.matches.len();
        let changed = finished || !results.is_empty();
        run.scanned += results.len();
        for (key, found) in results {
            if let Some(m) = found {
                run.matches.insert(key, m);
            }
        }
        if finished && run.phase == RunPhase::Running {
            run.phase = RunPhase::Done;
        }
        if run.matches.len() > before {
            self.relist_keeping_view();
        }
        changed
    }

    /// Rebuilds the list after new matches without moving the cursor or the
    /// viewport: the selected session and the top visible row are found again
    /// by identity, since rows can be inserted above both.
    fn relist_keeping_view(&mut self) {
        let selected = self.current().map(SessionKey::of);
        let offset = self.table_state.borrow().offset();
        let top = self
            .filtered
            .get(offset)
            .map(|&i| SessionKey::of(&self.sessions[i]));
        self.rebuild_filtered();
        if let Some(pos) = selected.and_then(|key| self.list_position(&key)) {
            self.selected = pos;
        }
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
        if let Some(pos) = top.and_then(|key| self.list_position(&key)) {
            *self.table_state.borrow_mut().offset_mut() = pos;
        }
    }

    fn list_position(&self, key: &SessionKey) -> Option<usize> {
        self.filtered
            .iter()
            .position(|&i| key.matches(&self.sessions[i]))
    }

    /// Detail opens on the matched turn of a session found by the current run.
    pub(crate) fn full_search_turn(&self, idx: usize) -> Option<usize> {
        self.full_search_match(&self.sessions[idx]).map(|m| m.turn)
    }
}
