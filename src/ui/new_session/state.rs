//! New Session dialog state, focus model, model/source options, and the pure
//! transition helpers driven by key handling.

use crate::model::Agent;
use crate::ui::TextInput;
use std::path::PathBuf;

/// Focused control in the new session dialog (cycled via Tab / Shift+Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewSessionFocus {
    /// Profile combo box (untypeable).
    Profile,
    /// Model combo box (untypeable). Available options depend on the selected profile's agent.
    Model,
    /// Folder combo box (typeable).
    Folder,
    /// Bottom OK/Cancel buttons row (navigated via ←/→).
    Buttons,
}

/// An option in the model dropdown of the new session dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// Value passed to CLI `--model`. None = Default (omits the flag, falling back to CLI default).
    pub value: Option<String>,
    pub label: String,
    /// Supplementary note (rendered in dim color).
    pub note: String,
    /// Placeholder indicating the configured default model is missing in the queried list.
    /// While selected, the OK button is disabled, requiring the user to choose another option.
    pub missing: bool,
}

/// Immutable reference to the source session captured when a contextual New
/// Session dialog opens. Identity is cloned (not a session index) because
/// indices are unstable after refresh/re-sorting. Changing the target Profile,
/// Model, or Folder never mutates this reference, enabling cross-agent and
/// cross-project use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContextRef {
    pub agent: Agent,
    pub profile_id: String,
    pub session_id: String,
    pub title: String,
}

/// New session dialog state (managing profile, model, and folder dropdowns).
///
/// Dropdown behavior: Enter/→ opens a closed dropdown (typing in Folder also opens it automatically).
/// Pressing Enter while open selects the value and closes; Space selects without closing; Esc cancels and closes.
/// Navigation keys ↑/↓ cycle through open items.
pub struct NewSessionState {
    /// Resolved selected profile index in the profiles list (changed via Enter/Space in dropdown).
    pub profile_idx: usize,
    /// Currently focused control.
    pub focus: NewSessionFocus,
    /// Whether the focused control's dropdown is open (at most one dropdown open at a time).
    pub dropdown_open: bool,
    /// Profile dropdown cursor (valid while open, referencing profiles index).
    pub profile_cursor: usize,
    /// Model dropdown options (0 = Default). Reconstructed only on opening the dialog or changing profiles
    /// (background model queries do not hot-swap open dropdown options to prevent misclicks from cursor jumps).
    pub model_options: Vec<ModelOption>,
    /// Resolved selected index in `model_options`.
    pub model_idx: usize,
    /// Model dropdown cursor (valid while open).
    pub model_cursor: usize,
    /// Folder direct text input path. Filled with the full path when a dropdown item is selected.
    pub input: TextInput,
    /// Existing workspace folders (full paths) discovered across all sessions.
    pub folders: Vec<PathBuf>,
    /// Sorted indices of `folders` prioritizing matches. The first `match_count` items are matches
    /// for the input string, followed by non-matching items (not hidden).
    pub ordered: Vec<usize>,
    /// Number of matching items at the head of `ordered` (used to distinguish colors in render).
    pub match_count: usize,
    /// Folder dropdown list cursor (index in `ordered`). None indicates highlight remains in the input field
    /// (e.g. immediately after typing auto-opens it), requiring ↓ key press to focus the list.
    pub folder_cursor: Option<usize>,
    /// Focused button in the button row: OK (true) or Cancel (false). OK and Cancel act as separate
    /// tab stops (cycling: Profile -> Folder -> OK -> Cancel).
    pub ok_focused: bool,
    /// Validation error message.
    pub error: Option<String>,
    /// Immutable source-session reference for "New Session with Context".
    /// None = ordinary New Session (identical behavior to before).
    pub context: Option<SessionContextRef>,
}

/// Request to start a new session. Processed by the main loop after releasing TUI.
#[derive(Debug, Clone)]
pub struct NewSessionRequest {
    pub profile_id: String,
    pub cwd: PathBuf,
    /// Selected model (appended via `--model`). None = Default (omits the flag).
    pub model: Option<String>,
    /// Source session used as historical context (drives bootstrap prompt injection).
    pub context: Option<SessionContextRef>,
}

impl NewSessionState {
    /// Tab/Shift+Tab: Cycles focus through Profile → Model → Folder → OK → Cancel (driven by delta).
    /// OK and Cancel act as independent tab stops. Commits current list cursor selections
    /// before closing dropdowns to prevent inadvertent data loss. Use Esc to cancel selection.
    pub(crate) fn move_focus(&mut self, delta: isize) {
        if self.dropdown_open {
            match self.focus {
                NewSessionFocus::Profile => self.profile_idx = self.profile_cursor,
                NewSessionFocus::Model => self.model_idx = self.model_cursor,
                NewSessionFocus::Folder => {
                    if let Some(i) = self.folder_cursor {
                        self.apply_folder_to_input(i);
                    }
                }
                NewSessionFocus::Buttons => {}
            }
            self.close_dropdown();
        }
        // Tab stop order: 0=Profile, 1=Model, 2=Folder, 3=OK, 4=Cancel.
        let cur = match self.focus {
            NewSessionFocus::Profile => 0isize,
            NewSessionFocus::Model => 1,
            NewSessionFocus::Folder => 2,
            NewSessionFocus::Buttons => {
                if self.ok_focused {
                    3
                } else {
                    4
                }
            }
        };
        match (cur + delta).rem_euclid(5) {
            0 => self.focus = NewSessionFocus::Profile,
            1 => self.focus = NewSessionFocus::Model,
            2 => self.focus = NewSessionFocus::Folder,
            3 => {
                self.focus = NewSessionFocus::Buttons;
                self.ok_focused = true;
            }
            _ => {
                self.focus = NewSessionFocus::Buttons;
                self.ok_focused = false;
            }
        }
    }

    /// ↑/↓ (closed dropdowns): Moves focus vertically between rows.
    /// Rotates through Profile (0) ↔ Model (1) ↔ Folder (2) ↔ Buttons (3), wrapping at
    /// bounds like Tab. Unlike Tab, the button row is a single vertical stop (OK/Cancel
    /// share a row); entering it focuses OK first.
    pub(crate) fn move_focus_vertical(&mut self, delta: isize) {
        let cur = match self.focus {
            NewSessionFocus::Profile => 0isize,
            NewSessionFocus::Model => 1,
            NewSessionFocus::Folder => 2,
            NewSessionFocus::Buttons => 3,
        };
        let was_buttons = matches!(self.focus, NewSessionFocus::Buttons);
        self.focus = match (cur + delta).rem_euclid(4) {
            0 => NewSessionFocus::Profile,
            1 => NewSessionFocus::Model,
            2 => NewSessionFocus::Folder,
            _ => NewSessionFocus::Buttons,
        };
        // Focuses OK first when entering button row (Down key).
        if matches!(self.focus, NewSessionFocus::Buttons) && !was_buttons {
            self.ok_focused = true;
        }
    }

    /// Closes dropdown without committing active selections (shared between Esc and focus changes).
    pub(crate) fn close_dropdown(&mut self) {
        self.dropdown_open = false;
        self.folder_cursor = None;
    }

    /// Post-edit helper for folder text input: clears errors, auto-opens dropdown (keeps highlight in input), and reorders matches.
    pub(crate) fn on_input_edited(&mut self) {
        self.error = None;
        self.dropdown_open = true;
        self.folder_cursor = None;
        self.reorder_folders();
    }

    /// Reorders folder options placing input matches at the top. Non-matching items are appended
    /// to the tail rather than hidden, preserving alphabetical order within each subset.
    pub(crate) fn reorder_folders(&mut self) {
        let q = crate::normalize::nfc_lower(self.input.value.trim());
        let mut matched = Vec::new();
        let mut rest = Vec::new();
        for (i, path) in self.folders.iter().enumerate() {
            let is_match = if q.is_empty() {
                true
            } else {
                let full = crate::normalize::nfc_lower(&path.to_string_lossy());
                let name = path
                    .file_name()
                    .map(|n| crate::normalize::nfc_lower(&n.to_string_lossy()))
                    .unwrap_or_default();
                full.contains(&q) || name.contains(&q)
            };
            if is_match {
                matched.push(i);
            } else {
                rest.push(i);
            }
        }
        self.match_count = matched.len();
        matched.extend(rest);
        self.ordered = matched;
        if let Some(c) = self.folder_cursor {
            self.folder_cursor = if self.ordered.is_empty() {
                None
            } else {
                Some(c.min(self.ordered.len() - 1))
            };
        }
    }

    /// Syncs the full path of the selected folder dropdown option (index in `ordered`) back to the input text box.
    /// Keeps the list cursor tracking the same folder even after sorting lists.
    pub(crate) fn apply_folder_to_input(&mut self, ordered_pos: usize) {
        let Some(&folder_i) = self.ordered.get(ordered_pos) else {
            return;
        };
        let Some(path) = self.folders.get(folder_i) else {
            return;
        };
        self.input = TextInput::new(path.to_string_lossy().into_owned());
        self.error = None;
        self.reorder_folders();
        self.folder_cursor = self.ordered.iter().position(|&i| i == folder_i);
    }
}

/// Bare project name: no path separator and no `~` prefix (dots and spaces allowed).
/// Such input resolves under `config::projects_dir()` instead of the process cwd.
pub(crate) fn is_bare_project_name(raw: &str) -> bool {
    !raw.starts_with('~') && !raw.chars().any(std::path::is_separator)
}
