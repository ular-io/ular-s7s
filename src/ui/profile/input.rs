//! Profile key handling and persistence logic: table navigation and shortcut
//! assignment, the add/edit form, deletion confirmation, and config-directory
//! creation confirmation (including the login request emitted on save).

use crate::model::Agent;
use crate::ui::{App, FormFocus, MessageKind, ProfileFormState, Screen, TextInput, UiMode};

impl App {
    fn move_profile_selection(&mut self, delta: isize) {
        let len = self.profiles.profiles.len() as isize;
        if len == 0 {
            return;
        }
        let s = (self.profile_selected as isize + delta).clamp(0, len - 1);
        self.profile_selected = s as usize;
    }

    /// Profile screen number keys: Inserts the selected profile at the requested shortcut position.
    fn assign_profile_slot(&mut self, slot_idx: usize) {
        let Some((id, name)) = self
            .profiles
            .profiles
            .get(self.profile_selected)
            .map(|p| (p.id.clone(), p.name.clone()))
        else {
            return;
        };
        if !self
            .profiles
            .assign_shortcut_slot(self.profile_selected, slot_idx)
        {
            return;
        }
        if let Err(e) = self.profiles.save() {
            self.status_msg = Some(format!("failed to save profiles.json: {e}"));
            return;
        }
        let assigned = self
            .profiles
            .numbered_profiles()
            .iter()
            .position(|p| p.id == id)
            .map(|idx| idx + 1)
            .unwrap_or(slot_idx + 1);
        self.status_msg = Some(format!("Profile shortcut <{assigned}>: {name}"));
    }

    /// Profile screen Space: Toggles the selected shortcut, appending new assignments at the end.
    pub(crate) fn toggle_selected_profile_shortcut(&mut self) {
        let Some(name) = self
            .profiles
            .profiles
            .get(self.profile_selected)
            .map(|p| p.name.clone())
        else {
            return;
        };
        let result = self.profiles.toggle_shortcut(self.profile_selected);
        let status = match result {
            crate::profile::ShortcutToggle::Assigned(slot) => {
                format!("Profile shortcut <{slot}>: {name}")
            }
            crate::profile::ShortcutToggle::Removed => {
                format!("Profile shortcut removed: {name}")
            }
            crate::profile::ShortcutToggle::Full => {
                self.show_message(
                    " Cannot Assign Shortcut ",
                    vec![
                        "All 5 profile shortcuts are already assigned.".to_string(),
                        "Remove a shortcut before assigning another profile.".to_string(),
                    ],
                    MessageKind::Error,
                );
                return;
            }
            crate::profile::ShortcutToggle::Invalid => return,
        };
        if let Err(e) = self.profiles.save() {
            self.status_msg = Some(format!("failed to save profiles.json: {e}"));
            return;
        }
        self.status_msg = Some(status);
    }

    /// Opens the profile form for creation (`editing = None`) or modification.
    pub(crate) fn open_profile_form(&mut self, editing: Option<usize>) {
        let form = match editing {
            Some(idx) => {
                let Some(p) = self.profiles.profiles.get(idx) else {
                    return;
                };
                ProfileFormState {
                    editing_id: Some(p.id.clone()),
                    builtin: p.builtin,
                    agy_allowed: p.agent == Agent::Antigravity,
                    agent_idx: Agent::all().iter().position(|a| *a == p.agent).unwrap_or(0),
                    name: TextInput::new(p.name.clone()),
                    path: TextInput::new(p.path.to_string_lossy().into_owned()),
                    focus: FormFocus::Name,
                    save_focused: true,
                    error: None,
                }
            }
            None => ProfileFormState {
                editing_id: None,
                builtin: false,
                agy_allowed: false,
                agent_idx: 0,
                name: TextInput::new(String::new()),
                path: TextInput::new(String::new()),
                focus: FormFocus::Agent,
                save_focused: true,
                error: None,
            },
        };
        self.profile_form = Some(form);
        self.mode = UiMode::ProfileForm;
        self.status_msg = None;
    }

    fn cancel_profile_form(&mut self) {
        self.profile_form = None;
        self.mode = UiMode::Table;
    }

    /// Form submission: stays in the form and displays validation messages on error.
    pub(crate) fn confirm_profile_form(&mut self) {
        let Some(form) = self.profile_form.as_ref() else {
            self.mode = UiMode::Table;
            return;
        };
        let name = form.name.value.trim().to_string();
        let path_str = form.path.value.trim().to_string();
        let agent = Agent::all()[form.agent_idx.min(Agent::all().len() - 1)];
        let editing_id = form.editing_id.clone();
        let exclude = editing_id.as_deref();

        let error = if name.is_empty() {
            Some("Name is required".to_string())
        } else if path_str.is_empty() {
            Some("Path is required".to_string())
        } else if agent == Agent::Antigravity && !form.agy_allowed {
            // Defensively check during saving, though already blocked in the radio UI selector.
            Some("Antigravity does not support custom config folders".to_string())
        } else if self.profiles.name_exists(&name, exclude) {
            Some(format!("Name already in use: {name}"))
        } else {
            let path = crate::config::expand(&path_str);
            if self.profiles.duplicate_exists(agent, &path, exclude) {
                Some("A profile with the same agent and path already exists".to_string())
            } else {
                None
            }
        };
        if let Some(err) = error {
            if let Some(form) = self.profile_form.as_mut() {
                form.error = Some(err);
            }
            return;
        }

        // Prompt directory creation if the config folder does not exist.
        let path = crate::config::expand(&path_str);
        if !path.is_dir() {
            self.dir_create_ok_focused = true; // Default focus to OK (creation is the natural workflow).
            self.mode = UiMode::ProfileDirConfirm;
            return;
        }
        self.commit_profile_form();
    }

    /// Saves the profile after validation. Returns the saved profile ID (None on failure).
    fn commit_profile_form(&mut self) -> Option<String> {
        let Some(form) = self.profile_form.as_ref() else {
            self.mode = UiMode::Table;
            return None;
        };
        let name = form.name.value.trim().to_string();
        let path_str = form.path.value.trim().to_string();
        let agent = Agent::all()[form.agent_idx.min(Agent::all().len() - 1)];
        let editing_id = form.editing_id.clone();
        let path = crate::config::expand(&path_str);
        // Since OAuth Token input fields were removed, preserve tokens in existing profiles, default to None for new profiles.
        let oauth_token = match &editing_id {
            Some(id) => self
                .profiles
                .profiles
                .iter()
                .find(|p| p.id == *id)
                .and_then(|p| p.oauth_token.clone()),
            None => None,
        };
        let saved_id = match editing_id {
            Some(id) => {
                if let Some(p) = self.profiles.profiles.iter_mut().find(|p| p.id == id) {
                    if !p.builtin {
                        p.agent = agent;
                    }
                    p.name = name.clone();
                    p.path = path;
                    p.oauth_token = oauth_token;
                }
                id
            }
            None => {
                let id = crate::profile::gen_id();
                let shortcut_count = self.profiles.numbered_profiles().len();
                let shortcut = (shortcut_count < crate::profile::MAX_PROFILE_SHORTCUTS)
                    .then_some((shortcut_count + 1) as u8);
                self.profiles.profiles.push(crate::profile::Profile {
                    id: id.clone(),
                    agent,
                    name: name.clone(),
                    path,
                    oauth_token,
                    active: shortcut.is_some(),
                    shortcut,
                    builtin: false,
                });
                id
            }
        };
        if let Err(e) = self.profiles.save() {
            if let Some(form) = self.profile_form.as_mut() {
                form.error = Some(format!("failed to save profiles.json: {e}"));
            }
            // Restore profile form mode since this might have been called from the directory confirmation modal.
            self.mode = UiMode::ProfileForm;
            return None;
        }

        self.profile_form = None;
        self.mode = UiMode::Table;
        // Immediately load sessions for the new/modified profile directory (rescan is cheap thanks to mtime caching).
        // Since usage and model catalogs queries require expensive PTY runs, only run incremental updates for the saved profile
        // (models are forcefully queried in case the config directory path has changed).
        self.refresh_sessions();
        self.start_usage_fetch_for(std::slice::from_ref(&saved_id));
        self.start_models_fetch_for(std::slice::from_ref(&saved_id), true);
        self.status_msg = Some(format!("Profile saved: {name}"));
        Some(saved_id)
    }

    /// Cancels config folder creation modal: returns to profile form, keeping inputs for correction.
    fn cancel_profile_dir_create(&mut self) {
        self.mode = UiMode::ProfileForm;
    }

    /// Confirms config folder creation: creates target directories, saves profile data,
    /// and requests launching the agent login sequence if applicable.
    fn confirm_profile_dir_create(&mut self) {
        let Some(form) = self.profile_form.as_ref() else {
            self.mode = UiMode::Table;
            return;
        };
        let path = crate::config::expand(form.path.value.trim());
        if let Err(e) = std::fs::create_dir_all(&path) {
            if let Some(form) = self.profile_form.as_mut() {
                form.error = Some(format!("failed to create folder: {e}"));
            }
            self.mode = UiMode::ProfileForm;
            return;
        }
        let Some(saved_id) = self.commit_profile_form() else {
            return;
        };
        // Custom Antigravity paths do not support environment variable overrides, rendering login launches meaningless.
        let (runnable, name) = self
            .profiles
            .find(&saved_id)
            .map(|p| {
                (
                    crate::profile::login_runnable(p.agent, &p.path),
                    p.name.clone(),
                )
            })
            .unwrap_or((false, String::new()));
        if runnable {
            self.login_request = Some(saved_id);
        } else {
            self.status_msg = Some(format!(
                "Profile saved: {name} — log in manually (custom config folder not supported)"
            ));
        }
    }

    /// Opens profile deletion confirmation modal. Blocked for built-in profiles.
    pub(crate) fn open_profile_delete(&mut self) {
        let Some(p) = self.profiles.profiles.get(self.profile_selected) else {
            return;
        };
        if p.builtin {
            self.show_message(
                " Cannot Delete ",
                vec![
                    format!("'{}' is a built-in profile.", p.name),
                    "Built-in profiles cannot be deleted.".to_string(),
                ],
                MessageKind::Warn,
            );
            return;
        }
        self.pending_profile_delete = Some(self.profile_selected);
        self.delete_ok_focused = false; // Default focus to Cancel (safer fallback).
        self.mode = UiMode::ProfileDeleteConfirm;
        self.status_msg = None;
    }

    fn cancel_profile_delete(&mut self) {
        self.pending_profile_delete = None;
        self.mode = UiMode::Table;
    }

    /// Confirms profile deletion: removes from active lists, filters, usage tracking, and cached sessions (directory preserved).
    fn confirm_profile_delete(&mut self) {
        let Some(idx) = self.pending_profile_delete.take() else {
            self.mode = UiMode::Table;
            return;
        };
        self.mode = UiMode::Table;
        if idx >= self.profiles.profiles.len() || self.profiles.profiles[idx].builtin {
            return;
        }
        let removed = self.profiles.profiles.remove(idx);
        if let Err(e) = self.profiles.save() {
            self.status_msg = Some(format!("failed to save profiles.json: {e}"));
        }
        self.sessions.retain(|s| s.profile_id != removed.id);
        self.filter.profile_ids.remove(&removed.id);
        self.usage.remove(&removed.id);
        self.models.remove(&removed.id);
        self.models.save().ok();
        self.rebuild_all_folders();
        self.recompute();
        self.profile_selected = self
            .profile_selected
            .min(self.profiles.profiles.len().saturating_sub(1));
        self.status_msg = Some(format!("Profile deleted: {} (folder kept)", removed.name));
    }

    /// Handles key inputs in the profile table view.
    pub fn on_key_profile_table(&mut self, key: crossterm::event::KeyEvent) {
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
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.open_quick_command(),
            // No session selection on this screen; shows a status message explaining why.
            KeyCode::Char('!') => self.open_quick_terminal(),
            // →: Switches to the session search view (simple transition, independent of selection).
            KeyCode::Right | KeyCode::Char('l') => self.switch_screen(Screen::Session),
            KeyCode::Up | KeyCode::Char('k') => self.move_profile_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_profile_selection(1),
            KeyCode::Home | KeyCode::Char('g') => self.profile_selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.profile_selected = self.profiles.profiles.len().saturating_sub(1);
            }
            KeyCode::Char(c @ '1'..='5') => self.assign_profile_slot(c as usize - '1' as usize),
            KeyCode::Char(' ') => self.toggle_selected_profile_shortcut(),
            // Ctrl+N: Opens the new session creation dialog (defaults to selected profile, empty directory).
            // No contextual variant here (no focused session), and SHIFT is excluded so a
            // Ctrl+Shift+N chord from an enhanced terminal is not captured as ordinary Ctrl+N.
            KeyCode::Char('n')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.open_new_session_modal(self.profile_selected, None, false, None);
            }
            KeyCode::Char('+') => self.open_profile_form(None),
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_profile_form(Some(self.profile_selected));
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_profile_delete();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.update_sessions_and_usage();
            }
            _ => {}
        }
    }

    /// Handles key inputs in the profile creation/edit form.
    pub fn on_key_profile_form(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(form) = self.profile_form.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => self.cancel_profile_form(),
            KeyCode::Tab | KeyCode::Down => form.focus_move(1),
            KeyCode::BackTab | KeyCode::Up => form.focus_move(-1),
            KeyCode::Enter => {
                if form.focus == FormFocus::Buttons {
                    if form.save_focused {
                        self.confirm_profile_form();
                    } else {
                        self.cancel_profile_form();
                    }
                }
            }
            _ => match form.focus {
                FormFocus::Agent => match key.code {
                    KeyCode::Left | KeyCode::Char('h') => form.cycle_agent(-1),
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => form.cycle_agent(1),
                    _ => {}
                },
                FormFocus::Buttons => {
                    if matches!(
                        key.code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
                    ) {
                        form.save_focused = !form.save_focused;
                    }
                }
                _ => {
                    let Some(input) = form.focused_input() else {
                        return;
                    };
                    match key.code {
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::CONTROL)
                                && !key.modifiers.contains(KeyModifiers::ALT)
                                && !key.modifiers.contains(KeyModifiers::SUPER) =>
                        {
                            input.insert_char(c);
                            form.error = None;
                        }
                        KeyCode::Backspace => input.backspace(),
                        KeyCode::Delete => input.delete(),
                        KeyCode::Left => input.move_left(),
                        KeyCode::Right => input.move_right(),
                        KeyCode::Home => input.home(),
                        KeyCode::End => input.end(),
                        _ => {}
                    }
                }
            },
        }
    }

    /// Handles key inputs in the profile deletion confirmation modal.
    pub fn on_key_profile_delete_confirm(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.delete_ok_focused = !self.delete_ok_focused;
            }
            KeyCode::Enter => {
                if self.delete_ok_focused {
                    self.confirm_profile_delete();
                } else {
                    self.cancel_profile_delete();
                }
            }
            KeyCode::Esc => self.cancel_profile_delete(),
            _ => {}
        }
    }

    /// Handles key inputs in the missing config folder creation confirmation modal.
    pub fn on_key_profile_dir_confirm(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.dir_create_ok_focused = !self.dir_create_ok_focused;
            }
            KeyCode::Enter => {
                if self.dir_create_ok_focused {
                    self.confirm_profile_dir_create();
                } else {
                    self.cancel_profile_dir_create();
                }
            }
            KeyCode::Esc => self.cancel_profile_dir_create(),
            _ => {}
        }
    }
}
