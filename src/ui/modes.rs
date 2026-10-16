//! Screen and UI-mode enums plus the terminal handover request.

use std::path::PathBuf;

/// Main screen variants. Cycled/switched via Quick Command (`:`) window commands or ←/→ arrows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Session list view (default), "Session Search screen".
    Session,
    /// Profile list view.
    Profile,
    /// Session details view (per-question workspace tasks/answers). Drill-down screen entered via → arrow key from search preview.
    Detail,
}

/// UI modes determining input event dispatching branches (TUI state machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMode {
    /// Main table navigation (Session/Profile lists depending on active Screen).
    Table,
    /// Keyword search text input.
    Keyword,
    /// Agent filter multi-select modal.
    AgentModal,
    /// Folder filter multi-select modal.
    FolderModal,
    /// Session deletion confirmation modal.
    DeleteConfirm,
    /// Attach/Cancel question for a Claude session held by Claude Code's daemon.
    AttachConfirm,
    /// Session renaming modal.
    Rename,
    /// Session folder change dialog (palette-only `Change Folder`). Re-points
    /// where the session opens next without moving files.
    ChangeFolder,
    /// Profile creation/edit form.
    ProfileForm,
    /// Profile deletion confirmation modal.
    ProfileDeleteConfirm,
    /// Directory creation confirmation modal prompt for missing config folders on profile save (login task triggered on confirm).
    ProfileDirConfirm,
    /// New session creation dialog (profile selection + folder lookup/input). Accessible globally via Ctrl+N.
    NewSession,
    /// Project folder creation confirmation modal shown when the New Session folder input is a
    /// bare name (no path separator) with no matching folder under `config::projects_dir()`.
    /// Create makes the folder and starts the session; Cancel returns to the New Session dialog.
    ProjectDirConfirm,
    /// `:`/`!` Quick Command window (palette: incremental search and command trigger;
    /// terminal: shell command input with history in the selected session's folder).
    QuickCommand,
    /// Theme selection dialog (live preview on cursor move; Enter commits, Esc reverts).
    ThemeSelect,
    /// Global keybindings help screen.
    Help,
    /// Generic alert dialog (info / warning / error). Reverts to the prior UI mode upon dismissal.
    Message,
    /// Workspace edit dialog (name, include/exclude words, folders; Save/Cancel),
    /// opened from the Session screen's workspace pane.
    WorkspaceEdit,
    /// Workspace deletion confirmation modal.
    WorkspaceDeleteConfirm,
}

/// Request to run a shell command in a session folder (`!` terminal mode).
/// Processed by the main loop after releasing TUI.
#[derive(Debug, Clone)]
pub struct TerminalRequest {
    pub cwd: PathBuf,
    pub command: String,
    pub kind: TerminalKind,
}

/// Origin of a terminal request; drives the post-exit behavior in the handover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    /// User `!` command: wait for a keypress after exit so short-lived output is
    /// not wiped by the TUI redraw. Failures show a warning and also wait.
    Command,
    /// Edit Config editor session: return without waiting (no output to read).
    /// On failure, offer to reopen the config with vim (a broken `editor` value
    /// would otherwise lock the user out of fixing it from within s7s).
    EditConfig,
}
