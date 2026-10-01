//! `ctrl+u` reload of app-owned stores that other running s7s instances may
//! have changed: bookmarks, workspaces, and profiles. These are otherwise read
//! only at startup — there is no file watching — and every save already merges
//! into the current file, so a reload only brings the screen up to date.

use super::refresh::SessionKey;
use super::{App, MessageKind};

impl App {
    /// Replaces the in-memory bookmarks, workspaces, and profiles with their
    /// files, keeping this instance's own view state: the open workspace (unless
    /// it was deleted elsewhere, which opens "All"), the selected profile, and
    /// the selected session. Runs at the start of a `ctrl+u` cycle, before the
    /// session scan, so the scan and usage probes see reloaded profiles.
    /// A file that cannot be read keeps the in-memory copy and is reported.
    pub(crate) fn reload_shared_stores(&mut self) {
        let current = self.current().map(SessionKey::of);
        let mut errors = Vec::new();

        // Unit tests keep the real bookmarks path unless a test points it at a
        // temporary store; never read the user's file from a test.
        if !(cfg!(test) && self.bookmarks_path == crate::config::bookmarks_path()) {
            match crate::bookmarks::BookmarkStore::load(&self.bookmarks_path) {
                Ok(store) => self.bookmarks = store,
                Err(err) => errors.push(format!("bookmarks.json: {err}")),
            }
        }

        let mut scope_changed = false;
        if let Some(path) = self.workspaces_path.clone() {
            match crate::workspaces::WorkspaceStore::load(&path) {
                Ok(mut store) => {
                    // The file's `active` is another instance's last scope.
                    store.active = self.workspaces.active.clone();
                    if store.active_index().is_none() {
                        store.active = None;
                    }
                    scope_changed = store.active != self.workspaces.active;
                    self.workspaces = store;
                }
                Err(err) => errors.push(format!("workspaces.json: {err}")),
            }
        }

        if let Some(path) = self.profiles_path.clone() {
            match crate::profile::ProfileStore::load_checked(&path) {
                Ok(store) => {
                    let selected = self
                        .profiles
                        .profiles
                        .get(self.profile_selected)
                        .map(|p| p.id.clone());
                    self.profiles = store;
                    let last = self.profiles.profiles.len().saturating_sub(1);
                    self.profile_selected = selected
                        .and_then(|id| self.profiles.profiles.iter().position(|p| p.id == id))
                        .unwrap_or(self.profile_selected)
                        .min(last);
                    let profiles = &self.profiles;
                    self.filter
                        .profile_ids
                        .retain(|id| profiles.find(id).is_some());
                }
                Err(err) => errors.push(format!("profiles.json: {err}")),
            }
        }

        if scope_changed {
            self.workspace_scope_changed();
        } else {
            self.refresh_workspace_folders();
            self.rebuild_filtered();
            let position = current.and_then(|key| {
                self.filtered
                    .iter()
                    .position(|&i| key.matches(&self.sessions[i]))
            });
            match position {
                Some(pos) => self.selected = pos,
                None => {
                    self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
                    self.preview_scroll = 0;
                    self.preview_expanded = false;
                }
            }
        }

        if !errors.is_empty() {
            let mut lines =
                vec!["These files could not be read and were not reloaded:".to_string()];
            lines.extend(errors);
            self.show_message(" Reload Failed ", lines, MessageKind::Warn);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::profile::{ProfileChange, ProfileStore};
    use crate::ui::test_support::*;
    use crate::ui::workspace::WorkspacePane;
    use crate::ui::{App, UiMode};
    use crate::workspaces::{Workspace, WorkspaceChange, WorkspaceStore};
    use std::path::PathBuf;

    /// App whose three shared stores live in a temporary directory.
    fn app_with_stores(root: &TempBookmarkStore) -> App {
        let mut app = app_with_context_chain();
        app.bookmarks_path = root.path.clone();
        app.workspaces_path = Some(root.path.with_file_name("workspaces.json"));
        app.profiles_path = Some(root.path.with_file_name("profiles.json"));
        app
    }

    fn names(app: &App) -> Vec<String> {
        app.workspaces
            .workspaces
            .iter()
            .map(|w| w.name.clone())
            .collect()
    }

    #[test]
    fn ctrl_u_shows_workspaces_another_instance_saved_and_keeps_this_scope() {
        let root = TempBookmarkStore::new();
        let mut app = app_with_stores(&root);
        let path = app.workspaces_path.clone().unwrap();
        let mut mine = Workspace::new("mine".into(), "Mine".into());
        mine.includes = "leaf".into();
        app.workspaces.workspaces.push(mine.clone());
        WorkspaceStore::commit(&path, &[WorkspaceChange::Upsert(mine)]).unwrap();
        app.set_active_workspace(Some(0));

        // Another instance adds a workspace and opens it.
        let theirs = Workspace::new("theirs".into(), "Theirs".into());
        WorkspaceStore::commit(
            &path,
            &[
                WorkspaceChange::Upsert(theirs),
                WorkspaceChange::Opened(Some("theirs".into())),
            ],
        )
        .unwrap();
        assert_eq!(names(&app), ["Mine"], "nothing changes before ctrl+u");

        app.reload_shared_stores();
        assert_eq!(names(&app), ["Mine", "Theirs"]);
        assert_eq!(app.active_workspace_name(), Some("Mine"));
        assert_eq!(app.filtered.len(), 1);
    }

    #[test]
    fn a_save_keeps_workspaces_another_instance_added() {
        let root = TempBookmarkStore::new();
        let mut app = app_with_stores(&root);
        let path = app.workspaces_path.clone().unwrap();
        let theirs = Workspace::new("theirs".into(), "Theirs".into());
        WorkspaceStore::commit(&path, &[WorkspaceChange::Upsert(theirs)]).unwrap();

        // This instance still lists none of them; adding and moving the cursor
        // must not drop "Theirs" from the file.
        app.enter_workspace_screen(WorkspacePane::List);
        app.on_key_workspace(key(
            crossterm::event::KeyCode::Char('+'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.on_key_workspace_edit(key(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        app.set_active_workspace(None);
        let stored = WorkspaceStore::load(&path).unwrap();
        let stored: Vec<&str> = stored.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(stored, ["Theirs", "New Workspace"]);
    }

    #[test]
    fn a_name_saved_elsewhere_keeps_the_edit_open() {
        let root = TempBookmarkStore::new();
        let mut app = app_with_stores(&root);
        let path = app.workspaces_path.clone().unwrap();
        let theirs = Workspace::new("theirs".into(), "New Workspace".into());
        WorkspaceStore::commit(&path, &[WorkspaceChange::Upsert(theirs)]).unwrap();

        app.enter_workspace_screen(WorkspacePane::List);
        app.on_key_workspace(key(
            crossterm::event::KeyCode::Char('+'),
            crossterm::event::KeyModifiers::NONE,
        ));
        app.on_key_workspace_edit(key(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.mode, UiMode::WorkspaceEdit);
        assert!(app
            .status_msg
            .as_deref()
            .is_some_and(|m| m.contains("already exists")));
    }

    #[test]
    fn a_workspace_deleted_elsewhere_reopens_all() {
        let root = TempBookmarkStore::new();
        let mut app = app_with_stores(&root);
        let path = app.workspaces_path.clone().unwrap();
        let mine = Workspace::new("mine".into(), "Mine".into());
        app.workspaces.workspaces.push(mine.clone());
        WorkspaceStore::commit(&path, &[WorkspaceChange::Upsert(mine)]).unwrap();
        app.set_active_workspace(Some(0));
        WorkspaceStore::commit(&path, &[WorkspaceChange::Remove("mine".into())]).unwrap();

        app.reload_shared_stores();
        assert!(app.workspaces.workspaces.is_empty());
        assert_eq!(app.active_workspace_name(), None);
    }

    #[test]
    fn ctrl_u_reloads_bookmarks_and_profiles_keeping_the_selection() {
        let root = TempBookmarkStore::new();
        let mut app = app_with_stores(&root);
        let profiles = app.profiles_path.clone().unwrap();
        app.selected = 1;
        let selected = app.current().unwrap().id.clone();

        // Another instance bookmarks "root" and adds a profile.
        let root_session = app
            .sessions
            .iter()
            .find(|s| s.id == "root")
            .unwrap()
            .clone();
        crate::bookmarks::BookmarkStore::toggle(&app.bookmarks_path, &root_session).unwrap();
        let mut extra = ProfileStore::load_checked(&profiles).unwrap().profiles[0].clone();
        extra.id = "profile-extra".into();
        extra.name = "Extra".into();
        extra.builtin = false;
        extra.path = PathBuf::from("/tmp/s7s-extra-profile");
        ProfileStore::commit(&profiles, &[ProfileChange::Upsert(extra)]).unwrap();

        app.reload_shared_stores();
        assert!(app.bookmarks.contains(&root_session));
        assert!(app.profiles.find("profile-extra").is_some());
        assert_eq!(
            app.current().unwrap().id,
            selected,
            "selection follows the session"
        );
        assert_eq!(app.sessions[app.filtered[0]].id, "root", "bookmarks lead");
    }

    #[test]
    fn an_unreadable_store_is_reported_and_kept_in_memory() {
        let root = TempBookmarkStore::new();
        let mut app = app_with_stores(&root);
        app.workspaces
            .workspaces
            .push(Workspace::new("mine".into(), "Mine".into()));
        std::fs::write(app.workspaces_path.as_ref().unwrap(), "not json").unwrap();
        app.reload_shared_stores();
        assert_eq!(names(&app), ["Mine"]);
        assert_eq!(app.mode, UiMode::Message);
    }
}
