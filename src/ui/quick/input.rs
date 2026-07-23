//! Quick Command key handling and command execution: opening the palette /
//! terminal window, mode switching, list navigation, terminal history recall,
//! and dispatching a selected command to the same handler its hotkey invokes.

use super::registry::{build_items, CommandId};
use super::state::{build_term_items, save_history, save_terminal_history, QuickMode, QuickState};
use super::VIEWPORT;
use crate::ui::{App, Screen, TerminalKind, TerminalRequest, UiMode};

/// Maximum number of recently executed keys preserved in the history file.
const MAX_HISTORY: usize = 20;

impl App {
    /// `:`: Opens the Quick Command palette (empty input defaults to recently executed + all commands).
    pub(crate) fn open_quick_command(&mut self) {
        self.quick = Some(QuickState::new(QuickMode::Palette, None));
        self.quick_recompute();
        self.mode = UiMode::QuickCommand;
        self.status_msg = None;
    }

    /// `!`: Opens the Quick Command window in terminal mode targeting the selected
    /// session's folder. Shows a status message instead if no target folder is available.
    pub(crate) fn open_quick_terminal(&mut self) {
        let Some(folder) = self.terminal_target() else {
            self.status_msg =
                Some("Terminal command needs a session with an existing folder".to_string());
            return;
        };
        self.quick = Some(QuickState::new(QuickMode::Terminal, Some(folder)));
        self.quick_term_recompute();
        self.mode = UiMode::QuickCommand;
        self.status_msg = None;
    }

    /// Resolves the folder terminal commands run in: the selected session's cwd (Session
    /// list) or the detail target's cwd (Detail). None if no session is selected, the
    /// session has no folder, or the folder no longer exists on disk.
    fn terminal_target(&self) -> Option<std::path::PathBuf> {
        let idx = match self.screen {
            Screen::Session => self.filtered.get(self.selected).copied(),
            Screen::Detail => self.detail.as_ref().map(|d| d.session_idx),
            Screen::Profile => None,
        }?;
        let cwd = &self.sessions.get(idx)?.cwd;
        (!cwd.as_os_str().is_empty() && cwd.is_dir()).then(|| cwd.clone())
    }

    /// Switches the open window to terminal mode (empty-input `!`), re-resolving the target
    /// folder. Keeps palette mode with a status message if no target folder is available.
    fn quick_switch_terminal(&mut self) {
        let Some(folder) = self.terminal_target() else {
            self.status_msg =
                Some("Terminal command needs a session with an existing folder".to_string());
            return;
        };
        if let Some(state) = self.quick.as_mut() {
            state.mode = QuickMode::Terminal;
            state.term_folder = Some(folder);
            state.term_typed.clear();
        }
        self.quick_term_recompute();
    }

    /// Switches the open window to palette mode (empty-input `:`).
    fn quick_switch_palette(&mut self) {
        if let Some(state) = self.quick.as_mut() {
            state.mode = QuickMode::Palette;
        }
        self.quick_recompute();
    }

    /// Re-evaluates list matches and resets cursor / scroll positions on input changes.
    fn quick_recompute(&mut self) {
        let Some(state) = self.quick.as_ref() else {
            return;
        };
        let query = state.input.value.clone();
        let items = build_items(&query, &self.quick_history, |id| self.quick_enabled(id));
        if let Some(state) = self.quick.as_mut() {
            state.items = items;
            state.cursor = 0;
            state.scroll = 0;
        }
    }

    /// Re-evaluates the terminal history list and resets selection / scroll positions.
    fn quick_term_recompute(&mut self) {
        let Some(query) = self.quick.as_ref().map(|s| s.term_typed.clone()) else {
            return;
        };
        let items = build_term_items(&query, &self.terminal_history);
        if let Some(state) = self.quick.as_mut() {
            state.term_items = items;
            state.term_selected = None;
            state.scroll = 0;
        }
    }

    /// Evaluates if the command is enabled on the active screen (drives dim rendering and sorting).
    fn quick_enabled(&self, id: CommandId) -> bool {
        use CommandId::*;
        match id {
            OpenSessionWindow => self.screen != Screen::Session,
            OpenProfileWindow => self.screen != Screen::Profile,
            // Contextual New Session needs a focused source session (Session/Detail only).
            ResumeSession | NewSessionWithContext | RenameSession | DeleteSession => {
                match self.screen {
                    Screen::Session => self.filtered.get(self.selected).is_some(),
                    Screen::Detail => self.detail.is_some(),
                    Screen::Profile => false,
                }
            }
            TerminalCommand => self.terminal_target().is_some(),
            NewSession => true,
            CreateProfile => self.screen == Screen::Profile,
            EditProfile | DeleteProfile => {
                self.screen == Screen::Profile && !self.profiles.profiles.is_empty()
            }
            ToggleProfileShortcut => {
                self.screen == Screen::Profile && !self.profiles.profiles.is_empty()
            }
            SearchSessions | FilterByAgent | FilterByFolder => self.screen == Screen::Session,
            ClearFilters => self.screen == Screen::Session && self.filter.is_active(),
            ToggleToolLogs => self.screen == Screen::Detail,
            RefreshAll | EditConfig | ChangeTheme | OpenHelp | ExitApp => true,
        }
    }

    /// Handles key inputs in the Quick Command modal, dispatching by window mode.
    /// `:`/`!` on an EMPTY input switches the window mode (both are ordinary typeable
    /// characters otherwise).
    pub fn on_key_quick(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some((mode, input_empty)) = self
            .quick
            .as_ref()
            .map(|s| (s.mode, s.input.value.is_empty()))
        else {
            self.mode = UiMode::Table;
            return;
        };
        if let KeyCode::Char(c @ (':' | '!')) = key.code {
            if input_empty && !key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    ':' if mode == QuickMode::Terminal => self.quick_switch_palette(),
                    '!' if mode == QuickMode::Palette => self.quick_switch_terminal(),
                    // Same-mode key: consumed as a mode key, not inserted as text.
                    _ => {}
                }
                return;
            }
        }
        match mode {
            QuickMode::Palette => self.on_key_quick_palette(key),
            QuickMode::Terminal => self.on_key_quick_terminal(key),
        }
    }

    /// Palette mode keys: Esc closes, Enter runs selection, Up/Down moves cursor,
    /// and editing keys update the search text input.
    fn on_key_quick_palette(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(state) = self.quick.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.quick = None;
                self.mode = UiMode::Table;
            }
            KeyCode::Enter => {
                let Some(item) = state.items.get(state.cursor).copied() else {
                    return;
                };
                if !item.enabled {
                    return;
                }
                self.quick = None;
                self.mode = UiMode::Table;
                self.quick_record_history(item.spec().key);
                self.quick_execute(item.spec().id);
            }
            KeyCode::Up => Self::quick_move_cursor(state, -1),
            KeyCode::Down => Self::quick_move_cursor(state, 1),
            KeyCode::PageUp => Self::quick_move_cursor(state, -(VIEWPORT as isize)),
            KeyCode::PageDown => Self::quick_move_cursor(state, VIEWPORT as isize),
            KeyCode::Left => state.input.move_left(),
            KeyCode::Right => state.input.move_right(),
            KeyCode::Home => state.input.home(),
            KeyCode::End => state.input.end(),
            KeyCode::Backspace => {
                state.input.backspace();
                self.quick_recompute();
            }
            KeyCode::Delete => {
                state.input.delete();
                self.quick_recompute();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.input.insert_char(c);
                self.quick_recompute();
            }
            _ => {}
        }
    }

    /// Terminal mode keys: Esc closes, Enter runs the input content, Up/Down moves the
    /// history selection (recalling the command into the editable input), and editing
    /// keys update the input (detaching any recall and re-filtering the history list).
    fn on_key_quick_terminal(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(state) = self.quick.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.quick = None;
                self.mode = UiMode::Table;
            }
            KeyCode::Enter => {
                let command = state.input.value.trim().to_string();
                if command.is_empty() {
                    return;
                }
                let Some(cwd) = state.term_folder.clone() else {
                    return;
                };
                self.quick = None;
                self.mode = UiMode::Table;
                self.record_terminal_history(&command);
                self.terminal_request = Some(TerminalRequest {
                    cwd,
                    command,
                    kind: TerminalKind::Command,
                });
            }
            KeyCode::Up => Self::term_move_selection(state, -1),
            KeyCode::Down => Self::term_move_selection(state, 1),
            KeyCode::PageUp => Self::term_move_selection(state, -(VIEWPORT as isize)),
            KeyCode::PageDown => Self::term_move_selection(state, VIEWPORT as isize),
            KeyCode::Left => state.input.move_left(),
            KeyCode::Right => state.input.move_right(),
            KeyCode::Home => state.input.home(),
            KeyCode::End => state.input.end(),
            KeyCode::Backspace => {
                state.input.backspace();
                self.term_after_edit();
            }
            KeyCode::Delete => {
                state.input.delete();
                self.term_after_edit();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.input.insert_char(c);
                self.term_after_edit();
            }
            _ => {}
        }
    }

    /// Editing detaches any history recall: the input becomes the typed filter text again.
    fn term_after_edit(&mut self) {
        if let Some(state) = self.quick.as_mut() {
            state.term_typed = state.input.value.clone();
        }
        self.quick_term_recompute();
    }

    /// Moves the history selection, recalling the selected command into the editable input.
    /// Moving above the first row returns focus to the input line and restores the typed text.
    fn term_move_selection(state: &mut QuickState, delta: isize) {
        if state.term_items.is_empty() {
            return;
        }
        let last = state.term_items.len() as isize - 1;
        // -1 represents "no selection" (focus on the input line).
        let cur = state.term_selected.map(|i| i as isize).unwrap_or(-1);
        let next = (cur + delta).clamp(-1, last);
        state.term_selected = (next >= 0).then_some(next as usize);
        match state.term_selected {
            Some(i) => {
                state.input.value = state.term_items[i].clone();
                state.scroll = if i < state.scroll {
                    i
                } else if i >= state.scroll + VIEWPORT {
                    i + 1 - VIEWPORT
                } else {
                    state.scroll
                };
            }
            None => {
                state.input.value = state.term_typed.clone();
                state.scroll = 0;
            }
        }
        state.input.cursor = state.input.value.len();
    }

    /// Records an executed terminal command to the head of history and persists it to disk.
    fn record_terminal_history(&mut self, command: &str) {
        self.terminal_history.retain(|c| c != command);
        self.terminal_history.insert(0, command.to_string());
        self.terminal_history.truncate(MAX_HISTORY);
        save_terminal_history(&self.terminal_history);
    }

    /// Moves cursor index, shifting the viewport (max 10 rows) to keep cursor visible.
    fn quick_move_cursor(state: &mut QuickState, delta: isize) {
        if state.items.is_empty() {
            return;
        }
        let last = state.items.len() - 1;
        state.cursor = (state.cursor as isize + delta).clamp(0, last as isize) as usize;
        if state.cursor < state.scroll {
            state.scroll = state.cursor;
        } else if state.cursor >= state.scroll + VIEWPORT {
            state.scroll = state.cursor + 1 - VIEWPORT;
        }
    }

    /// Records executed key to the head of history and serializes changes to disk.
    fn quick_record_history(&mut self, key: &str) {
        self.quick_history.retain(|k| k != key);
        self.quick_history.insert(0, key.to_string());
        self.quick_history.truncate(MAX_HISTORY);
        save_history(&self.quick_history);
    }

    /// Executes the selected command, delegating to the same handler method invoked by standard hotkeys.
    fn quick_execute(&mut self, id: CommandId) {
        use CommandId::*;
        // Session index mapping back to `sessions` list from search or details views.
        let session_idx = match self.screen {
            Screen::Session => self.filtered.get(self.selected).copied(),
            Screen::Detail => self.detail.as_ref().map(|d| d.session_idx),
            Screen::Profile => None,
        };
        match id {
            OpenSessionWindow => self.switch_screen(Screen::Session),
            OpenProfileWindow => self.switch_screen(Screen::Profile),
            ResumeSession => {
                if let Some(idx) = session_idx {
                    self.request_resume(idx);
                }
            }
            NewSession => {
                if self.screen == Screen::Profile {
                    self.open_new_session_modal(self.profile_selected, None, false, None);
                } else {
                    self.open_new_session_modal_for_session(session_idx, false);
                }
            }
            // Guaranteed fallback for terminals that cannot distinguish Ctrl+Shift+N.
            NewSessionWithContext => self.open_new_session_modal_for_session(session_idx, true),
            RenameSession => {
                if let Some(idx) = session_idx {
                    self.open_rename_modal_at(idx);
                }
            }
            DeleteSession => {
                if let Some(idx) = session_idx {
                    self.open_delete_confirm_at(idx);
                }
            }
            TerminalCommand => self.open_quick_terminal(),
            CreateProfile => self.open_profile_form(None),
            EditProfile => self.open_profile_form(Some(self.profile_selected)),
            DeleteProfile => self.open_profile_delete(),
            ToggleProfileShortcut => self.toggle_selected_profile_shortcut(),
            SearchSessions => {
                self.mode = UiMode::Keyword;
                self.keyword_cursor = self.filter.keyword.len();
            }
            FilterByAgent => self.open_agent_modal(),
            FilterByFolder => self.open_folder_modal(),
            ClearFilters => self.clear_all_filters(),
            RefreshAll => self.pending_effect = Some(crate::ui::effect::AppEffect::RefreshAll),
            ToggleToolLogs => self.detail_show_tools = !self.detail_show_tools,
            EditConfig => self.request_edit_config(),
            ChangeTheme => self.open_theme_select(),
            OpenHelp => self.open_help(),
            ExitApp => self.should_quit = true,
        }
    }

    /// Opens `~/.config/s7s/config.toml` in the resolved editor via the terminal
    /// handover path. A missing or blank file is seeded with the commented template
    /// first (which also creates the config dir, so `cd` in the handover succeeds).
    fn request_edit_config(&mut self) {
        crate::config::ensure_config_template();
        let dir = crate::config::config_base_dir();
        let path = crate::config::config_file_path();
        let command = format!(
            "{} {}",
            self.cfg.editor_command(),
            crate::resume::shell_quote(&path.to_string_lossy())
        );
        self.terminal_request = Some(TerminalRequest {
            cwd: dir,
            command,
            kind: TerminalKind::EditConfig,
        });
    }
}
