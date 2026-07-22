//! Session search screen key handling: the main table navigation (`on_key_table`),
//! the `/` keyword search prompt (`on_key_keyword`), and the pure table selection
//! and preview scroll helpers driven by those handlers.
//!
//! Extracted from `ui::mod` per the refactoring plan (R8b). These handlers keep
//! operating on the shared `App` fields (`selected`, `filtered`, `filter`,
//! `preview_scroll`, `focus`); the §8.1 `App` split is deferred. Cross-feature
//! coordination they call (`clear_all_filters`, `set_single_profile`,
//! `switch_screen`, `recompute`, the filter/rename/delete overlays) stays in
//! `ui::mod`; `App`, being declared in the ancestor `ui` module, keeps those
//! private methods reachable from this descendant module without widening.

use crate::ui::{next_char_boundary, prev_char_boundary, App, Focus, Screen, UiMode};

impl App {
    fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let mut s = self.selected as isize + delta;
        if s < 0 {
            s = 0;
        }
        if s >= len {
            s = len - 1;
        }
        self.selected = s as usize;
        self.preview_scroll = 0;
    }

    /// Handles key inputs in the main table navigation view.
    pub fn on_key_table(&mut self, key: crossterm::event::KeyEvent) {
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
        match key.code {
            KeyCode::Char('/') => {
                self.mode = UiMode::Keyword;
                self.keyword_cursor = self.filter.keyword.len();
            }
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.open_quick_command(),
            KeyCode::Char('!') => self.open_quick_terminal(),
            KeyCode::Char('a') => self.open_agent_modal(),
            KeyCode::Char('f') => self.open_folder_modal(),
            KeyCode::Char('d') | KeyCode::Delete
                if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.is_empty() =>
            {
                self.open_delete_confirm()
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_rename_modal()
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.update_sessions_and_usage();
            }
            // Contextual New Session must match BEFORE ordinary Ctrl+N: terminals
            // with the enhanced keyboard protocol report the SHIFT modifier
            // (possibly with 'N'), legacy terminals send plain Ctrl+N instead.
            KeyCode::Char('n') | KeyCode::Char('N')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let idx = self.filtered.get(self.selected).copied();
                self.open_new_session_modal_for_session(idx, true);
            }
            KeyCode::Char('n')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let idx = self.filtered.get(self.selected).copied();
                self.open_new_session_modal_for_session(idx, false);
            }
            KeyCode::Char(c @ '1'..='5') => self.set_single_profile(c as usize - '1' as usize),
            KeyCode::Char('0') => self.clear_all_filters(),
            // ←/→ (h/l): Moves focus between the left table and the right preview column.
            // Pressing → again while preview is focused enters the session details screen.
            // Pressing ← while the table (session list) is focused moves to the profile list screen.
            KeyCode::Left | KeyCode::Char('h') => {
                if self.focus == Focus::Preview {
                    self.focus = Focus::Table;
                } else {
                    self.switch_screen(Screen::Profile);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.focus == Focus::Table {
                    self.focus = Focus::Preview;
                } else {
                    self.open_session_detail();
                }
            }
            // Arrow keys: Scrolls preview contents if preview is focused, otherwise changes table row selection.
            KeyCode::Up | KeyCode::Char('k') => {
                if self.focus == Focus::Preview {
                    self.scroll_preview(-1);
                } else {
                    self.move_selection(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.focus == Focus::Preview {
                    self.scroll_preview(1);
                } else {
                    self.move_selection(1);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if self.focus == Focus::Preview {
                    self.preview_scroll = 0;
                } else {
                    self.selected = 0;
                    self.preview_scroll = 0;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.focus == Focus::Preview {
                    self.preview_scroll = self.preview_max_scroll.get();
                } else {
                    self.selected = self.filtered.len().saturating_sub(1);
                    self.preview_scroll = 0;
                }
            }
            KeyCode::PageUp => self.scroll_preview(-10),
            KeyCode::PageDown => self.scroll_preview(10),
            KeyCode::Enter => {
                if let Some(&idx) = self.filtered.get(self.selected) {
                    self.request_resume(idx);
                }
            }
            // Esc: Reverts back to table focus if the preview panel is currently focused.
            // Otherwise, resets/clears active search keywords and filters (does not exit the application).
            KeyCode::Esc if self.focus == Focus::Preview => {
                self.focus = Focus::Table;
            }
            KeyCode::Esc => {
                if self.filter.is_active() {
                    self.clear_all_filters();
                } else {
                    self.status_msg = Some("Press q or ctrl+c twice to quit".to_string());
                }
            }
            _ => {}
        }
    }

    /// Scrolls the preview panel by `delta` lines (clamped to bounds). The upper scroll limit is computed during
    /// the last render frame based on actual lines and viewport height, resolving to 0 if all contents fit.
    fn scroll_preview(&mut self, delta: isize) {
        let max = self.preview_max_scroll.get();
        let cur = self.preview_scroll as isize;
        let next = (cur + delta).clamp(0, max as isize);
        self.preview_scroll = next as u16;
    }

    /// Handles key inputs in Keyword search mode.
    pub fn on_key_keyword(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        // Defensively clamp cursor within string bounds.
        self.keyword_cursor = self.keyword_cursor.min(self.filter.keyword.len());
        match key.code {
            KeyCode::Char(c) => {
                self.filter.keyword.insert(self.keyword_cursor, c);
                self.keyword_cursor += c.len_utf8();
                self.recompute();
            }
            KeyCode::Backspace => {
                if self.keyword_cursor > 0 {
                    let prev = prev_char_boundary(&self.filter.keyword, self.keyword_cursor);
                    self.filter.keyword.drain(prev..self.keyword_cursor);
                    self.keyword_cursor = prev;
                    self.recompute();
                }
            }
            KeyCode::Delete => {
                if self.keyword_cursor < self.filter.keyword.len() {
                    let next = next_char_boundary(&self.filter.keyword, self.keyword_cursor);
                    self.filter.keyword.drain(self.keyword_cursor..next);
                    self.recompute();
                }
            }
            KeyCode::Left => {
                self.keyword_cursor = prev_char_boundary(&self.filter.keyword, self.keyword_cursor);
            }
            KeyCode::Right => {
                self.keyword_cursor = next_char_boundary(&self.filter.keyword, self.keyword_cursor);
            }
            KeyCode::Home => self.keyword_cursor = 0,
            KeyCode::End => self.keyword_cursor = self.filter.keyword.len(),
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            // Tab: Exits text input mode while keeping the search term, focusing directly on the desired panel.
            KeyCode::Tab => {
                self.mode = UiMode::Table;
                self.focus = Focus::Preview;
            }
            KeyCode::BackTab => {
                self.mode = UiMode::Table;
                self.focus = Focus::Table;
            }
            // Enter: Commits the input keyword. Esc: Cancels search, clearing the active keyword.
            KeyCode::Enter => {
                self.mode = UiMode::Table;
                self.focus = Focus::Table;
            }
            KeyCode::Esc => {
                self.filter.keyword.clear();
                self.keyword_cursor = 0;
                self.recompute();
                self.mode = UiMode::Table;
                self.focus = Focus::Table;
            }
            _ => {}
        }
    }
}
