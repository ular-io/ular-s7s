//! Workspaces screen state: the focused pane, the Detail pane cursor and its
//! stable folder rows, and the in-place text edit. The workspace list cursor is
//! not stored here — it is `WorkspaceStore::active`, so the list row the user
//! sits on is always the scope the session lists show.

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
    /// Detail row: `0..DETAIL_FIELDS.len()` are fields, the rest index `folders`.
    pub detail_cursor: usize,
    /// Folder rows of the Detail pane in an order captured when the workspace
    /// is opened (its selected folders first, then the rest by latest activity).
    /// Toggling does not reorder, so the cursor stays on the row it toggled.
    pub folders: Vec<PathBuf>,
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
            edit: None,
            list_scroll: Cell::new(0),
            folder_scroll: Cell::new(0),
        }
    }
}

impl WorkspaceScreenState {
    pub(crate) fn detail_rows(&self) -> usize {
        DETAIL_FIELDS.len() + self.folders.len()
    }

    /// Field under the Detail cursor, if it is on a field row.
    pub(crate) fn cursor_field(&self) -> Option<WorkspaceField> {
        DETAIL_FIELDS.get(self.detail_cursor).copied()
    }

    /// Folder under the Detail cursor, if it is on a folder row.
    pub(crate) fn cursor_folder(&self) -> Option<&PathBuf> {
        self.detail_cursor
            .checked_sub(DETAIL_FIELDS.len())
            .and_then(|i| self.folders.get(i))
    }
}
