//! Workspace key handling and mutations: the Session screen's workspace pane
//! (opening a scope, `[NEW WORKSPACE]`, deletion), the Workspaces screen's
//! Detail pane (the in-place name/include/exclude edit, the folder search, and
//! folder toggles), and moving between the two.

use super::state::{WorkspaceEdit, WorkspaceField, FIRST_FOLDER_ROW};
use crate::ui::{App, Focus, Screen, TextInput, UiMode};
use crate::workspaces::{Workspace, WorkspaceChange, WorkspaceStore};
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

    /// Shows the Workspaces screen for the open workspace, with its Detail
    /// pane at the top. "All" has nothing to edit and never gets here.
    fn enter_workspace_screen(&mut self) {
        self.switch_screen(Screen::Workspace);
        self.workspace.detail_cursor = 0;
        self.workspace.folder_scroll.set(0);
        self.workspace.clear_folder_query();
        self.refresh_workspace_folders();
    }

    /// Esc on the Workspaces screen: back to the Session screen with the
    /// workspace pane focused on the same workspace.
    fn leave_workspace_screen(&mut self) {
        self.switch_screen(Screen::Session);
        self.focus = Focus::Workspaces;
        self.workspace.new_row = false;
    }

    /// Row of the workspace pane cursor: 0 = "All", then the stored
    /// workspaces, then `[NEW WORKSPACE]`.
    pub(crate) fn workspace_pane_cursor(&self) -> usize {
        match self.workspaces.active_index() {
            Some(i) => i + 1,
            None if self.workspace.new_row => self.workspaces.workspaces.len() + 1,
            None => 0,
        }
    }

    /// Name of the open workspace, or `None` while "All" is open.
    pub(crate) fn active_workspace_name(&self) -> Option<&str> {
        self.workspaces.active_workspace().map(|w| w.name.as_str())
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

    /// Rebuilds the Detail pane folder rows for the open workspace: its selected
    /// folders first, then every other session folder, each group by latest
    /// activity. A selected folder with no session sorts last in its group. A
    /// cursor on a folder row stays on the same folder.
    pub(crate) fn refresh_workspace_folders(&mut self) {
        let previous = self.workspace.cursor_folder().cloned();
        let selected: Vec<PathBuf> = self
            .workspaces
            .active_workspace()
            .map(|w| w.folders.clone())
            .unwrap_or_default();
        let (mut rows, others): (Vec<PathBuf>, Vec<PathBuf>) =
            crate::ui::cwds_by_latest(&self.sessions)
                .into_iter()
                .partition(|path| selected.contains(path));
        let mut gone: Vec<PathBuf> = selected.into_iter().filter(|f| !rows.contains(f)).collect();
        gone.sort();
        rows.extend(gone);
        rows.extend(others);
        self.workspace.folders = rows;
        self.workspace.folder_counts = crate::ui::cwd_counts(&self.sessions);
        self.workspace.rebuild_visible();

        if let Some(prev) = previous {
            let pos = self.workspace.visible_folders().position(|f| *f == prev);
            if let Some(pos) = pos {
                self.workspace.detail_cursor = FIRST_FOLDER_ROW + pos;
            }
        }
        let last = self.workspace.detail_rows().saturating_sub(1);
        self.workspace.detail_cursor = self.workspace.detail_cursor.min(last);
    }

    /// Opens the workspace at `idx` (`None` = "All"): the scope every session
    /// list shows from now on. Session selection restarts at the top.
    pub(crate) fn set_active_workspace(&mut self, idx: Option<usize>) {
        let before = self.workspaces.active.clone();
        self.workspaces.set_active(idx);
        if self.workspaces.active != before {
            self.persist_workspace_changes(&[self.opened_change()]);
            self.workspace_scope_changed();
        }
    }

    /// Re-applies a changed scope (open workspace, or the store it lives in):
    /// the Detail rows restart at the top and so does the session list.
    pub(crate) fn workspace_scope_changed(&mut self) {
        self.workspace.detail_cursor = 0;
        self.workspace.folder_scroll.set(0);
        self.workspace.clear_folder_query();
        self.refresh_workspace_folders();
        self.selected = 0;
        self.recompute();
        // The Workspaces screen edits one workspace; once "All" is open (the
        // edited one was cancelled or deleted elsewhere) there is nothing left.
        if self.screen == Screen::Workspace && self.workspaces.active_index().is_none() {
            self.leave_workspace_screen();
        }
    }

    /// Restores a workspace by id (context-source Back). An id that no longer
    /// exists opens "All".
    pub(crate) fn restore_active_workspace(&mut self, id: Option<&str>) {
        let idx = id.and_then(|id| self.workspaces.workspaces.iter().position(|w| w.id == id));
        self.set_active_workspace(idx);
    }

    /// Palette `Open Workspace <name>` / `Close Workspace`: opens the scope and
    /// shows it on the Session screen (not the Workspaces screen).
    pub(crate) fn open_workspace_from_palette(&mut self, idx: Option<usize>) {
        self.set_active_workspace(idx);
        self.switch_screen(Screen::Session);
        self.focus = Focus::Table;
        self.status_msg = Some(match self.active_workspace_name() {
            Some(name) => format!("Workspace opened: {name}"),
            None => "Workspace closed: all sessions".to_string(),
        });
    }

    /// Handles keys in the Session screen's workspace pane (`Focus::Workspaces`).
    /// The cursor opens the row's workspace; Enter edits it on the Workspaces
    /// screen; →/Esc close the pane and ← goes on to the Profile screen.
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
            KeyCode::Char('+') => self.workspace_pane_move(isize::MAX),
            KeyCode::Left | KeyCode::Char('h') => {
                self.close_workspace_pane();
                self.switch_screen(Screen::Profile);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Esc => self.close_workspace_pane(),
            KeyCode::Up | KeyCode::Char('k') => self.workspace_pane_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.workspace_pane_move(1),
            KeyCode::Home | KeyCode::Char('g') => self.workspace_pane_move(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.workspace_pane_move(isize::MAX),
            KeyCode::Enter if self.workspace.new_row => self.add_workspace(),
            KeyCode::Enter if self.workspaces.active_index().is_some() => {
                self.enter_workspace_screen()
            }
            KeyCode::Char('d') if ctrl => self.open_workspace_delete(),
            KeyCode::Delete => self.open_workspace_delete(),
            _ => {}
        }
    }

    /// Moves the workspace pane cursor, opening the row's workspace.
    /// `isize::MIN`/`MAX` jump to "All"/`[NEW WORKSPACE]`, which shows "All".
    fn workspace_pane_move(&mut self, delta: isize) {
        let new_row = self.workspaces.workspaces.len() + 1;
        let next = (self.workspace_pane_cursor() as isize)
            .saturating_add(delta)
            .clamp(0, new_row as isize) as usize;
        self.workspace.new_row = next == new_row;
        let idx = if self.workspace.new_row {
            None
        } else {
            next.checked_sub(1)
        };
        self.set_active_workspace(idx);
    }

    /// Handles keys on the Workspaces screen in table mode. Only its Detail
    /// pane takes keys; Esc returns to the Session screen's workspace pane.
    pub fn on_key_workspace(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let was_on_search = self.workspace_search_focused();
        if was_on_search && self.on_key_workspace_search(key) {
            self.update_workspace_search_selection(was_on_search);
            return;
        }

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
            KeyCode::Char('n') if ctrl && !key.modifiers.contains(KeyModifiers::SHIFT) => {
                let idx = self.filtered.get(self.selected).copied();
                self.open_new_session_modal_for_session(idx, false);
            }
            KeyCode::Up | KeyCode::Char('k') => self.workspace_detail_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.workspace_detail_move(1),
            KeyCode::Home | KeyCode::Char('g') => self.workspace_detail_move(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.workspace_detail_move(isize::MAX),
            KeyCode::Enter => {
                if let Some(field) = self.workspace.cursor_field() {
                    self.begin_workspace_edit(field);
                }
            }
            KeyCode::Char(' ') => self.toggle_workspace_folder(),
            // Entered with Enter, left with Esc: ←/→ do not leave the screen.
            KeyCode::Esc => {
                self.leave_workspace_screen();
                return;
            }
            _ => {}
        }
        self.update_workspace_search_selection(was_on_search);
    }

    /// True while the Detail cursor sits on the folder search row, which then
    /// takes typed characters and pastes directly.
    pub(crate) fn workspace_search_focused(&self) -> bool {
        self.screen == Screen::Workspace
            && self.mode == UiMode::Table
            && self.workspace.cursor_on_search()
            && self.workspaces.active_index().is_some()
    }

    /// Arriving on the Search row selects the whole query, so typing replaces
    /// it; leaving drops the selection.
    fn update_workspace_search_selection(&mut self, was_on_search: bool) {
        let on_search = self.workspace_search_focused();
        let query = &mut self.workspace.folder_query;
        if on_search && !was_on_search {
            query.select_all = !query.value.is_empty();
            query.cursor = query.value.len();
        } else if !on_search {
            query.select_all = false;
        }
    }

    /// Keys on the folder search row. Printable characters (including the
    /// pane's letter shortcuts and space) edit the query and ←/→/Home/End move
    /// its text cursor, so the row is left only with ↑/↓. ctrl combinations
    /// fall through to the pane handler. Returns whether the key was consumed.
    fn on_key_workspace_search(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        let query = &mut self.workspace.folder_query;
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                query.insert_char(c)
            }
            KeyCode::Backspace => query.backspace(),
            KeyCode::Esc if !query.value.is_empty() => self.workspace.clear_folder_query(),
            KeyCode::Enter => {
                // Jump to the first match so space can toggle it right away.
                if !self.workspace.visible.is_empty() {
                    self.workspace.detail_cursor = FIRST_FOLDER_ROW;
                }
                return true;
            }
            // Text cursor keys stay in the query: the row never moves panes.
            // A whole-query selection collapses to the matching edge first.
            KeyCode::Left => {
                query.move_left();
                return true;
            }
            KeyCode::Right => {
                query.move_right();
                return true;
            }
            KeyCode::Home => {
                query.home();
                return true;
            }
            KeyCode::End => {
                query.end();
                return true;
            }
            // Deletes at the cursor instead of opening the delete dialog.
            KeyCode::Delete => query.delete(),
            _ => return false,
        }
        self.quit_armed = false;
        self.status_msg = None;
        self.workspace_folder_query_changed();
        true
    }

    pub(crate) fn paste_into_workspace_search(&mut self, text: &str) {
        let outcome = self.workspace.folder_query.insert_paste(text);
        if outcome.inserted > 0 {
            self.workspace_folder_query_changed();
        }
        self.note_paste_outcome(outcome);
    }

    fn workspace_folder_query_changed(&mut self) {
        self.workspace.rebuild_visible();
        self.workspace.folder_scroll.set(0);
    }

    /// Moves the Detail cursor. `isize::MIN`/`MAX` jump to the first/last row.
    fn workspace_detail_move(&mut self, delta: isize) {
        let last = self.workspace.detail_rows().saturating_sub(1);
        self.workspace.detail_cursor = (self.workspace.detail_cursor as isize)
            .saturating_add(delta)
            .clamp(0, last as isize) as usize;
    }

    /// Enter on `[NEW WORKSPACE]`: appends "New Workspace", opens it, and
    /// shows it on the Workspaces screen with its Name row in edit. The
    /// workspace is saved only when the name is confirmed.
    fn add_workspace(&mut self) {
        let ws = Workspace::new(self.workspaces.new_id(), self.workspaces.next_new_name());
        self.workspaces.workspaces.push(ws);
        let idx = self.workspaces.workspaces.len() - 1;
        // Open the new (still empty, so all-matching) scope without saving it.
        self.workspaces.set_active(Some(idx));
        self.workspace.new_row = false;
        self.selected = 0;
        self.recompute();
        self.enter_workspace_screen();
        let name = self.workspaces.workspaces[idx].name.clone();
        self.workspace.edit = Some(WorkspaceEdit {
            workspace: idx,
            field: WorkspaceField::Name,
            input: TextInput::selected(name.clone()),
            original: name,
            created: true,
        });
        self.mode = UiMode::WorkspaceEdit;
    }

    fn begin_workspace_edit(&mut self, field: WorkspaceField) {
        let Some(idx) = self.workspaces.active_index() else {
            self.status_msg = Some("The All workspace cannot be edited".to_string());
            return;
        };
        let ws = &self.workspaces.workspaces[idx];
        let value = match field {
            WorkspaceField::Name => ws.name.clone(),
            WorkspaceField::Includes => ws.includes.clone(),
            WorkspaceField::Excludes => ws.excludes.clone(),
        };
        self.workspace.edit = Some(WorkspaceEdit {
            workspace: idx,
            field,
            input: TextInput::new(value.clone()),
            original: value,
            created: false,
        });
        self.mode = UiMode::WorkspaceEdit;
    }

    /// Handles keys while a workspace name/include/exclude value is edited.
    /// Enter saves, Esc restores the previous value.
    pub fn on_key_workspace_edit(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(edit) = self.workspace.edit.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Enter => return self.commit_workspace_edit(),
            KeyCode::Esc => return self.cancel_workspace_edit(),
            KeyCode::Left => edit.input.move_left(),
            KeyCode::Right => edit.input.move_right(),
            KeyCode::Home => edit.input.home(),
            KeyCode::End => edit.input.end(),
            KeyCode::Backspace => edit.input.backspace(),
            KeyCode::Delete => edit.input.delete(),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                edit.input.insert_char(c)
            }
            _ => return,
        }
        self.apply_live_workspace_edit();
    }

    pub(crate) fn paste_into_workspace_edit(&mut self, text: &str) {
        let Some(edit) = self.workspace.edit.as_mut() else {
            return;
        };
        let outcome = edit.input.insert_paste(text);
        if outcome.inserted > 0 {
            self.apply_live_workspace_edit();
        }
        self.note_paste_outcome(outcome);
    }

    /// Include/exclude words filter the session list while they are typed.
    /// Names change nothing until confirmed.
    fn apply_live_workspace_edit(&mut self) {
        let Some(edit) = self.workspace.edit.as_ref() else {
            return;
        };
        let value = edit.input.value.clone();
        let Some(ws) = self.workspaces.workspaces.get_mut(edit.workspace) else {
            return;
        };
        match edit.field {
            WorkspaceField::Name => return,
            WorkspaceField::Includes => ws.includes = value,
            WorkspaceField::Excludes => ws.excludes = value,
        }
        self.recompute();
    }

    fn commit_workspace_edit(&mut self) {
        let Some(edit) = self.workspace.edit.as_ref() else {
            self.mode = UiMode::Table;
            return;
        };
        let idx = edit.workspace;
        let created = edit.created;
        let Some(mut ws) = self.workspaces.workspaces.get(idx).cloned() else {
            self.workspace.edit = None;
            self.mode = UiMode::Table;
            return;
        };
        let mut status = None;
        if edit.field == WorkspaceField::Name {
            let name = edit.input.value.trim().to_string();
            if name.is_empty() {
                self.status_msg = Some("Workspace name cannot be empty".to_string());
                return;
            }
            if !self.workspaces.name_available(&name, Some(idx)) {
                self.status_msg = Some(format!("A workspace named '{name}' already exists"));
                return;
            }
            ws.name = name.clone();
            status = Some(if created {
                format!("Workspace added: {name}")
            } else {
                format!("Workspace renamed: {name}")
            });
        }
        let mut changes = vec![WorkspaceChange::Upsert(ws.clone())];
        if created {
            changes.push(WorkspaceChange::Opened(Some(ws.id.clone())));
        }
        // A failed save keeps the edit open (Esc still cancels it), e.g. when
        // another instance already saved a workspace under this name.
        if !self.persist_workspace_changes(&changes) {
            return;
        }
        self.workspaces.workspaces[idx] = ws;
        self.workspace.edit = None;
        self.mode = UiMode::Table;
        self.status_msg = status;
    }

    fn cancel_workspace_edit(&mut self) {
        let Some(edit) = self.workspace.edit.take() else {
            self.mode = UiMode::Table;
            return;
        };
        self.mode = UiMode::Table;
        if edit.created {
            if edit.workspace < self.workspaces.workspaces.len() {
                self.workspaces.workspaces.remove(edit.workspace);
            }
            // Back to the `[NEW WORKSPACE]` row it was created from, which
            // shows "All"; the scope change leaves the Workspaces screen.
            self.workspaces.set_active(None);
            self.workspace_scope_changed();
            self.workspace.new_row = true;
            return;
        }
        if let Some(ws) = self.workspaces.workspaces.get_mut(edit.workspace) {
            match edit.field {
                WorkspaceField::Name => return,
                WorkspaceField::Includes => ws.includes = edit.original,
                WorkspaceField::Excludes => ws.excludes = edit.original,
            }
        }
        self.recompute();
    }

    /// Space on a folder row: adds/removes it from the open workspace.
    fn toggle_workspace_folder(&mut self) {
        let Some(folder) = self.workspace.cursor_folder().cloned() else {
            return;
        };
        let Some(idx) = self.workspaces.active_index() else {
            return;
        };
        self.workspaces.workspaces[idx].toggle_folder(&folder);
        let ws = self.workspaces.workspaces[idx].clone();
        self.persist_workspace_changes(&[WorkspaceChange::Upsert(ws)]);
        self.recompute();
    }

    fn open_workspace_delete(&mut self) {
        let Some(idx) = self.workspaces.active_index() else {
            self.status_msg = Some(if self.workspace.new_row {
                "Select a workspace to delete".to_string()
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
