//! Workspaces screen state: the focused pane, the Detail pane cursor, its
//! stable folder rows and their search, and the in-place text edit. The
//! workspace list cursor is not stored here — it is `WorkspaceStore::active`,
//! so the list row the user sits on is always the scope the session lists show.

use crate::ui::TextInput;
use std::cell::Cell;
use std::path::PathBuf;

/// Panes of the Workspaces screen, left to right. Moved between with ←/→.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspacePane {
    List,
    Detail,
    Sessions,
}

/// Editable text attributes of a workspace (the Detail pane's first rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceField {
    Name,
    Includes,
    Excludes,
}

/// Detail pane rows before the folder rows, in display order.
pub(crate) const DETAIL_FIELDS: [WorkspaceField; 3] = [
    WorkspaceField::Name,
    WorkspaceField::Includes,
    WorkspaceField::Excludes,
];

/// Detail row of the folder search, right above the folder rows.
pub(crate) const SEARCH_ROW: usize = DETAIL_FIELDS.len();
/// Detail row of the first visible folder.
pub(crate) const FIRST_FOLDER_ROW: usize = SEARCH_ROW + 1;

/// An in-progress edit (present while `UiMode::WorkspaceEdit`).
pub struct WorkspaceEdit {
    /// Index into `WorkspaceStore::workspaces`.
    pub workspace: usize,
    pub field: WorkspaceField,
    /// Name edit drawn in the list pane (after `+` or Enter on a list row)
    /// rather than on the Detail pane's Name row.
    pub in_list: bool,
    pub input: TextInput,
    /// Value before the edit, restored by Esc. Include/exclude edits apply
    /// live so the session list follows each keystroke.
    pub original: String,
    /// Workspace added by `+` and not yet saved: Esc removes it again and
    /// reopens `previous_active`.
    pub created: bool,
    pub previous_active: Option<String>,
}

pub struct WorkspaceScreenState {
    pub pane: WorkspacePane,
    /// Detail row: `0..DETAIL_FIELDS.len()` are fields, `SEARCH_ROW` is the
    /// folder search, and from `FIRST_FOLDER_ROW` on the rows index `visible`.
    pub detail_cursor: usize,
    /// Folder rows of the Detail pane in an order captured when the workspace
    /// is opened (its selected folders first, then the rest by latest activity).
    /// Toggling does not reorder, so the cursor stays on the row it toggled.
    pub folders: Vec<PathBuf>,
    /// Folder search, typed directly while the cursor is on `SEARCH_ROW`
    /// (no Enter). Not stored; cleared when the scope changes.
    pub folder_query: TextInput,
    /// Indices into `folders` matching `folder_query`, in `folders` order.
    pub visible: Vec<usize>,
    pub edit: Option<WorkspaceEdit>,
    /// Viewport offsets, adjusted during render to keep the cursor visible.
    pub list_scroll: Cell<usize>,
    pub folder_scroll: Cell<usize>,
}

impl Default for WorkspaceScreenState {
    fn default() -> Self {
        Self {
            pane: WorkspacePane::List,
            detail_cursor: 0,
            folders: Vec::new(),
            folder_query: TextInput::new(String::new()),
            visible: Vec::new(),
            edit: None,
            list_scroll: Cell::new(0),
            folder_scroll: Cell::new(0),
        }
    }
}

impl WorkspaceScreenState {
    pub(crate) fn detail_rows(&self) -> usize {
        FIRST_FOLDER_ROW + self.visible.len()
    }

    pub(crate) fn cursor_on_search(&self) -> bool {
        self.detail_cursor == SEARCH_ROW
    }

    /// Empties the folder search. Callers rebuild `visible` afterwards.
    pub(crate) fn clear_folder_query(&mut self) {
        self.folder_query = TextInput::new(String::new());
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

    /// Field under the Detail cursor, if it is on a field row.
    pub(crate) fn cursor_field(&self) -> Option<WorkspaceField> {
        DETAIL_FIELDS.get(self.detail_cursor).copied()
    }

    /// Folder under the Detail cursor, if it is on a folder row.
    pub(crate) fn cursor_folder(&self) -> Option<&PathBuf> {
        self.detail_cursor
            .checked_sub(FIRST_FOLDER_ROW)
            .and_then(|i| self.visible.get(i))
            .and_then(|&i| self.folders.get(i))
    }
}

/// Bare basename of a folder row (`[SCRATCH]` for the scratch workspace).
pub(crate) fn folder_display_label(folder: &std::path::Path) -> String {
    let basename = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.to_string_lossy().into_owned());
    crate::scratch::folder_label(folder, &basename).to_string()
}
