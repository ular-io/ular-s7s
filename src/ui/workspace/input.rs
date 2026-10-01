//! Workspaces screen key handling and workspace mutations: opening a scope,
//! pane focus, the in-place name/include/exclude edit, folder toggles, and
//! creation/deletion. The Sessions pane delegates to the Session screen's table
//! handler so its shortcuts stay identical.

use super::state::{WorkspaceEdit, WorkspaceField, WorkspacePane, DETAIL_FIELDS};
use crate::ui::{App, Focus, Screen, TextInput, UiMode};
use crate::workspaces::{Workspace, WorkspaceChange, WorkspaceStore};
use std::collections::HashMap;
use std::path::PathBuf;

impl App {
    /// Shows the Workspaces screen with `pane` focused. The Detail pane is
    /// skipped while "All" is open because it has nothing to edit.
    pub(crate) fn enter_workspace_screen(&mut self, pane: WorkspacePane) {
        self.switch_screen(Screen::Workspace);
        self.focus = Focus::Table;
        self.refresh_workspace_folders();
        self.workspace.pane = self.reachable_pane(pane);
    }

    fn reachable_pane(&self, pane: WorkspacePane) -> WorkspacePane {
        if pane == WorkspacePane::Detail && self.workspaces.active_index().is_none() {
            WorkspacePane::List
        } else {
            pane
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
    /// folders in selection order, then every other session folder by latest
    /// activity. A cursor on a folder row stays on the same folder.
    pub(crate) fn refresh_workspace_folders(&mut self) {
        let previous = self.workspace.cursor_folder().cloned();
        let mut rows: Vec<PathBuf> = self
            .workspaces
            .active_workspace()
            .map(|w| w.folders.clone())
            .unwrap_or_default();
        let mut latest: HashMap<&std::path::Path, i64> = HashMap::new();
        for s in &self.sessions {
            if s.cwd.as_os_str().is_empty() {
                continue;
            }
            let at = latest.entry(s.cwd.as_path()).or_insert(i64::MIN);
            *at = (*at).max(s.updated_at_ms);
        }
        let mut others: Vec<(&std::path::Path, i64)> = latest
            .into_iter()
            .filter(|(path, _)| !rows.iter().any(|r| r == path))
            .collect();
        others.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        rows.extend(others.into_iter().map(|(path, _)| path.to_path_buf()));
        self.workspace.folders = rows;

        if let Some(prev) = previous {
            if let Some(pos) = self.workspace.folders.iter().position(|f| *f == prev) {
                self.workspace.detail_cursor = DETAIL_FIELDS.len() + pos;
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
        self.refresh_workspace_folders();
        self.selected = 0;
        self.recompute();
        self.workspace.pane = self.reachable_pane(self.workspace.pane);
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

    /// Handles keys on the Workspaces screen in table mode.
    pub fn on_key_workspace(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let pane = self.workspace.pane;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let pane_key = matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
        ) || key.code == KeyCode::Char('+');
        if pane == WorkspacePane::Sessions && !pane_key {
            // Same shortcuts as the Session screen's list; its focus model is
            // pinned to the table so preview-only keys stay inert here.
            self.focus = Focus::Table;
            self.on_key_table(key);
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
            KeyCode::Char('+') => self.add_workspace(),
            KeyCode::Left | KeyCode::Char('h') => self.workspace_pane_left(),
            KeyCode::Right | KeyCode::Char('l') => self.workspace_pane_right(),
            KeyCode::Up | KeyCode::Char('k') => self.workspace_move(-1),
            KeyCode::Down | KeyCode::Char('j') => self.workspace_move(1),
            KeyCode::Home | KeyCode::Char('g') => self.workspace_move(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.workspace_move(isize::MAX),
            KeyCode::Enter => match pane {
                WorkspacePane::List => self.begin_workspace_edit(WorkspaceField::Name, true),
                _ => {
                    if let Some(field) = self.workspace.cursor_field() {
                        self.begin_workspace_edit(field, false);
                    }
                }
            },
            KeyCode::Char(' ') if pane == WorkspacePane::Detail => self.toggle_workspace_folder(),
            KeyCode::Char('d') if ctrl => self.open_workspace_delete(),
            KeyCode::Delete => self.open_workspace_delete(),
            KeyCode::Esc => {
                self.status_msg = Some("Press q or ctrl+c twice to quit".to_string());
            }
            _ => {}
        }
    }

    fn workspace_pane_left(&mut self) {
        self.workspace.pane = match self.workspace.pane {
            WorkspacePane::List => {
                self.switch_screen(Screen::Profile);
                return;
            }
            WorkspacePane::Detail => WorkspacePane::List,
            WorkspacePane::Sessions => self.reachable_pane(WorkspacePane::Detail),
        };
    }

    fn workspace_pane_right(&mut self) {
        self.workspace.pane = match self.workspace.pane {
            WorkspacePane::List if self.workspaces.active_index().is_some() => {
                WorkspacePane::Detail
            }
            WorkspacePane::List | WorkspacePane::Detail => WorkspacePane::Sessions,
            WorkspacePane::Sessions => {
                // Continue to the full Session screen with the same scope and row.
                self.switch_screen(Screen::Session);
                self.focus = Focus::Table;
                return;
            }
        };
    }

    /// Moves the cursor of the focused List/Detail pane. `isize::MIN`/`MAX`
    /// jump to the first/last row. On the list this opens the row's workspace.
    fn workspace_move(&mut self, delta: isize) {
        let (current, len) = match self.workspace.pane {
            WorkspacePane::List => (
                self.workspaces.active_index().map_or(0, |i| i + 1),
                self.workspaces.workspaces.len() + 1,
            ),
            WorkspacePane::Detail => (self.workspace.detail_cursor, self.workspace.detail_rows()),
            WorkspacePane::Sessions => return,
        };
        let next = (current as isize)
            .saturating_add(delta)
            .clamp(0, len.saturating_sub(1) as isize) as usize;
        match self.workspace.pane {
            WorkspacePane::List => self.set_active_workspace(next.checked_sub(1)),
            _ => self.workspace.detail_cursor = next,
        }
    }

    /// `+`: appends "New Workspace", opens it, and starts editing its name in
    /// the list. The workspace is saved only when the name is confirmed.
    fn add_workspace(&mut self) {
        let previous_active = self.workspaces.active.clone();
        let ws = Workspace::new(self.workspaces.new_id(), self.workspaces.next_new_name());
        self.workspaces.workspaces.push(ws);
        let idx = self.workspaces.workspaces.len() - 1;
        // Open the new (still empty, so all-matching) scope without saving it.
        self.workspaces.set_active(Some(idx));
        self.workspace.detail_cursor = 0;
        self.refresh_workspace_folders();
        self.selected = 0;
        self.recompute();
        self.workspace.pane = WorkspacePane::List;
        let name = self.workspaces.workspaces[idx].name.clone();
        self.workspace.edit = Some(WorkspaceEdit {
            workspace: idx,
            field: WorkspaceField::Name,
            in_list: true,
            input: TextInput::selected(name.clone()),
            original: name,
            created: true,
            previous_active,
        });
        self.mode = UiMode::WorkspaceEdit;
    }

    fn begin_workspace_edit(&mut self, field: WorkspaceField, in_list: bool) {
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
            in_list,
            input: TextInput::new(value.clone()),
            original: value,
            created: false,
            previous_active: None,
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
            let previous = edit
                .previous_active
                .as_deref()
                .and_then(|id| self.workspaces.workspaces.iter().position(|w| w.id == id));
            self.workspaces.set_active(previous);
            self.workspace_scope_changed();
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
            self.status_msg = Some("The All workspace cannot be deleted".to_string());
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
