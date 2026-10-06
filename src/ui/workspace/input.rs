//! Workspace key handling and mutations: the Session screen's workspace pane
//! (opening a scope, `[NEW WORKSPACE]`, deletion) and the edit dialog it opens
//! (a draft name, include/exclude words, and a `Folders ▾` checklist with its
//! search, saved only by its Save button).

use super::state::{WorkspaceDialog, WorkspaceField, DIALOG_FIELDS};
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

    /// Rebuilds the edit dialog's folder rows (when the checklist opens, and
    /// after a session rescan): the draft's selected folders first, then every
    /// other session folder, each group by latest activity. A selected folder
    /// with no session sorts last in its group. An open checklist's cursor
    /// stays on the same folder. No-op while the dialog is closed.
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

        if let Some(row) = dialog.folder_list {
            let kept = previous.and_then(|prev| dialog.visible_folders().position(|f| *f == prev));
            dialog.folder_list = Some(kept.map_or(row, |pos| pos + 1).min(dialog.list_rows() - 1));
        }
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

    /// Handles keys in the workspace edit dialog. Text rows take typing
    /// directly; ↑/↓ move one row and Tab/BackTab do the same, wrapping; Enter
    /// on a text row moves on, Enter or space on `Folders ▾` opens its
    /// checklist, and only the Save button saves. Esc cancels.
    pub fn on_key_workspace_dialog(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        if dialog.folder_list.is_some() {
            return self.on_key_workspace_folder_list(key);
        }
        let on_combo = dialog.cursor_field() == Some(WorkspaceField::Folders);
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        match key.code {
            KeyCode::Esc => self.cancel_workspace_dialog(),
            KeyCode::Tab => self.workspace_dialog_move(1, true),
            KeyCode::BackTab => self.workspace_dialog_move(-1, true),
            KeyCode::Up => self.workspace_dialog_move(-1, false),
            KeyCode::Down => self.workspace_dialog_move(1, false),
            _ if dialog.on_buttons => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') => {
                    dialog.save_focused = !dialog.save_focused
                }
                KeyCode::Enter if dialog.save_focused => self.save_workspace_dialog(),
                KeyCode::Enter => self.cancel_workspace_dialog(),
                _ => {}
            },
            // A closed combo changes its value only through the open checklist.
            KeyCode::Enter | KeyCode::Char(' ') if on_combo && plain => {
                self.open_workspace_folder_list()
            }
            _ if on_combo => {}
            // A text input never submits the form: Enter moves to the next row.
            KeyCode::Enter => self.workspace_dialog_move(1, false),
            _ => self.edit_workspace_dialog_text(key),
        }
    }

    /// Keys of the open folder checklist, which owns every key: ↑/↓ move,
    /// space toggles the cursor row, typing filters, Enter closes, Tab/BackTab
    /// close and move on, Esc clears a query and then closes. Toggles apply at
    /// once, so closing never changes the selection.
    fn on_key_workspace_folder_list(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let Some(row) = dialog.folder_list else {
            return;
        };
        let last = dialog.list_rows() - 1;
        match key.code {
            KeyCode::Up => dialog.folder_list = Some(row.saturating_sub(1)),
            KeyCode::Down => dialog.folder_list = Some((row + 1).min(last)),
            KeyCode::PageUp => dialog.folder_list = Some(row.saturating_sub(10)),
            KeyCode::PageDown => dialog.folder_list = Some((row + 10).min(last)),
            KeyCode::Enter => self.close_workspace_folder_list(),
            KeyCode::Esc if !dialog.folder_query.value.is_empty() => {
                // The cursor stays on its folder as the hidden rows return.
                let kept = dialog.cursor_folder().cloned();
                dialog.folder_query = crate::ui::TextInput::new(String::new());
                dialog.rebuild_visible();
                let pos = kept.and_then(|k| dialog.visible_folders().position(|f| *f == k));
                dialog.folder_list = Some(pos.map_or(0, |p| p + 1));
            }
            KeyCode::Esc => self.close_workspace_folder_list(),
            KeyCode::Tab => {
                self.close_workspace_folder_list();
                self.workspace_dialog_move(1, true);
            }
            KeyCode::BackTab => {
                self.close_workspace_folder_list();
                self.workspace_dialog_move(-1, true);
            }
            _ if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) => {}
            KeyCode::Char(' ') => self.toggle_workspace_folder(),
            _ => self.edit_workspace_dialog_text(key),
        }
    }

    /// Opens the `Folders ▾` checklist on `[ALL FOLDERS]` with an empty
    /// search, the rows re-sorted so the current selection comes first.
    fn open_workspace_folder_list(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        dialog.folder_query = crate::ui::TextInput::new(String::new());
        dialog.folder_list = None;
        dialog.folder_scroll.set(0);
        self.refresh_workspace_folders();
        if let Some(dialog) = self.workspace.dialog.as_mut() {
            dialog.folder_list = Some(0);
        }
    }

    /// Closes the checklist, keeping every toggle, and drops its search.
    fn close_workspace_folder_list(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        dialog.folder_list = None;
        dialog.folder_query = crate::ui::TextInput::new(String::new());
        dialog.rebuild_visible();
    }

    /// Edits the text input under the cursor (a field, or the open
    /// checklist's search): characters, deletion, and text-cursor movement.
    fn edit_workspace_dialog_text(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(input) = self
            .workspace
            .dialog
            .as_mut()
            .and_then(|d| d.cursor_input())
        else {
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

    /// Inserts a bracketed paste into the text input under the cursor. The
    /// closed folder combo and the buttons own no text, so a paste there is
    /// dropped.
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

    /// After an edit of the text input under the cursor: the checklist search
    /// filters the rows and puts the cursor on the first match (on
    /// `[ALL FOLDERS]` when none); include/exclude words update the match count.
    fn workspace_dialog_text_changed(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        dialog.error = None;
        if dialog.folder_list.is_some() {
            dialog.rebuild_visible();
            dialog.folder_list = Some(usize::from(!dialog.visible.is_empty()));
            dialog.folder_scroll.set(0);
            return;
        }
        dialog.draft.includes = dialog.includes.value.clone();
        dialog.draft.excludes = dialog.excludes.value.clone();
        self.update_workspace_matching();
    }

    /// Moves one row through the fields and the button row. `wrap` (Tab /
    /// BackTab) cycles past either end; ↑/↓ stop there.
    fn workspace_dialog_move(&mut self, delta: isize, wrap: bool) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        let buttons = DIALOG_FIELDS.len() as isize;
        let current = if dialog.on_buttons {
            buttons
        } else {
            dialog.cursor as isize
        };
        let next = if wrap {
            (current + delta).rem_euclid(buttons + 1)
        } else {
            (current + delta).clamp(0, buttons)
        } as usize;
        dialog.on_buttons = next == DIALOG_FIELDS.len();
        if !dialog.on_buttons {
            dialog.cursor = next;
        }
    }

    /// Space in the open checklist: toggles the cursor folder in the draft, or
    /// on `[ALL FOLDERS]` clears the selection. An empty selection is every
    /// folder, so the first toggle from it selects that folder alone and
    /// clearing the last one returns to every folder.
    fn toggle_workspace_folder(&mut self) {
        let Some(dialog) = self.workspace.dialog.as_mut() else {
            return;
        };
        match dialog.cursor_folder().cloned() {
            Some(folder) => dialog.draft.toggle_folder(&folder),
            None => dialog.draft.folders.clear(),
        }
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
            dialog.folder_list = None;
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
