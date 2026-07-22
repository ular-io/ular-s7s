//! New Session key handling and launch logic: opening the dialog (with or
//! without source context), model-option construction, folder/project-directory
//! confirmation, and issuing the `NewSessionRequest`.

use super::state::{
    is_bare_project_name, ModelOption, NewSessionFocus, NewSessionRequest, NewSessionState,
    SessionContextRef,
};
use crate::models::{self, LastSelection};
use crate::ui::{App, Screen, TextInput, UiMode};
use std::fs;
use std::path::PathBuf;

impl App {
    pub(crate) fn cancel_project_dir_create(&mut self) {
        self.project_dir_pending = None;
        self.mode = UiMode::NewSession;
    }

    /// Confirms project folder creation: creates the pending folder under
    /// `config::projects_dir()` and starts the session in it with the profile/model/context
    /// already selected in the New Session dialog.
    pub(crate) fn confirm_project_dir_create(&mut self) {
        let Some(path) = self.project_dir_pending.take() else {
            self.mode = UiMode::NewSession;
            return;
        };
        // Validation/creation failures must land back in the dialog.
        self.mode = UiMode::NewSession;
        let cwd = match std::fs::create_dir_all(&path).and_then(|_| fs::canonicalize(&path)) {
            Ok(cwd) => cwd,
            Err(e) => {
                if let Some(state) = self.new_session.as_mut() {
                    state.error = Some(format!("failed to create folder: {e}"));
                }
                return;
            }
        };
        self.start_new_session_at(cwd);
    }

    /// Opens the new session creation dialog (accessible globally via Ctrl+N).
    ///
    /// `profile_idx`: Default index in `profiles` to select initially
    /// (corresponds to active session's profile in search/details screens, or active profile in profile screen).
    /// `initial_dir`: Initial value for the directory input field (resolves to active session's cwd,
    /// or empty when launched from the profile screen).
    pub(crate) fn open_new_session_modal(
        &mut self,
        profile_idx: usize,
        initial_dir: Option<String>,
        focus_ok: bool,
        context: Option<SessionContextRef>,
    ) {
        if self.profiles.profiles.is_empty() {
            self.status_msg = Some("No profiles available".to_string());
            return;
        }
        let profile_idx = profile_idx.min(self.profiles.profiles.len() - 1);
        let mut folders: Vec<PathBuf> = self
            .sessions
            .iter()
            .map(|s| s.cwd.clone())
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        folders.sort_unstable();
        folders.dedup();

        // Initial focus: OK button when requested (Session view), else Folder for profile
        // screens (empty path), otherwise the Profile dropdown.
        let focus = if focus_ok {
            NewSessionFocus::Buttons
        } else if initial_dir.is_none() {
            NewSessionFocus::Folder
        } else {
            NewSessionFocus::Profile
        };
        let (model_options, model_idx) = self.new_session_model_options(profile_idx);
        let mut state = NewSessionState {
            profile_idx,
            focus,
            dropdown_open: false,
            profile_cursor: profile_idx,
            model_options,
            model_idx,
            model_cursor: model_idx,
            input: TextInput::new(initial_dir.unwrap_or_default()),
            folders,
            ordered: Vec::new(),
            match_count: 0,
            folder_cursor: None,
            ok_focused: true,
            error: None,
            context,
        };
        state.reorder_folders();
        // If starting with Folder focused due to empty inputs, pre-open the dropdown list.
        if state.focus == NewSessionFocus::Folder {
            state.dropdown_open = true;
            state.folder_cursor = (!state.ordered.is_empty()).then_some(0);
        }
        self.new_session = Some(state);
        self.mode = UiMode::NewSession;
        self.status_msg = None;
    }

    /// Search/Details view helper: Opens dialog using targeted session's profile and cwd as defaults.
    /// Falls back to first profile and empty directory if session index is invalid.
    ///
    /// `with_context = true` (Ctrl+Shift+N / palette "New Session with Context")
    /// additionally captures the focused session as an immutable context source;
    /// it requires a valid focused session and keeps the current screen otherwise.
    pub(crate) fn open_new_session_modal_for_session(
        &mut self,
        session_idx: Option<usize>,
        with_context: bool,
    ) {
        let session = session_idx.and_then(|idx| self.sessions.get(idx));
        if with_context && session.is_none() {
            self.status_msg = Some("Select a session first".to_string());
            return;
        }
        let context = if with_context {
            session.map(|s| SessionContextRef {
                agent: s.agent,
                profile_id: s.profile_id.clone(),
                session_id: s.id.clone(),
                title: s.title(),
            })
        } else {
            None
        };
        let (profile_idx, dir) = session
            .map(|s| {
                let profile_idx = self
                    .profiles
                    .profiles
                    .iter()
                    .position(|p| p.id == s.profile_id)
                    .unwrap_or(0);
                let dir =
                    (!s.cwd.as_os_str().is_empty()).then(|| s.cwd.to_string_lossy().into_owned());
                (profile_idx, dir)
            })
            .unwrap_or((0, None));
        // Session view opens with the OK button focused for a quick start; other screens
        // (Detail) keep the Profile dropdown focused.
        let focus_ok = self.screen == Screen::Session;
        self.open_new_session_modal(profile_idx, dir, focus_ok, context);
    }

    fn cancel_new_session(&mut self) {
        self.new_session = None;
        self.mode = UiMode::Table;
    }

    /// Generates model dropdown options and default index for the specified profile.
    ///
    /// - Index 0 is always reserved for Default (no --model flag injected).
    /// - Lists populated from model caches (extra agy profiles share the default profile cache),
    ///   falling back to hardcoded configurations if empty (claude aliases only).
    /// - Initial selection priority: the user's last launched pick (`last_selected`) →
    ///   the CLI-configured default (`default_model`) → a missing placeholder. A last pick
    ///   that is no longer in the fetched list is skipped (falls through to the CLI default);
    ///   the placeholder (which disables OK) only appears when the CLI default itself is missing.
    fn new_session_model_options(&self, profile_idx: usize) -> (Vec<ModelOption>, usize) {
        let default_only = || {
            (
                vec![ModelOption {
                    value: None,
                    label: "Default".to_string(),
                    note: "run without --model".to_string(),
                    missing: false,
                }],
                0,
            )
        };
        let Some(profile) = self.profiles.profiles.get(profile_idx) else {
            return default_only();
        };
        let (entries, default_model, last_selected) = match self.models.for_profile(profile) {
            Some(pm) => (
                pm.models.clone(),
                pm.default_model.clone(),
                pm.last_selected.clone(),
            ),
            None => (models::fallback_models(profile.agent), None, None),
        };
        let mut options = vec![ModelOption {
            value: None,
            label: "Default".to_string(),
            note: match &default_model {
                Some(d) => format!("run without --model (currently {d})"),
                None => "run without --model".to_string(),
            },
            missing: false,
        }];
        options.extend(entries.iter().map(|m| ModelOption {
            value: Some(m.value.clone()),
            label: m.label.clone(),
            note: m.note.clone(),
            missing: false,
        }));
        // 1) The last launched pick wins when it is still resolvable.
        if let Some(last) = &last_selected {
            match last {
                LastSelection::Default => return (options, 0),
                LastSelection::Model(v) => {
                    if let Some(pos) = entries.iter().position(|m| &m.value == v) {
                        return (options, pos + 1);
                    }
                    // Stale pick (removed after a CLI upgrade): fall through to the CLI default.
                }
            }
        }
        // 2) CLI-configured default, else 3) a missing placeholder that disables OK.
        let idx = match default_model {
            None => 0,
            Some(d) => match entries.iter().position(|m| m.value == d) {
                Some(pos) => pos + 1,
                None => {
                    // Configured default model missing: insert placeholder and disable OK button
                    // to prevent launching with typos or outdated configuration values silently.
                    options.insert(
                        1,
                        ModelOption {
                            value: Some(d.clone()),
                            label: d,
                            note: "not in the fetched model list".to_string(),
                            missing: true,
                        },
                    );
                    1
                }
            },
        };
        (options, idx)
    }

    /// Re-evaluates model dropdown options when selected profile changes.
    fn refresh_new_session_model_options(&mut self) {
        let Some(profile_idx) = self.new_session.as_ref().map(|s| s.profile_idx) else {
            return;
        };
        let (options, idx) = self.new_session_model_options(profile_idx);
        if let Some(state) = self.new_session.as_mut() {
            state.model_options = options;
            state.model_idx = idx;
            state.model_cursor = idx;
        }
    }

    /// Confirms session dialog (Enter on closed dropdown): requests starting a new session
    /// in the selected profile and directory. Displays error if directory input is empty.
    pub(crate) fn confirm_new_session(&mut self) {
        let Some(state) = self.new_session.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        let raw = state.input.value.trim();
        if raw.is_empty() {
            state.error = Some("Select a folder first".to_string());
            return;
        }
        // Bare project name: resolved under `config::projects_dir()` only (never relative to
        // the process cwd). A missing folder is a creation candidate, not an error.
        let bare = is_bare_project_name(raw);
        let path = if bare {
            crate::config::projects_dir().join(raw)
        } else {
            resolve_input_path(raw)
        };
        if bare && !path.exists() {
            self.project_dir_pending = Some(path);
            self.dir_create_ok_focused = true; // Default focus to Create (creation is the natural workflow).
            self.mode = UiMode::ProjectDirConfirm;
            return;
        }
        let cwd = match fs::canonicalize(&path) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                state.error = Some("Path is not a directory".to_string());
                return;
            }
            Err(err) => {
                state.error = Some(format!("Cannot open path: {err}"));
                return;
            }
        };
        self.start_new_session_at(cwd);
    }

    /// Validates profile/model/context and issues the `NewSessionRequest` for a resolved
    /// `cwd`. Validation failures surface on the dialog state (mode must be NewSession).
    fn start_new_session_at(&mut self, cwd: PathBuf) {
        let Some(state) = self.new_session.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };

        // Prevent execution if placeholder missing model is selected.
        let model_option = state.model_options.get(state.model_idx);
        if model_option.is_some_and(|o| o.missing) {
            state.error = Some("Model not available — select another model".to_string());
            return;
        }
        let model = model_option.and_then(|o| o.value.clone());

        let Some(profile) = self.profiles.profiles.get(state.profile_idx) else {
            state.error = Some("Profile no longer exists".to_string());
            return;
        };
        let profile_id = profile.id.clone();

        // Contextual launch: the source must still exist at OK time. Abort with an
        // error instead of launching an agent whose bootstrap command would fail
        // (and never fall back to another source profile).
        let context = state.context.clone();
        if let Some(ctx) = &context {
            let source_exists = self
                .sessions
                .iter()
                .any(|s| s.agent == ctx.agent && s.id == ctx.session_id);
            if !source_exists {
                if let Some(state) = self.new_session.as_mut() {
                    state.error =
                        Some("Source session not found — it may have been deleted".to_string());
                }
                return;
            }
            if self.profiles.find(&ctx.profile_id).is_none() {
                if let Some(state) = self.new_session.as_mut() {
                    state.error = Some("Source profile not found".to_string());
                }
                return;
            }
        }

        // Remember this pick as the profile's next default (last_selected → CLI default →
        // placeholder). Persist immediately; the profile has a cached catalog whenever the
        // dialog could offer real models, so set_last_selected finds an entry to hold it.
        let selection = match &model {
            Some(v) => LastSelection::Model(v.clone()),
            None => LastSelection::Default,
        };
        self.models.set_last_selected(&profile_id, selection);
        self.models.save().ok();

        self.new_session_request = Some(NewSessionRequest {
            profile_id,
            cwd,
            model,
            context,
        });
        self.new_session = None;
        self.mode = UiMode::Table;
    }

    pub fn on_key_project_dir_confirm(&mut self, key: crossterm::event::KeyEvent) {
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
                    self.confirm_project_dir_create();
                } else {
                    self.cancel_project_dir_create();
                }
            }
            KeyCode::Esc => self.cancel_project_dir_create(),
            _ => {}
        }
    }

    /// Handles key inputs in the new session creation dialog (profile, model, and folder dropdown controls).
    ///
    /// - Tab/Shift+Tab: Cycles focus through Profile → Model → Folder → OK → Cancel.
    ///   Closes open dropdowns, committing the active cursor selection first (commit-then-move).
    /// - ↑/↓: Closed dropdowns → rotates focus vertically (wraps at bounds like Tab,
    ///   enters buttons row focusing OK first). Cycles through dropdown items if open.
    /// - Enter: closed -> opens dropdown list (if Buttons, executes: OK=start, Cancel=cancel) /
    ///   open -> commits cursor selection and closes dropdown.
    /// - Space: open -> commits selection but keeps dropdown open (folders are instantly reflected in input).
    /// - →: opens Folder dropdown if closed / moves text cursor right if open. For Profile/Model,
    ///   opens dropdown if closed. On Buttons focus, moves between OK ↔ Cancel using ←/→.
    /// - Esc: open -> closes dropdown without selection / closed -> cancels session dialog.
    ///
    /// Reconstructs model dropdown items based on the active profile's agent type
    /// when the profile selection is committed (via Enter / Space / Tab).
    pub fn on_key_new_session(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let profile_count = self.profiles.profiles.len();
        let prev_profile_idx = self.new_session.as_ref().map(|s| s.profile_idx);
        let Some(state) = self.new_session.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                if state.dropdown_open {
                    state.close_dropdown();
                } else {
                    self.cancel_new_session();
                }
            }
            KeyCode::Tab => state.move_focus(1),
            KeyCode::BackTab => state.move_focus(-1),
            KeyCode::Enter => {
                if state.dropdown_open {
                    // Enter key on open dropdown: selects the active item and closes dropdown.
                    match state.focus {
                        NewSessionFocus::Profile => {
                            state.profile_idx = state.profile_cursor;
                            state.close_dropdown();
                        }
                        NewSessionFocus::Model => {
                            state.model_idx = state.model_cursor;
                            state.close_dropdown();
                        }
                        NewSessionFocus::Folder => {
                            // If cursor is on a listed item, select it; otherwise, commit the text input directly.
                            if let Some(i) = state.folder_cursor {
                                state.apply_folder_to_input(i);
                            }
                            state.close_dropdown();
                        }
                        // Dropdown cannot be opened while focused on buttons (defensive close).
                        NewSessionFocus::Buttons => state.close_dropdown(),
                    }
                } else {
                    // Enter key on closed state: executes the control's default action.
                    match state.focus {
                        // Dropdown: opens selection list (replaces prior ↓ action).
                        NewSessionFocus::Profile => {
                            if profile_count > 0 {
                                state.profile_cursor = state.profile_idx.min(profile_count - 1);
                                state.dropdown_open = true;
                            }
                        }
                        NewSessionFocus::Model => {
                            state.model_cursor = state
                                .model_idx
                                .min(state.model_options.len().saturating_sub(1));
                            state.dropdown_open = true;
                        }
                        NewSessionFocus::Folder => {
                            state.dropdown_open = true;
                            state.folder_cursor = (!state.ordered.is_empty()).then_some(0);
                        }
                        // Buttons: executes button action.
                        NewSessionFocus::Buttons => {
                            if state.ok_focused {
                                self.confirm_new_session();
                            } else {
                                self.cancel_new_session();
                            }
                        }
                    }
                }
            }
            // Arrow keys ↑/↓ on closed dropdowns move focus between controls.
            KeyCode::Down if !state.dropdown_open => state.move_focus_vertical(1),
            KeyCode::Up if !state.dropdown_open => state.move_focus_vertical(-1),
            _ => match state.focus {
                NewSessionFocus::Profile => match key.code {
                    // Dropdowns are guaranteed to be open here (closed states handled above).
                    KeyCode::Down => {
                        if profile_count > 0 {
                            // Cycle from bottom to top index on Down arrow.
                            state.profile_cursor = if state.profile_cursor + 1 >= profile_count {
                                0
                            } else {
                                state.profile_cursor + 1
                            };
                        }
                    }
                    KeyCode::Up => {
                        if profile_count > 0 {
                            // Cycle from top to bottom index on Up arrow (keeps dropdown open).
                            state.profile_cursor = if state.profile_cursor == 0 {
                                profile_count - 1
                            } else {
                                state.profile_cursor - 1
                            };
                        }
                    }
                    // → key (closed): opens dropdown (places cursor on active selection); no action if already open.
                    KeyCode::Right if !state.dropdown_open => {
                        if profile_count > 0 {
                            state.profile_cursor = state.profile_idx.min(profile_count - 1);
                            state.dropdown_open = true;
                        }
                    }
                    KeyCode::Char(' ') => {
                        state.profile_idx = state.profile_cursor;
                    }
                    _ => {}
                },
                NewSessionFocus::Model => match key.code {
                    // Dropdowns are guaranteed to be open here (closed states handled above).
                    KeyCode::Down => {
                        let n = state.model_options.len();
                        if n > 0 {
                            // Cycle from bottom to top index on Down arrow.
                            state.model_cursor = if state.model_cursor + 1 >= n {
                                0
                            } else {
                                state.model_cursor + 1
                            };
                        }
                    }
                    KeyCode::Up => {
                        let n = state.model_options.len();
                        if n > 0 {
                            // Cycle from top to bottom index on Up arrow (keeps dropdown open).
                            state.model_cursor = if state.model_cursor == 0 {
                                n - 1
                            } else {
                                state.model_cursor - 1
                            };
                        }
                    }
                    // → key (closed): opens dropdown (places cursor on active selection); no action if already open.
                    KeyCode::Right if !state.dropdown_open => {
                        state.model_cursor = state
                            .model_idx
                            .min(state.model_options.len().saturating_sub(1));
                        state.dropdown_open = true;
                    }
                    KeyCode::Char(' ') => {
                        state.model_idx = state.model_cursor;
                    }
                    _ => {}
                },
                NewSessionFocus::Folder => match key.code {
                    // Arrow keys ↑/↓ reach here only when the dropdown is open (moves cursor in list).
                    KeyCode::Down => {
                        // Cycle from bottom to top index on Down arrow.
                        state.folder_cursor = match state.folder_cursor {
                            None => (!state.ordered.is_empty()).then_some(0),
                            Some(i) if i + 1 >= state.ordered.len() => Some(0),
                            Some(i) => Some(i + 1),
                        };
                    }
                    KeyCode::Up => {
                        // Cycle from top (first item / text highlight) to bottom index on Up arrow (keeps dropdown open).
                        let last = state.ordered.len().saturating_sub(1);
                        state.folder_cursor = match state.folder_cursor {
                            Some(i) if i > 0 => Some(i - 1),
                            _ => (!state.ordered.is_empty()).then_some(last),
                        };
                    }
                    // Space key: updates text inputs immediately while keeping the dropdown list open.
                    // (handled as literal space character input if the list cursor is not set)
                    KeyCode::Char(' ') if state.dropdown_open && state.folder_cursor.is_some() => {
                        if let Some(i) = state.folder_cursor {
                            state.apply_folder_to_input(i);
                        }
                    }
                    // → key: opens dropdown list if closed; moves text cursor right if open.
                    KeyCode::Right => {
                        if !state.dropdown_open {
                            state.dropdown_open = true;
                            state.folder_cursor = (!state.ordered.is_empty()).then_some(0);
                        } else {
                            state.input.move_right();
                        }
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                            && !key.modifiers.contains(KeyModifiers::SUPER) =>
                    {
                        state.input.insert_char(c);
                        state.on_input_edited();
                    }
                    KeyCode::Backspace => {
                        state.input.backspace();
                        state.on_input_edited();
                    }
                    KeyCode::Delete => {
                        state.input.delete();
                        state.on_input_edited();
                    }
                    KeyCode::Left => state.input.move_left(),
                    KeyCode::Home => state.input.home(),
                    KeyCode::End => state.input.end(),
                    _ => {}
                },
                NewSessionFocus::Buttons => {
                    if matches!(
                        key.code,
                        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l')
                    ) {
                        state.ok_focused = !state.ok_focused;
                    }
                }
            },
        }
        // Reconstruct model catalogs if selected profile changed (committed via Enter / Space / Tab).
        if let Some(prev) = prev_profile_idx {
            if self
                .new_session
                .as_ref()
                .is_some_and(|s| s.profile_idx != prev)
            {
                self.refresh_new_session_model_options();
            }
        }
    }
}

fn resolve_input_path(raw: &str) -> PathBuf {
    let path = crate::config::expand(raw);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
