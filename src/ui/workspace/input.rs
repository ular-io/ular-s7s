//! Workspace key handling and mutations: the Session screen's workspace pane
//! (opening a scope, `[NEW WORKSPACE]`, deletion) and the edit dialog it opens
//! (a draft name, include/exclude words, folder search and toggles, saved
//! only by its Save button).

use super::state::{WorkspaceDialog, FIRST_FOLDER_ROW, SEARCH_ROW};
use crate::ui::{App, Focus, Screen, UiMode};
use crate::workspaces::{Workspace, WorkspaceChange, WorkspaceScope, WorkspaceStore};
use std::path::PathBuf;

impl App {
    /// Shows the workspace pane on the Session screen, focused, with its
    /// cursor on the open workspace.
    pub(crate) fn open_workspace_pane(&mut self) {
        if self.screen != Screen::Session {
            self.switch_screen(Screen::Session);
        }
        self.focus = Focus::Workspaces;
        self.workspace.new_row = false;
    }

    /// Hides the workspace pane and hands focus to the session table.
    fn close_workspace_pane(&mut self) {
        self.focus = Focus::Table;
        self.workspace.new_row = false;
    }

    /// Cursor rows: All, stored workspaces, Unassigned, New Workspace.
    pub(crate) fn workspace_pane_cursor(&self) -> usize {
        match &self.workspaces.active {
            WorkspaceScope::Unassigned => self.workspaces.workspaces.len() + 1,
            WorkspaceScope::Workspace(_) => self.workspaces.active_index().map_or(0, |i| i + 1),
            WorkspaceScope::All if self.workspace.new_row => self.workspaces.workspaces.len() + 2,
            WorkspaceScope::All => 0,
        }
    }

    /// Name of the open workspace, or `None` while "All" is open.
    pub(crate) fn active_workspace_name(&self) -> Option<&str> {
        if self.workspaces.active == WorkspaceScope::Unassigned {
            return Some(crate::workspaces::UNASSIGNED_WORKSPACE_NAME);
        }
        self.workspaces.active_workspace().map(|w| w.name.as_str())
    }

    pub(crate) fn invalidate_workspace_membership(&mut self) {
        *self.workspace.membership.get_mut() = None;
    }

    /// Shared by the session list and the folder filter's counts.
    pub(crate) fn retain_workspace_scope(&self, indices: &mut Vec<usize>) {
        if self.workspaces.active == WorkspaceScope::Unassigned {
            let mut cache = self.workspace.membership.borrow_mut();
            let membership = cache.get_or_insert_with(|| {
                crate::workspaces::membership(&self.sessions, &self.workspaces.workspaces)
            });
            indices.retain(|&idx| !membership[idx]);
        } else if let Some(ws) = self.workspaces.active_workspace() {
            indices.retain(|&idx| ws.matches(&self.sessions[idx]));
        }
    }

    /// Applies `changes` onto the current file (`WorkspaceStore::commit`), so
    /// workspaces saved by another running s7s are kept. Returns false and
    /// reports in the status bar when the file was not written; the in-memory
    /// state is the caller's to keep or roll back. `None` path = unit tests.
    pub(crate) fn persist_workspace_changes(&mut self, changes: &[WorkspaceChange]) -> bool {
        let Some(path) = self.workspaces_path.as_ref() else {
            return true;
        };
        match WorkspaceStore::commit(path, changes) {
            Ok(()) => true,
            Err(err) => {
                self.status_msg = Some(format!("Workspaces not saved: {err}"));
                false
            }
        }
    }

    fn opened_change(&self) -> WorkspaceChange {
        WorkspaceChange::Opened(self.workspaces.active.clone())
    }

    /// Rebuilds the edit dialog's folder rows (after a session rescan): the
    /// draft's selected folders first, then every other session folder, each
    /// group by latest activity. A selected folder with no session sorts last
    /// in its group. A cursor on a folder row stays on the same folder. No-op
    /// while the dialog is closed.
    pub(crate) fn refresh_workspace_folders(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let previous = dialog.cursor_folder().cloned();
        let selected = &dialog.draft.folders;
        let (mut rows, others): (Vec<PathBuf>, Vec<PathBuf>) =
            crate::ui::cwds_by_latest(&self.sessions)
                .into_iter()
                .partition(|path| selected.contains(path));
        let mut gone: Vec<PathBuf> = selected
            .iter()
            .filter(|f| !rows.contains(f))
            .cloned()
            .collect();
        gone.sort();
        rows.extend(gone);
        rows.extend(others);
        dialog.folders = rows;
        dialog.folder_counts = crate::ui::cwd_counts(&self.sessions);
        dialog.rebuild_visible();

        if let Some(prev) = previous {
            let pos = dialog.visible_folders().position(|f| *f == prev);
            if let Some(pos) = pos {
                dialog.cursor = FIRST_FOLDER_ROW + pos;
            }
        }
        dialog.cursor = dialog.cursor.min(dialog.rows() - 1);
        self.update_workspace_matching();
    }

    /// Opens the workspace at `idx` (`None` = "All"): the scope every session
    /// list shows from now on. Session selection restarts at the top.
    pub(crate) fn set_active_workspace(&mut self, idx: Option<usize>) {
        let scope = idx
            .and_then(|i| self.workspaces.workspaces.get(i))
            .map(|ws| ws.id.clone())
            .into();
        self.set_workspace_scope(scope);
    }

    pub(crate) fn set_workspace_scope(&mut self, scope: WorkspaceScope) {
        let before = self.workspaces.active.clone();
        self.workspaces.active = scope;
        self.workspaces.validate_active();
        if self.workspaces.active != WorkspaceScope::All {
            self.workspace.new_row = false;
        }
        if self.workspaces.active != before {
            self.persist_workspace_changes(&[self.opened_change()]);
            self.workspace_scope_changed();
        }
    }

    /// Re-applies a changed scope (open workspace, or the store it lives in):
    /// the session list restarts at the top.
    pub(crate) fn workspace_scope_changed(&mut self) {
        self.selected = 0;
        self.recompute();
    }

    /// Palette `Open Workspace <name>` / `Close Workspace`: opens the scope and
    /// shows it on the Session screen with the table focused.
    pub(crate) fn open_workspace_from_palette(&mut self, idx: Option<usize>) {
        self.set_active_workspace(idx);
        self.show_workspace_from_palette();
    }

    pub(crate) fn open_unassigned_workspace_from_palette(&mut self) {
        self.set_workspace_scope(WorkspaceScope::Unassigned);
        self.show_workspace_from_palette();
    }

    fn show_workspace_from_palette(&mut self) {
        self.switch_screen(Screen::Session);
        self.focus = Focus::Table;
        self.status_msg = Some(match self.active_workspace_name() {
            Some(name) => format!("Workspace opened: {name}"),
            None => "Workspace closed: all sessions".to_string(),
        });
    }

    /// Handles keys in the Session screen's workspace pane (`Focus::Workspaces`).
    /// The cursor opens the row's workspace; Enter edits it in the edit dialog
    /// (or adds one on `[NEW WORKSPACE]`); →/Esc close the pane and ← goes on
    /// to the Profile screen.
    pub(crate) fn on_key_workspace_pane(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let is_quit_key =
            matches!(key.code, KeyCode::Char('q')) || (key.code == KeyCode::Char('c') && ctrl);
        if is_quit_key {
            self.arm_quit();
            return;
        }
        self.quit_armed = false;
        self.status_msg = None;
        match key.code {
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.open_quick_command(),
            KeyCode::Char('!') => self.open_quick_terminal(),
            KeyCode::Char('/') => {
                self.mode = UiMode::Keyword;
                self.keyword_cursor = self.filter.keyword.len();
            }
            KeyCode::Char('w') if ctrl => self.open_workspace_palette(),
            KeyCode::Char('u') if ctrl => {
                self.pending_effect = Some(crate::ui::effect::AppEffect::RefreshAll);
            }
            // Same as Enter on `[NEW WORKSPACE]`, from any row; the cursor and
            // scope stay put until the new workspace is saved.
            KeyCode::Char('+') => self.open_new_workspace_dialog(),
            KeyCode::Left | KeyCode::Char('h') => {
                self.close_workspace_pane();
                self.switch_screen(Screen::Profile);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Esc => self.close_workspace_pane(),
            KeyCode::Up | KeyCode::Char('k') => self.workspace_pane_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.workspace_pane_move(1),
            KeyCode::Home | KeyCode::Char('g') => self.workspace_pane_move(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.workspace_pane_move(isize::MAX),
            KeyCode::Enter if self.workspace.new_row => self.open_new_workspace_dialog(),
            KeyCode::Enter => self.open_workspace_dialog(),
            KeyCode::Char('d') if ctrl => self.open_workspace_delete(),
            KeyCode::Delete => self.open_workspace_delete(),
            _ => {}
        }
    }

    /// Moves the workspace pane cursor, opening the row's workspace.
    /// `isize::MIN`/`MAX` jump to "All"/`[NEW WORKSPACE]`, which shows "All".
    fn workspace_pane_move(&mut self, delta: isize) {
        let new_row = self.workspaces.workspaces.len() + 2;
        let next = (self.workspace_pane_cursor() as isize)
            .saturating_add(delta)
            .clamp(0, new_row as isize) as usize;
        self.workspace.new_row = next == new_row;
        if next == new_row - 1 {
            self.set_workspace_scope(WorkspaceScope::Unassigned);
        } else {
            let idx = if self.workspace.new_row {
                None
            } else {
                next.checked_sub(1)
            };
            self.set_active_workspace(idx);
        }
    }

    /// Enter edits a stored workspace. All and Unassigned have nothing to edit.
    fn open_workspace_dialog(&mut self) {
        let Some(ws) = self.workspaces.active_workspace().cloned() else {
            return;
        };
        self.start_workspace_dialog(ws, false);
    }

    /// Enter on `[NEW WORKSPACE]` or `+`: the dialog for a workspace that
    /// exists only once saved, its suggested name selected.
    fn open_new_workspace_dialog(&mut self) {
        let ws = Workspace::new(self.workspaces.new_id(), self.workspaces.next_new_name());
        self.start_workspace_dialog(ws, true);
    }

    fn start_workspace_dialog(&mut self, draft: Workspace, created: bool) {
        self.status_msg = None;
        self.workspace.dialog = Some(WorkspaceDialog::new(draft, created));
        self.mode = UiMode::WorkspaceEdit;
        self.refresh_workspace_folders();
    }

    /// Recounts the sessions the dialog's draft matches.
    fn update_workspace_matching(&mut self) {
        if let Some(dialog) = self.workspace.dialog.as_mut() {
            dialog.matching = self
                .sessions
                .iter()
                .filter(|s| dialog.draft.matches(s))
                .count();
        }
    }

    /// Handles keys in the workspace edit dialog. Text rows (the fields and
    /// the folder search) take typing directly; ↑/↓ move one row, Tab/BackTab
    /// move between groups (each field, search, folders, buttons); space or
    /// Enter toggles a folder, and ←/`h` on a folder returns to the field last
    /// edited; Enter on a field moves on, and only the Save button saves. Esc
    /// clears a folder query first, then cancels.
    pub fn on_key_workspace_dialog(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        let was_on_search = dialog.cursor_on_search();
        let on_text_row = !dialog.on_buttons && dialog.cursor < FIRST_FOLDER_ROW;
        match key.code {
            KeyCode::Esc if was_on_search && !dialog.folder_query.value.is_empty() => {
                dialog.folder_query = crate::ui::TextInput::new(String::new());
                self.workspace_dialog_text_changed();
            }
            KeyCode::Esc => return self.cancel_workspace_dialog(),
            KeyCode::Tab => self.workspace_dialog_tab(1),
            KeyCode::BackTab => self.workspace_dialog_tab(-1),
            KeyCode::Up => self.workspace_dialog_move(-1),
            KeyCode::Down => self.workspace_dialog_move(1),
            _ if dialog.on_buttons => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                    dialog.save_focused = !dialog.save_focused
                }
                KeyCode::Enter if dialog.save_focused => return self.save_workspace_dialog(),
                KeyCode::Enter => return self.cancel_workspace_dialog(),
                _ => {}
            },
            KeyCode::Enter if was_on_search => {
                // Jump to the first match so space can toggle it right away.
                if !dialog.visible.is_empty() {
                    dialog.cursor = FIRST_FOLDER_ROW;
                }
            }
            // A text input never submits the form: Enter moves to the next row.
            KeyCode::Enter if on_text_row => self.workspace_dialog_move(1),
            _ if on_text_row => {
                let Some(input) = dialog.cursor_input() else {
                    return;
                };
                let edited = match key.code {
                    KeyCode::Char(c)
                        if !key.modifiers.intersects(
                            KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                        ) =>
                    {
                        input.insert_char(c);
                        true
                    }
                    KeyCode::Backspace => {
                        input.backspace();
                        true
                    }
                    KeyCode::Delete => {
                        input.delete();
                        true
                    }
                    KeyCode::Left => {
                        input.move_left();
                        false
                    }
                    KeyCode::Right => {
                        input.move_right();
                        false
                    }
                    KeyCode::Home => {
                        input.home();
                        false
                    }
                    KeyCode::End => {
                        input.end();
                        false
                    }
                    _ => false,
                };
                if edited {
                    self.workspace_dialog_text_changed();
                }
            }
            // Folder rows: ctrl/alt combinations have no meaning in the dialog.
            _ if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_workspace_folder(),
            // Back to the left column. Text rows keep ←/→ for the text cursor,
            // so only a folder row switches columns.
            KeyCode::Left | KeyCode::Char('h') => dialog.cursor = dialog.last_field,
            KeyCode::Char('k') => self.workspace_dialog_move(-1),
            KeyCode::Char('j') => self.workspace_dialog_move(1),
            KeyCode::Home | KeyCode::Char('g') => dialog.cursor = FIRST_FOLDER_ROW,
            KeyCode::End | KeyCode::Char('G') => dialog.cursor = dialog.rows() - 1,
            _ => {}
        }
        self.update_workspace_search_selection(was_on_search);
    }

    /// Arriving on the Search row selects the whole query, so typing replaces
    /// it; leaving drops the selection.
    fn update_workspace_search_selection(&mut self, was_on_search: bool) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let on_search = dialog.cursor_on_search();
        let query = &mut dialog.folder_query;
        if on_search && !was_on_search {
            query.select_all = !query.value.is_empty();
            query.cursor = query.value.len();
        } else if !on_search {
            query.select_all = false;
        }
    }

    /// Inserts a bracketed paste into the text row under the cursor. Folder
    /// rows and the buttons own no text, so a paste there is dropped.
    pub(crate) fn paste_into_workspace_dialog(&mut self, text: &str) {
        let Some(input) = self
            .workspace
            .dialog
            .as_mut()
            .and_then(|d| d.cursor_input())
        else {
            return;
        };
        let outcome = input.insert_paste(text);
        if outcome.inserted > 0 {
            self.workspace_dialog_text_changed();
        }
        self.note_paste_outcome(outcome);
    }

    /// After an edit of the text row under the cursor: the folder search
    /// filters the rows; include/exclude words update the match count.
    fn workspace_dialog_text_changed(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        dialog.error = None;
        if dialog.cursor_on_search() {
            dialog.rebuild_visible();
            dialog.folder_scroll.set(0);
            return;
        }
        dialog.last_field = dialog.cursor;
        dialog.draft.includes = dialog.includes.value.clone();
        dialog.draft.excludes = dialog.excludes.value.clone();
        self.update_workspace_matching();
    }

    /// Moves one row through the fields, search, visible folders, and finally
    /// the button row. `isize::MIN`/`MAX` jump to the first row/the buttons.
    fn workspace_dialog_move(&mut self, delta: isize) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let buttons = dialog.rows();
        let current = if dialog.on_buttons {
            buttons
        } else {
            dialog.cursor
        };
        let next = (current as isize)
            .saturating_add(delta)
            .clamp(0, buttons as isize) as usize;
        dialog.on_buttons = next == buttons;
        if !dialog.on_buttons {
            dialog.cursor = next;
        }
    }

    /// Tab/BackTab: each field, the search, the first folder (when any is
    /// shown), then the buttons, wrapping around.
    fn workspace_dialog_tab(&mut self, delta: isize) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        // Stops as (cursor row, on buttons).
        let mut stops: Vec<(usize, bool)> = (0..=SEARCH_ROW).map(|r| (r, false)).collect();
        if !dialog.visible.is_empty() {
            stops.push((FIRST_FOLDER_ROW, false));
        }
        stops.push((dialog.cursor, true));
        let buttons = stops.len() - 1;
        let current = if dialog.on_buttons {
            buttons
        } else {
            stops[..buttons]
                .iter()
                .rposition(|&(row, _)| row <= dialog.cursor)
                .unwrap_or(0)
        };
        let next = (current as isize + delta).rem_euclid(stops.len() as isize) as usize;
        let (row, buttons) = stops[next];
        dialog.cursor = row;
        dialog.on_buttons = buttons;
    }

    /// Space/Enter on a folder row: adds/removes it from the draft.
    fn toggle_workspace_folder(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let Some(folder) = dialog.cursor_folder().cloned() else {
            return;
        };
        dialog.draft.toggle_folder(&folder);
        dialog.error = None;
        self.update_workspace_matching();
    }

    /// Save button: validates the name, writes the workspace, and closes the
    /// dialog. A new workspace is opened, so the pane cursor lands on it. A
    /// refused name or a failed write keeps the dialog open with the reason.
    fn save_workspace_dialog(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let name = dialog.name.value.trim().to_string();
        let stored = self
            .workspaces
            .workspaces
            .iter()
            .position(|w| w.id == dialog.draft.id);
        let refusal = if name.is_empty() {
            Some("Workspace name cannot be empty".to_string())
        } else if let Some(reserved) = crate::workspaces::reserved_name(&name) {
            Some(format!("The name {reserved} is reserved for a fixed scope"))
        } else if !self.workspaces.name_available(&name, stored) {
            Some(format!("A workspace named '{name}' already exists"))
        } else {
            None
        };
        if let Some(reason) = refusal {
            dialog.error = Some(reason);
            dialog.on_buttons = false;
            dialog.cursor = 0;
            return;
        }
        let mut ws = dialog.draft.clone();
        ws.name = name.clone();
        let created = dialog.created;
        let mut changes = vec![WorkspaceChange::Upsert(ws.clone())];
        if created {
            changes.push(WorkspaceChange::Opened(WorkspaceScope::Workspace(
                ws.id.clone(),
            )));
        }
        // E.g. another instance saved a workspace under this name meanwhile.
        if !self.persist_workspace_changes(&changes) {
            let reason = self.status_msg.take();
            if let Some(dialog) = self.workspace.dialog.as_mut() {
                dialog.error = reason;
            }
            return;
        }
        self.workspace.dialog = None;
        self.mode = UiMode::Table;
        let id = ws.id.clone();
        self.workspaces.upsert(ws);
        self.invalidate_workspace_membership();
        if created {
            self.workspaces.active = WorkspaceScope::Workspace(id);
            self.workspace.new_row = false;
        }
        self.workspace_scope_changed();
        self.status_msg = Some(if created {
            format!("Workspace added: {name}")
        } else {
            format!("Workspace saved: {name}")
        });
    }

    /// Cancel button or Esc: drops the draft. Nothing was saved, so the pane
    /// and the session list are as they were.
    fn cancel_workspace_dialog(&mut self) {
        self.workspace.dialog = None;
        self.mode = UiMode::Table;
    }

    fn open_workspace_delete(&mut self) {
        let Some(idx) = self.workspaces.active_index() else {
            self.status_msg = Some(if self.workspace.new_row {
                "Select a workspace to delete".to_string()
            } else if self.workspaces.active == WorkspaceScope::Unassigned {
                "The No Workspace scope cannot be deleted".to_string()
            } else {
                "The All workspace cannot be deleted".to_string()
            });
            return;
        };
        self.pending_workspace_delete = Some(idx);
        self.delete_ok_focused = false; // Cancel first, like the other delete dialogs.
        self.mode = UiMode::WorkspaceDeleteConfirm;
    }

    fn confirm_workspace_delete(&mut self) {
        self.mode = UiMode::Table;
        let Some(idx) = self.pending_workspace_delete.take() else {
            return;
        };
        if idx >= self.workspaces.workspaces.len() {
            return;
        }
        let removed = self.workspaces.workspaces.remove(idx);
        self.invalidate_workspace_membership();
        // The cursor stays on the same row: the next workspace, else the previous.
        let len = self.workspaces.workspaces.len();
        let next = if len == 0 {
            None
        } else {
            Some(idx.min(len - 1))
        };
        self.workspaces.set_active(next);
        let saved = self.persist_workspace_changes(&[
            WorkspaceChange::Remove(removed.id.clone()),
            self.opened_change(),
        ]);
        self.workspace_scope_changed();
        if saved {
            self.status_msg = Some(format!("Workspace deleted: {}", removed.name));
        }
    }

    /// Handles keys in the workspace deletion confirmation modal.
    pub fn on_key_workspace_delete_confirm(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => self.delete_ok_focused = !self.delete_ok_focused,
            KeyCode::Enter if self.delete_ok_focused => self.confirm_workspace_delete(),
            KeyCode::Enter | KeyCode::Esc => {
                self.pending_workspace_delete = None;
                self.mode = UiMode::Table;
            }
            _ => {}
        }
    }
}
