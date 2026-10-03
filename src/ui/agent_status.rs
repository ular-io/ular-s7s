//! Scheduling and application of live-session status sweeps (Claude, agy).
//!
//! A sweep runs `claude agents --json` once per Claude profile on a worker
//! (`crate::agent_status`). It starts at launch, with every session rescan
//! (handover return, mutations, `ctrl+u`), and every [`AGENT_STATUS_INTERVAL`]
//! while the TUI is idle. Results replace the profile's previous map; a failed
//! query clears it so a stale marker never outlives its source.

use crate::agent_status::{LiveEntry, LiveStatus};
use crate::model::{Agent, Session};
use crate::ui::{App, MessageKind};
use std::time::{Duration, Instant};

/// Period of the idle-time sweep.
pub(crate) const AGENT_STATUS_INTERVAL: Duration = Duration::from_secs(30);

impl App {
    /// Starts a sweep now (unless one is running) and restarts the periodic
    /// clock. Unit tests never spawn the CLI; they inject receivers instead.
    pub(crate) fn request_agent_status(&mut self) {
        self.agent_status_due = Instant::now() + AGENT_STATUS_INTERVAL;
        if cfg!(test) {
            return;
        }
        self.background
            .spawn_agent_status(self.profiles.profiles.clone());
    }

    /// Starts the periodic sweep once it is due.
    pub(crate) fn tick_agent_status(&mut self) {
        if Instant::now() >= self.agent_status_due {
            self.request_agent_status();
        }
    }

    /// Time left until the periodic sweep, bounding the idle event wait.
    pub(crate) fn agent_status_wait(&self) -> Duration {
        self.agent_status_due
            .saturating_duration_since(Instant::now())
    }

    /// Applies finished sweep results. Returns true when a marker changed.
    pub(crate) fn poll_agent_status(&mut self) -> bool {
        let mut changed = false;
        for (profile_id, result) in self.background.drain_agent_status() {
            let next = result.unwrap_or_default();
            // An empty map is stored as an absent entry.
            let prev_empty = !self.agent_status.contains_key(&profile_id);
            if next.is_empty() {
                changed |= !prev_empty;
                self.agent_status.remove(&profile_id);
            } else if self.agent_status.get(&profile_id) != Some(&next) {
                changed = true;
                self.agent_status.insert(profile_id, next);
            }
        }
        changed
    }

    /// Live holder of `session` from the last sweep: Claude Code's daemon or
    /// another terminal. Codex sessions are never marked.
    fn session_live_entry(&self, session: &Session) -> Option<LiveEntry> {
        if session.agent == Agent::Codex {
            return None;
        }
        self.agent_status
            .get(&session.profile_id)?
            .get(&session.id.to_ascii_lowercase())
            .cloned()
    }

    /// Live status of `session`: held by Claude Code's daemon or open in another terminal.
    pub(crate) fn session_live_status(&self, session: &Session) -> Option<LiveStatus> {
        self.session_live_entry(session).map(|e| e.status)
    }

    /// Holder check right before an action that must not race a live process.
    /// Queries the session's profile now, so a sweep up to 30 s old cannot let
    /// it through, and refreshes that profile's markers. A failed query falls
    /// back to the last sweep. Unit tests read the injected map only.
    fn live_entry_now(&mut self, idx: usize) -> Option<LiveEntry> {
        let session = self.sessions.get(idx)?;
        if session.agent == Agent::Codex {
            return None;
        }
        #[cfg(not(test))]
        {
            let profile_id = session.profile_id.clone();
            let fresh = self
                .profiles
                .find(&profile_id)
                .and_then(crate::agent_status::query);
            match fresh {
                Some(map) if map.is_empty() => {
                    self.agent_status.remove(&profile_id);
                }
                Some(map) => {
                    self.agent_status.insert(profile_id, map);
                }
                None => {}
            }
        }
        self.session_live_entry(&self.sessions[idx])
    }

    /// Diverts `action` on a session a live Claude process holds. Returns true
    /// when it did: resuming a background session asks whether to attach;
    /// everything else explains the block. Opening a session another terminal
    /// holds would make two processes write one transcript, a flagged resume of
    /// a background session exits 1, and deleting or renaming either races the
    /// process still writing it.
    pub(crate) fn divert_live_session(&mut self, idx: usize, action: LiveAction) -> bool {
        let Some(entry) = self.live_entry_now(idx) else {
            return false;
        };
        let background = entry.status != LiveStatus::Open;
        if let (LiveAction::Resume, true, Some(job)) = (action, background, entry.job.clone()) {
            self.open_attach_confirm(idx, job, entry.status);
            return true;
        }
        let hint = if action == LiveAction::Resume && !background {
            "Continue it there, or exit it there and open it again.".to_string()
        } else {
            entry.release_hint()
        };
        let lines = vec![entry.status.holder_sentence(), String::new(), hint];
        self.show_message(action.title(), lines, MessageKind::Warn);
        true
    }
}

/// Session actions that must not run while a live Claude process holds the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveAction {
    Resume,
    Delete,
    Rename,
}

impl LiveAction {
    fn title(self) -> &'static str {
        match self {
            LiveAction::Resume => " Cannot Resume ",
            LiveAction::Delete => " Cannot Delete ",
            LiveAction::Rename => " Cannot Rename ",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_status::StatusMap;
    use crate::ui::test_support::empty_app;

    fn session(profile: &str, id: &str, agent: Agent) -> Session {
        let mut s = crate::ui::test_support::app_with_session().sessions[0].clone();
        s.id = id.to_string();
        s.profile_id = profile.to_string();
        s.agent = agent;
        s
    }

    fn map(entries: &[(&str, LiveStatus)]) -> StatusMap {
        entries
            .iter()
            .map(|(id, st)| {
                let job = (*st != LiveStatus::Open).then(|| id.chars().take(8).collect());
                (id.to_string(), LiveEntry { status: *st, job })
            })
            .collect()
    }

    #[test]
    fn results_mark_only_matching_claude_sessions() {
        let mut app = empty_app();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(("p1".into(), Some(map(&[("abc", LiveStatus::Done)]))))
            .unwrap();
        drop(tx);
        app.background.set_agent_status_receiver(rx);
        assert!(app.poll_agent_status());
        assert!(!app.background_in_flight());

        assert_eq!(
            app.session_live_status(&session("p1", "ABC", Agent::Claude)),
            Some(LiveStatus::Done)
        );
        assert_eq!(
            app.session_live_status(&session("p2", "abc", Agent::Claude)),
            None
        );
        assert_eq!(
            app.session_live_status(&session("p1", "abc", Agent::Antigravity)),
            Some(LiveStatus::Done)
        );
        assert_eq!(
            app.session_live_status(&session("p1", "abc", Agent::Codex)),
            None
        );
    }

    #[test]
    fn failed_or_empty_sweep_clears_the_profile() {
        let mut app = empty_app();
        app.agent_status
            .insert("p1".into(), map(&[("abc", LiveStatus::Working)]));
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(("p1".into(), None)).unwrap();
        drop(tx);
        app.background.set_agent_status_receiver(rx);
        assert!(app.poll_agent_status());
        assert!(app.agent_status.is_empty());

        // An unchanged result does not request a redraw.
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(("p1".into(), Some(StatusMap::new()))).unwrap();
        drop(tx);
        app.background.set_agent_status_receiver(rx);
        assert!(!app.poll_agent_status());
    }

    fn table_rows(app: &App) -> (Vec<String>, ratatui::buffer::Buffer) {
        use ratatui::{backend::TestBackend, Terminal};
        let mut terminal = Terminal::new(TestBackend::new(100, 8)).unwrap();
        terminal
            .draw(|f| crate::ui::session::render::draw_table(f, app, f.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let rows = (0..8)
            .map(|y| (0..100).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        (rows, buffer)
    }

    #[test]
    fn table_title_carries_bold_status_marker_after_the_bookmark() {
        use ratatui::style::Modifier;
        let mut app = crate::ui::test_support::app_with_session();
        app.sessions[0].agent = Agent::Claude;
        app.sessions[0].profile_id = "p1".into();
        app.agent_status
            .insert("p1".into(), map(&[("session-1", LiveStatus::NeedsInput)]));
        app.recompute();

        let (rows, _) = table_rows(&app);
        assert!(rows.iter().any(|r| r.contains("Ⓑ  hello")), "{rows:#?}");

        app.bookmarks.set(&app.sessions[0], true);
        let (rows, buffer) = table_rows(&app);
        assert!(rows.iter().any(|r| r.contains("♥ Ⓑ  hello")), "{rows:#?}");
        let (x, y) = (0..8)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == "Ⓑ")
            .expect("status cell");
        assert!(buffer[(x, y)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(x + 1, y)].symbol(), " ");
        assert_eq!(buffer[(x + 2, y)].symbol(), " ");
    }

    #[test]
    fn session_metadata_shows_the_status_marker() {
        let mut app = crate::ui::test_support::app_with_session();
        app.sessions[0].agent = Agent::Claude;
        app.sessions[0].profile_id = "p1".into();
        app.agent_status
            .insert("p1".into(), map(&[("session-1", LiveStatus::Done)]));
        let marks = app.title_marks(&app.sessions[0]);
        let th = crate::theme::default_theme();
        let text: String =
            crate::ui::render::session_meta_lines(&app.sessions[0], marks, false, 60, &th, false)
                .iter()
                .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
                .collect();
        assert!(text.contains("Ⓑ  hello"), "{text}");
    }

    #[test]
    fn prompt_metadata_explains_each_marker_under_name() {
        let mut app = crate::ui::test_support::app_with_session();
        app.sessions[0].agent = Agent::Claude;
        app.sessions[0].profile_id = "p1".into();
        app.agent_status
            .insert("p1".into(), map(&[("session-1", LiveStatus::Working)]));
        app.bookmarks.set(&app.sessions[0], true);
        let marks = app.title_marks(&app.sessions[0]);
        let th = crate::theme::default_theme();
        let rows = |legend: bool| -> Vec<String> {
            crate::ui::render::session_meta_lines(&app.sessions[0], marks, legend, 80, &th, false)
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect())
                .collect()
        };

        let shown = rows(true);
        let name = shown
            .iter()
            .position(|r| r.starts_with("- Name: "))
            .unwrap();
        assert_eq!(shown[name + 1], "- ♥ : Bookmarked");
        assert_eq!(shown[name + 2], "- Ⓑ : Claude background session · working");
        assert!(shown[name + 3].starts_with("- Created at: "));

        let hidden = rows(false);
        assert!(hidden.iter().all(|r| !r.starts_with("- ♥")), "{hidden:#?}");

        // No markers, no legend rows.
        let plain = crate::ui::render::session_meta_lines(
            &app.sessions[0],
            Default::default(),
            true,
            80,
            &th,
            false,
        );
        assert_eq!(plain.len(), hidden.len());
    }

    fn live_app(status: LiveStatus) -> App {
        let mut app = crate::ui::test_support::app_with_session();
        app.sessions[0].agent = Agent::Claude;
        app.sessions[0].profile_id = "p1".into();
        app.agent_status
            .insert("p1".into(), map(&[("session-1", status)]));
        app.recompute();
        app
    }

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crate::ui::test_support::key(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn resume_of_a_session_open_elsewhere_is_blocked() {
        let mut app = live_app(LiveStatus::Open);
        app.request_resume(0);
        assert_eq!(app.resume_request, None);
        assert_eq!(app.mode, crate::ui::UiMode::Message);
        let msg = app.message.as_ref().unwrap();
        assert_eq!(msg.title, " Cannot Resume ");
        assert!(msg.lines[0].contains("open in another terminal"));
    }

    #[test]
    fn resume_of_a_background_session_asks_to_attach_with_cancel_first() {
        use crossterm::event::KeyCode;
        let mut app = live_app(LiveStatus::Done);
        app.request_resume(0);
        assert_eq!(app.resume_request, None);
        assert_eq!(app.mode, crate::ui::UiMode::AttachConfirm);
        assert!(!app.attach_ok_focused);

        // Enter on the default button cancels.
        app.on_key_attach_confirm(key(KeyCode::Enter));
        assert_eq!(app.mode, crate::ui::UiMode::Table);
        assert_eq!(app.attach_request, None);

        app.request_resume(0);
        app.on_key_attach_confirm(key(KeyCode::Left));
        app.on_key_attach_confirm(key(KeyCode::Enter));
        assert_eq!(
            app.attach_request,
            Some(crate::ui::AttachRequest {
                idx: 0,
                job: "session-".into()
            })
        );
        assert_eq!(app.resume_request, None);
    }

    #[test]
    fn attach_dialog_names_the_state_and_both_buttons() {
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = live_app(LiveStatus::NeedsInput);
        app.request_resume(0);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| crate::ui::render::draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..30)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Attach Background Session"), "{text}");
        assert!(text.contains("(needs input)"), "{text}");
        assert!(text.contains("Attach") && text.contains("Cancel"));
    }

    #[test]
    fn delete_and_rename_are_blocked_for_live_sessions() {
        for status in [LiveStatus::Open, LiveStatus::Working] {
            let mut app = live_app(status);
            app.open_delete_confirm_at(0);
            assert_eq!(app.pending_delete, None);
            assert_eq!(app.message.as_ref().unwrap().title, " Cannot Delete ");

            let mut app = live_app(status);
            app.open_rename_modal_at(0);
            assert!(app.rename_modal.is_none());
            assert_eq!(app.message.as_ref().unwrap().title, " Cannot Rename ");
        }
    }

    #[test]
    fn sessions_without_a_live_holder_open_as_before() {
        let mut app = crate::ui::test_support::app_with_session();
        app.sessions[0].agent = Agent::Claude;
        app.request_resume(0);
        assert_eq!(app.resume_request, Some(0));
        app.open_delete_confirm_at(0);
        assert_eq!(app.pending_delete, Some(0));
    }

    #[test]
    fn request_restarts_the_periodic_clock() {
        let mut app = empty_app();
        app.agent_status_due = Instant::now();
        app.tick_agent_status();
        assert!(app.agent_status_wait() > AGENT_STATUS_INTERVAL - Duration::from_secs(1));
    }
}
