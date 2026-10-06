//! Workspace state: the Session screen's workspace pane (the `[NEW WORKSPACE]`
//! row flag, list scroll, and membership cache) and the edit dialog (a draft of one workspace,
//! its text fields, its `Folders ▾` combo with the stable folder rows and search of
//! its open checklist, and the button row). The workspace list cursor is not
//! stored here — it is `WorkspaceStore::active`, so the list row the user sits
//! on is always the scope the session lists show.

use crate::ui::TextInput;
use crate::workspaces::Workspace;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

/// Dialog rows above the buttons: three text fields and the folder combo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceField {
    Name,
    Folders,
    Includes,
    Excludes,
}

/// Dialog rows in display order.
pub(crate) const DIALOG_FIELDS: [WorkspaceField; 4] = [
    WorkspaceField::Name,
    WorkspaceField::Folders,
    WorkspaceField::Includes,
    WorkspaceField::Excludes,
];

/// The workspace edit dialog (`UiMode::WorkspaceEdit`). Nothing reaches the
/// store until Save: Cancel/Esc drop the draft, and a new workspace exists only
/// once saved.
pub struct WorkspaceDialog {
    /// Copy being edited. Its includes/excludes follow the inputs so the match
    /// count stays current; its name is taken from `name` on Save.
    pub(crate) draft: Workspace,
    /// Opened from `[NEW WORKSPACE]` / `+`: Save adds it and opens it.
    pub created: bool,
    pub name: TextInput,
    pub includes: TextInput,
    pub excludes: TextInput,
    /// Row cursor, an index into `DIALOG_FIELDS`. Ignored while `on_buttons`.
    pub cursor: usize,
    /// The Save/Cancel row has focus.
    pub on_buttons: bool,
    /// Save (true) or Cancel (false) within the button row.
    pub save_focused: bool,
    /// Cursor of the open folder checklist, which takes every key while open:
    /// row 0 is the fixed `[ALL FOLDERS]` row, row `n` is `visible[n - 1]`.
    /// `None` while the `Folders ▾` combo is closed.
    pub folder_list: Option<usize>,
    /// Folder rows in an order captured each time the checklist opens (the
    /// draft's selected folders first, then the rest by latest activity).
    /// Toggling does not reorder, so the cursor stays on the row it toggled.
    pub folders: Vec<PathBuf>,
    /// Sessions per folder over every session, captured with `folders`; a
    /// stored folder with no session left is absent and reads 0.
    pub folder_counts: HashMap<PathBuf, usize>,
    /// Checklist search: typing goes here while the checklist is open. Empty
    /// whenever it opens.
    pub folder_query: TextInput,
    /// Indices into `folders` matching `folder_query`, in `folders` order.
    pub visible: Vec<usize>,
    /// Sessions the draft matches, over every session.
    pub matching: usize,
    /// Why the last Save was refused; shown on the notice line until the next edit.
    pub error: Option<String>,
    /// Checklist viewport offset, adjusted during render to keep the cursor visible.
    pub folder_scroll: Cell<usize>,
}

impl WorkspaceDialog {
    pub(crate) fn new(draft: Workspace, created: bool) -> Self {
        let name = if created {
            // The suggested name is selected so typing replaces it.
            TextInput::selected(draft.name.clone())
        } else {
            TextInput::new(draft.name.clone())
        };
        Self {
            includes: TextInput::new(draft.includes.clone()),
            excludes: TextInput::new(draft.excludes.clone()),
            name,
            draft,
            created,
            cursor: 0,
            on_buttons: false,
            save_focused: true,
            folder_list: None,
            folders: Vec::new(),
            folder_counts: HashMap::new(),
            folder_query: TextInput::new(String::new()),
            visible: Vec::new(),
            matching: 0,
            error: None,
            folder_scroll: Cell::new(0),
        }
    }

    /// Checklist rows: the fixed `[ALL FOLDERS]` row and the visible folders.
    pub(crate) fn list_rows(&self) -> usize {
        1 + self.visible.len()
    }

    /// Recomputes `visible` from `folder_query`. Every whitespace-separated
    /// word must occur in the full path or the displayed label, ignoring case.
    pub(crate) fn rebuild_visible(&mut self) {
        let words: Vec<String> = crate::normalize::nfc_lower(&self.folder_query.value)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        self.visible = self
            .folders
            .iter()
            .enumerate()
            .filter(|(_, path)| {
                if words.is_empty() {
                    return true;
                }
                let full = crate::normalize::nfc_lower(&path.to_string_lossy());
                let label = crate::normalize::nfc_lower(&folder_display_label(path));
                words.iter().all(|w| full.contains(w) || label.contains(w))
            })
            .map(|(i, _)| i)
            .collect();
    }

    /// Visible folder rows, in display order.
    pub(crate) fn visible_folders(&self) -> impl Iterator<Item = &PathBuf> {
        self.visible.iter().filter_map(|&i| self.folders.get(i))
    }

    /// Row under the cursor, if it is not on the buttons.
    pub(crate) fn cursor_field(&self) -> Option<WorkspaceField> {
        if self.on_buttons {
            return None;
        }
        DIALOG_FIELDS.get(self.cursor).copied()
    }

    /// Folder under the open checklist's cursor (`None` on `[ALL FOLDERS]`).
    pub(crate) fn cursor_folder(&self) -> Option<&PathBuf> {
        self.folder_list
            .and_then(|row| row.checked_sub(1))
            .and_then(|i| self.visible.get(i))
            .and_then(|&i| self.folders.get(i))
    }

    /// Text input under the cursor: the open checklist's search, or a text
    /// field. The closed combo and the buttons own no text.
    pub(crate) fn cursor_input(&mut self) -> Option<&mut TextInput> {
        if self.folder_list.is_some() {
            return Some(&mut self.folder_query);
        }
        match self.cursor_field() {
            Some(WorkspaceField::Name) => Some(&mut self.name),
            Some(WorkspaceField::Includes) => Some(&mut self.includes),
            Some(WorkspaceField::Excludes) => Some(&mut self.excludes),
            Some(WorkspaceField::Folders) | None => None,
        }
    }

    /// Text field input; `None` for the folder combo.
    pub(crate) fn input(&self, field: WorkspaceField) -> Option<&TextInput> {
        match field {
            WorkspaceField::Name => Some(&self.name),
            WorkspaceField::Includes => Some(&self.includes),
            WorkspaceField::Excludes => Some(&self.excludes),
            WorkspaceField::Folders => None,
        }
    }

    /// Selected folders in checklist order. A stored folder the rows lack
    /// (only before the first refresh) follows in stored order.
    pub(crate) fn selected_folders(&self) -> Vec<&PathBuf> {
        let mut out: Vec<&PathBuf> = self
            .folders
            .iter()
            .filter(|f| self.draft.has_folder(f))
            .collect();
        out.extend(
            self.draft
                .folders
                .iter()
                .filter(|f| !self.folders.contains(f)),
        );
        out
    }
}

#[derive(Default)]
pub struct WorkspaceScreenState {
    /// Lazy membership over the current session vector and saved workspaces.
    /// Session replacement/removal and saved workspace changes invalidate it;
    /// ordinary filters and scope navigation reuse it.
    pub(crate) membership: RefCell<Option<Vec<bool>>>,
    /// The workspace pane cursor is on `[NEW WORKSPACE]`, below the stored
    /// workspaces. "All" is open meanwhile, and the flag counts only while it is.
    pub new_row: bool,
    /// Present while `UiMode::WorkspaceEdit`.
    pub dialog: Option<WorkspaceDialog>,
    /// Pane viewport offset, adjusted during render to keep the cursor visible.
    pub list_scroll: Cell<usize>,
}

/// Bare basename of a folder row (`[SCRATCH]` for the scratch workspace).
pub(crate) fn folder_display_label(folder: &std::path::Path) -> String {
    let basename = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.to_string_lossy().into_owned());
    crate::scratch::folder_label(folder, &basename).to_string()
}
