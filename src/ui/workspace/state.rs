//! Workspace state: the Session screen's workspace pane (the `[NEW WORKSPACE]`
//! row flag and list scroll) and the edit dialog (a draft of one workspace,
//! its text fields, its stable folder rows and their search, and the button
//! row). The workspace list cursor is not stored here — it is
//! `WorkspaceStore::active`, so the list row the user sits on is always the
//! scope the session lists show.

use crate::ui::TextInput;
use crate::workspaces::Workspace;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;

/// Editable text attributes of a workspace (the dialog's first rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceField {
    Name,
    Includes,
    Excludes,
}

/// Dialog rows before the folder rows, in display order.
pub(crate) const DIALOG_FIELDS: [WorkspaceField; 3] = [
    WorkspaceField::Name,
    WorkspaceField::Includes,
    WorkspaceField::Excludes,
];

/// Dialog row of the folder search, right above the folder rows.
pub(crate) const SEARCH_ROW: usize = DIALOG_FIELDS.len();
/// Dialog row of the first visible folder.
pub(crate) const FIRST_FOLDER_ROW: usize = SEARCH_ROW + 1;

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
    /// Row cursor: `0..DIALOG_FIELDS.len()` are fields, `SEARCH_ROW` is the
    /// folder search, and from `FIRST_FOLDER_ROW` on the rows index `visible`.
    /// Ignored while `on_buttons`.
    pub cursor: usize,
    /// The Save/Cancel row has focus.
    pub on_buttons: bool,
    /// Save (true) or Cancel (false) within the button row.
    pub save_focused: bool,
    /// Field row last edited (Name until one is): `←` on a folder row returns
    /// there, to the left column.
    pub last_field: usize,
    /// Folder rows in an order captured when the dialog opens (the draft's
    /// selected folders first, then the rest by latest activity). Toggling does
    /// not reorder, so the cursor stays on the row it toggled.
    pub folders: Vec<PathBuf>,
    /// Sessions per folder over every session, captured with `folders`; a
    /// stored folder with no session left is absent and reads 0.
    pub folder_counts: HashMap<PathBuf, usize>,
    /// Folder search, typed directly while the cursor is on `SEARCH_ROW`.
    pub folder_query: TextInput,
    /// Indices into `folders` matching `folder_query`, in `folders` order.
    pub visible: Vec<usize>,
    /// Sessions the draft matches, over every session.
    pub matching: usize,
    /// Why the last Save was refused; shown on the notice line until the next edit.
    pub error: Option<String>,
    /// Folder viewport offset, adjusted during render to keep the cursor visible.
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
            last_field: 0,
            folders: Vec::new(),
            folder_counts: HashMap::new(),
            folder_query: TextInput::new(String::new()),
            visible: Vec::new(),
            matching: 0,
            error: None,
            folder_scroll: Cell::new(0),
        }
    }

    /// Cursor rows: the fields, the search row, and the visible folders.
    pub(crate) fn rows(&self) -> usize {
        FIRST_FOLDER_ROW + self.visible.len()
    }

    pub(crate) fn cursor_on_search(&self) -> bool {
        !self.on_buttons && self.cursor == SEARCH_ROW
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

    /// Field under the cursor, if it is on a field row.
    pub(crate) fn cursor_field(&self) -> Option<WorkspaceField> {
        if self.on_buttons {
            return None;
        }
        DIALOG_FIELDS.get(self.cursor).copied()
    }

    /// Folder under the cursor, if it is on a folder row.
    pub(crate) fn cursor_folder(&self) -> Option<&PathBuf> {
        if self.on_buttons {
            return None;
        }
        self.cursor
            .checked_sub(FIRST_FOLDER_ROW)
            .and_then(|i| self.visible.get(i))
            .and_then(|&i| self.folders.get(i))
    }

    /// Text input under the cursor: a field or the folder search.
    pub(crate) fn cursor_input(&mut self) -> Option<&mut TextInput> {
        match self.cursor_field() {
            Some(WorkspaceField::Name) => Some(&mut self.name),
            Some(WorkspaceField::Includes) => Some(&mut self.includes),
            Some(WorkspaceField::Excludes) => Some(&mut self.excludes),
            None if self.cursor_on_search() => Some(&mut self.folder_query),
            None => None,
        }
    }

    pub(crate) fn input(&self, field: WorkspaceField) -> &TextInput {
        match field {
            WorkspaceField::Name => &self.name,
            WorkspaceField::Includes => &self.includes,
            WorkspaceField::Excludes => &self.excludes,
        }
    }
}

#[derive(Default)]
pub struct WorkspaceScreenState {
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
