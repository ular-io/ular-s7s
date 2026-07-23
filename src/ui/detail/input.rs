//! Session detail key handling: opening the screen from the search preview,
//! closing back to the session list, and the in-screen navigation, focus
//! toggling, tool-log visibility toggle, and session operations (resume,
//! rename, delete, contextual New Session) shared with the main view.

use crate::ui::{App, DetailFocus, Focus, Screen, SessionDetailState, UiMode};

impl App {
    // ---- Session Details Screen ----

    /// Enters details view for the selected session (triggered via preview focus + → key).
    /// Parses the raw session file to structure turns (questions, workspace tasks, and answers).
    pub(crate) fn open_session_detail(&mut self) {
        let Some(&idx) = self.filtered.get(self.selected) else {
            self.status_msg = Some("No session selected".to_string());
            return;
        };
        let turns = crate::handoff::load_turns(&self.sessions[idx]);
        if turns.is_empty() {
            self.status_msg = Some("No turns to show for this session".to_string());
            return;
        }
        self.detail = Some(SessionDetailState {
            session_idx: idx,
            turns,
            selected: 0,
            expanded_prompt: None,
            focus: DetailFocus::Questions,
            left_scroll: std::cell::Cell::new(0),
            left_scrollable: std::cell::Cell::new(false),
            left_scroll_min: std::cell::Cell::new(0),
            left_scroll_max: std::cell::Cell::new(0),
            right_scroll: std::cell::Cell::new(0),
            right_max_scroll: std::cell::Cell::new(0),
        });
        self.screen = Screen::Detail;
        self.status_msg = None;
    }

    /// Closes details screen and returns to main search screen. Focuses the left session list
    /// table so that users can immediately resume browsing/navigating.
    pub(crate) fn close_session_detail(&mut self) {
        self.detail = None;
        self.screen = Screen::Session;
        self.mode = UiMode::Table;
        self.focus = Focus::Table;
    }

    /// Handles key inputs on the session details screen.
    ///
    /// ←/→ moves focus between the left (questions list) and right (workspace tasks / agent responses) panels.
    /// Pressing ← while focused on the questions list returns to the session search screen.
    pub fn on_key_detail(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let is_quit_key = matches!(key.code, KeyCode::Char('q'))
            || (matches!(key.code, KeyCode::Char('c'))
                && key.modifiers.contains(KeyModifiers::CONTROL));
        if is_quit_key {
            self.arm_quit();
            return;
        }

        self.quit_armed = false;
        self.status_msg = None;
        if self.detail.is_none() {
            self.close_session_detail();
            return;
        }
        match key.code {
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.open_quick_command(),
            KeyCode::Char('!') => self.open_quick_terminal(),
            // Session operations identical to the main search view: resume, rename, and delete.
            KeyCode::Enter => {
                if let Some(idx) = self.detail.as_ref().map(|d| d.session_idx) {
                    self.request_resume(idx);
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(idx) = self.detail.as_ref().map(|d| d.session_idx) {
                    self.open_rename_modal_at(idx);
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.pending_effect = Some(crate::ui::effect::AppEffect::RefreshAll);
            }
            // Contextual New Session (matched before ordinary Ctrl+N; see on_key_table).
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let idx = self.detail.as_ref().map(|d| d.session_idx);
                self.open_new_session_modal_for_session(idx, true);
            }
            KeyCode::Char('n')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let idx = self.detail.as_ref().map(|d| d.session_idx);
                self.open_new_session_modal_for_session(idx, false);
            }
            KeyCode::Char('d') | KeyCode::Delete
                if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.is_empty() =>
            {
                if let Some(idx) = self.detail.as_ref().map(|d| d.session_idx) {
                    self.open_delete_confirm_at(idx);
                }
            }
            // Focus-aware expand toggle:
            // - Prompt (Questions) panel: expand the selected turn's omitted prompt.
            // - Work & Answer panel: reveal hidden tool calls/results and lift the
            //   per-entry line caps (`detail_show_tools`).
            KeyCode::Char('.') => {
                let on_questions = self
                    .detail
                    .as_ref()
                    .is_some_and(|d| d.focus == DetailFocus::Questions);
                if on_questions {
                    if let Some(d) = self.detail.as_mut() {
                        let idx = d.selected;
                        if d.expanded_prompt == Some(idx) {
                            // Collapse the currently expanded turn.
                            d.expanded_prompt = None;
                            d.left_scrollable.set(false);
                            self.status_msg = Some(format!("Q{} prompt collapsed", idx + 1));
                        } else if crate::ui::render::preview_turn_is_truncated(&d.turns[idx].user) {
                            // Expand this turn (implicitly collapsing any other) and reset the
                            // manual scroll so it starts at the top of the turn.
                            d.expanded_prompt = Some(idx);
                            d.left_scroll.set(0);
                            self.status_msg = Some(format!("Q{} prompt expanded", idx + 1));
                        }
                        // Turns short enough to render in full are not expandable: ignore `.`.
                    }
                } else {
                    self.detail_show_tools = !self.detail_show_tools;
                    self.status_msg = Some(
                        if self.detail_show_tools {
                            "Work & Answer expanded (tools + full length)"
                        } else {
                            "Work & Answer collapsed"
                        }
                        .to_string(),
                    );
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                let on_questions = self
                    .detail
                    .as_ref()
                    .is_some_and(|d| d.focus == DetailFocus::Questions);
                if on_questions {
                    self.close_session_detail();
                } else if let Some(d) = self.detail.as_mut() {
                    d.focus = DetailFocus::Questions;
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(d) = self.detail.as_mut() {
                    d.focus = DetailFocus::Work;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.detail_nav(-1),
            KeyCode::Down | KeyCode::Char('j') => self.detail_nav(1),
            KeyCode::PageUp => {
                if let Some(d) = self.detail.as_ref() {
                    d.scroll_work(-10);
                }
            }
            KeyCode::PageDown => {
                if let Some(d) = self.detail.as_ref() {
                    d.scroll_work(10);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                let Some(d) = self.detail.as_mut() else {
                    return;
                };
                match d.focus {
                    DetailFocus::Questions => {
                        if d.selected != 0 {
                            d.selected = 0;
                            d.right_scroll.set(0);
                        }
                    }
                    DetailFocus::Work => d.right_scroll.set(0),
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                let Some(d) = self.detail.as_mut() else {
                    return;
                };
                match d.focus {
                    DetailFocus::Questions => {
                        let last = d.turns.len().saturating_sub(1);
                        if d.selected != last {
                            d.selected = last;
                            d.right_scroll.set(0);
                        }
                    }
                    DetailFocus::Work => d.right_scroll.set(d.right_max_scroll.get()),
                }
            }
            _ => {}
        }
    }

    /// Details view ↑/↓: changes selected question if focused on the left panel, scrolls workspace tasks if focused on the right panel.
    fn detail_nav(&mut self, delta: isize) {
        let Some(d) = self.detail.as_mut() else {
            return;
        };
        match d.focus {
            // While an expanded turn overflows the panel, ↑/↓ scroll it instead of
            // moving to the previous/next turn (bounds computed during render).
            DetailFocus::Questions if d.left_scrollable.get() => d.scroll_prompt(delta),
            DetailFocus::Questions => d.move_selection(delta),
            DetailFocus::Work => d.scroll_work(delta),
        }
    }
}
