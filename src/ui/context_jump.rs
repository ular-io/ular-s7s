//! Context-source navigation for sessions started via "New Session with Context".
//!
//! `ctrl+o` moves to the session the focused one was launched from — the same
//! source the `● Context Source` block renders — and `ctrl+b` walks back along
//! the jumps already made. Only `ctrl+o` jumps enter the return stack: a general
//! navigation history would leave the user unable to predict what `ctrl+b` undoes.
//!
//! Two rules make the pair behave predictably:
//!
//! - **The target is always reachable.** The source is frequently in another
//!   agent/folder/profile (context sessions are explicitly allowed to cross all
//!   three), so an active filter would otherwise hide it. When the target is not
//!   in `filtered`, the filters are cleared and the status bar says so.
//! - **Returning restores the filter it left.** Because the jump may have cleared
//!   the filters, the origin's `Filter` is captured with it; the origin was
//!   selected under that filter, so restoring it can never hide the origin.

use crate::filter::Filter;
use crate::model::{Agent, Session};
use crate::ui::{App, Screen};

/// Upper bound on the return stack. Jump chains are short in practice; the cap
/// only keeps a long-running process from growing the stack without limit.
const RETURN_STACK_CAP: usize = 32;

/// One `ctrl+o` origin: where the jump started, and the filter state it started
/// under. Sessions are keyed by identity rather than index because a rescan
/// (`App::refresh_sessions`) rebuilds `sessions` and invalidates every index.
#[derive(Debug, Clone)]
pub(crate) struct JumpOrigin {
    agent: Agent,
    id: String,
    filter: Filter,
}

impl App {
    /// Index into `sessions` of `session`'s context source, matched by agent + id
    /// — the same rule `render::context_source_lines` resolves by, so what the
    /// Context Source block shows and what `ctrl+o` can reach never diverge.
    /// `None` when the session has no source, or the source was deleted / belongs
    /// to a profile that is not scanned.
    pub(crate) fn context_source_index(&self, session: &Session) -> Option<usize> {
        let src = session.context_source.as_ref()?;
        self.sessions
            .iter()
            .position(|c| c.agent == src.agent && c.id == src.id)
    }

    /// Session the active screen operates on: the list cursor on Session, the
    /// detail target on Detail. The Profile screen has no focused session.
    fn focused_session_index(&self) -> Option<usize> {
        match self.screen {
            Screen::Session => self.filtered.get(self.selected).copied(),
            Screen::Detail => self.detail.as_ref().map(|d| d.session_idx),
            Screen::Profile => None,
        }
    }

    /// Whether `ctrl+o` has a reachable target (also gates the palette entry).
    pub(crate) fn can_jump_to_context_source(&self) -> bool {
        self.focused_session_index()
            .and_then(|idx| self.sessions.get(idx))
            .and_then(|s| self.context_source_index(s))
            .is_some()
    }

    /// Whether `ctrl+b` has an origin to return to (also gates the palette entry).
    pub(crate) fn can_return_to_jump_origin(&self) -> bool {
        !self.context_jump_origins.is_empty()
    }

    /// `ctrl+o`: moves to the focused session's context source, pushing the
    /// current position onto the return stack. Repeating it walks further up the
    /// chain, since every session carries its own source.
    pub(crate) fn jump_to_context_source(&mut self) {
        let Some(origin_idx) = self.focused_session_index() else {
            self.status_msg = Some("Select a session first".to_string());
            return;
        };
        let Some(origin) = self.sessions.get(origin_idx) else {
            self.status_msg = Some("Select a session first".to_string());
            return;
        };
        if origin.context_source.is_none() {
            self.status_msg = Some("This session has no context source".to_string());
            return;
        }
        let Some(target) = self.context_source_index(origin) else {
            self.status_msg =
                Some("Context source is unavailable (deleted or unscanned profile)".to_string());
            return;
        };
        let entry = JumpOrigin {
            agent: origin.agent,
            id: origin.id.clone(),
            filter: self.filter.clone(),
        };

        self.context_jump_origins.push(entry);
        if self.context_jump_origins.len() > RETURN_STACK_CAP {
            self.context_jump_origins.remove(0);
        }
        let cleared = self.focus_session(target);
        self.status_msg = Some(if cleared {
            "Moved to context source (filters cleared)".to_string()
        } else {
            "Moved to context source".to_string()
        });
    }

    /// `ctrl+b`: returns to the origin of the most recent `ctrl+o` jump, restoring
    /// the filter that was active there. Origins whose session is gone (deleted,
    /// or dropped by a rescan) are skipped rather than failing the whole return.
    pub(crate) fn return_to_jump_origin(&mut self) {
        while let Some(origin) = self.context_jump_origins.pop() {
            let Some(idx) = self
                .sessions
                .iter()
                .position(|s| s.agent == origin.agent && s.id == origin.id)
            else {
                continue;
            };
            self.filter = origin.filter;
            self.keyword_cursor = self.filter.keyword.len();
            self.recompute();
            let cleared = self.focus_session(idx);
            self.status_msg = Some(if cleared {
                "Returned to previous session (filters cleared)".to_string()
            } else {
                "Returned to previous session".to_string()
            });
            return;
        }
        self.status_msg = Some("No previous session to return to".to_string());
    }

    /// Moves the list cursor onto `idx`, clearing the filters first when they hide
    /// it (the default filter matches every session, so the target is always
    /// reachable afterwards). On the Detail screen the detail view is reopened on
    /// the new session. Returns whether the filters had to be cleared.
    fn focus_session(&mut self, idx: usize) -> bool {
        let cleared = !self.filtered.contains(&idx);
        if cleared {
            self.filter = Filter::default();
            self.keyword_cursor = 0;
            self.recompute();
        }
        let Some(pos) = self.filtered.iter().position(|&i| i == idx) else {
            return cleared;
        };
        self.selected = pos;
        self.preview_scroll = 0;
        self.preview_expanded = false;
        if self.screen == Screen::Detail {
            // A source with no parsable turns keeps the current detail open (the
            // list cursor still moved); `open_session_detail` reports that itself.
            self.open_session_detail();
        }
        cleared
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::test_support::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn ctrl(c: char) -> crossterm::event::KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_o_walks_up_the_context_chain() {
        let mut app = app_with_context_chain();
        assert_eq!(app.current().unwrap().id, "leaf");

        app.on_key_table(ctrl('o'));
        assert_eq!(app.current().unwrap().id, "middle");

        // Every session carries its own source, so repeating the key keeps going up.
        app.on_key_table(ctrl('o'));
        assert_eq!(app.current().unwrap().id, "root");

        // The chain root has no source: the cursor stays put.
        app.on_key_table(ctrl('o'));
        assert_eq!(app.current().unwrap().id, "root");
        assert_eq!(
            app.status_msg.as_deref(),
            Some("This session has no context source")
        );
    }

    #[test]
    fn ctrl_o_clears_filters_that_hide_the_source() {
        let mut app = app_with_context_chain();
        app.filter.folders.insert("leaf".to_string());
        app.recompute();
        assert_eq!(app.filtered.len(), 1);

        app.on_key_table(ctrl('o'));

        assert_eq!(app.current().unwrap().id, "middle");
        assert!(app.filter.folders.is_empty());
        assert_eq!(app.filtered.len(), 3);
        assert_eq!(
            app.status_msg.as_deref(),
            Some("Moved to context source (filters cleared)")
        );
    }

    #[test]
    fn ctrl_o_reports_an_unavailable_source() {
        let mut app = app_with_context_chain();
        // Source deleted, or living in a profile that is not scanned.
        app.sessions[0].context_source.as_mut().unwrap().id = "ghost".to_string();

        app.on_key_table(ctrl('o'));

        assert_eq!(app.current().unwrap().id, "leaf");
        assert_eq!(
            app.status_msg.as_deref(),
            Some("Context source is unavailable (deleted or unscanned profile)")
        );
        assert!(!app.can_return_to_jump_origin());
    }

    #[test]
    fn ctrl_b_returns_to_the_origin_and_restores_its_filter() {
        let mut app = app_with_context_chain();
        app.filter.folders.insert("leaf".to_string());
        app.recompute();

        app.on_key_table(ctrl('o'));
        assert_eq!(app.current().unwrap().id, "middle");

        app.on_key_table(ctrl('b'));

        assert_eq!(app.current().unwrap().id, "leaf");
        // The jump cleared the folder filter; returning puts it back.
        assert!(app.filter.folders.contains("leaf"));
        assert_eq!(app.filtered.len(), 1);
        assert_eq!(
            app.status_msg.as_deref(),
            Some("Returned to previous session")
        );
        assert!(!app.can_return_to_jump_origin());
    }

    #[test]
    fn ctrl_b_skips_an_origin_whose_session_is_gone() {
        let mut app = app_with_context_chain();
        app.on_key_table(ctrl('o'));
        app.on_key_table(ctrl('o'));
        assert_eq!(app.current().unwrap().id, "root");

        // "middle" (the most recent origin) is deleted while the stack still names it.
        app.sessions.retain(|s| s.id != "middle");
        app.recompute();

        app.on_key_table(ctrl('b'));

        assert_eq!(app.current().unwrap().id, "leaf");
        assert!(!app.can_return_to_jump_origin());
    }

    #[test]
    fn ctrl_b_reports_an_empty_return_stack() {
        let mut app = app_with_context_chain();

        app.on_key_table(ctrl('b'));

        assert_eq!(app.current().unwrap().id, "leaf");
        assert_eq!(
            app.status_msg.as_deref(),
            Some("No previous session to return to")
        );
    }

    #[test]
    fn ctrl_o_reopens_the_detail_screen_on_the_source() {
        let mut app = app_with_context_chain();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().session_idx, 0);

        app.on_key_detail(ctrl('o'));

        let detail = app.detail.as_ref().expect("detail stays open");
        assert_eq!(app.sessions[detail.session_idx].id, "middle");
        // The list cursor follows, so leaving the detail screen lands on the source.
        assert_eq!(app.current().unwrap().id, "middle");

        app.on_key_detail(ctrl('b'));
        let detail = app.detail.as_ref().expect("detail stays open");
        assert_eq!(app.sessions[detail.session_idx].id, "leaf");
    }
}
