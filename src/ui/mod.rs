//! TUI application state and state machine.

pub(crate) mod agent_status;
pub mod background;
pub(crate) mod bookmarks;
pub mod components;
pub mod context_jump;
pub mod copy;
pub mod detail;
pub mod effect;
mod folders;
pub(crate) mod full_search;
mod modes;
pub mod new_session;
pub mod overlays;
pub mod paste;
mod probes;
pub mod profile;
pub mod quick;
pub(crate) mod refresh;
pub(crate) mod reload;
pub mod render;
mod screen;
pub mod session;
pub mod workspace;

pub(crate) use components::input::{
    insert_paste_at, next_grapheme_boundary, prev_grapheme_boundary, PasteOutcome, TextInput,
};
pub use detail::state::{DetailFocus, SessionDetailState};
pub use modes::{Screen, TerminalKind, TerminalRequest, UiMode};
pub use new_session::state::{
    ModelOption, NewSessionFocus, NewSessionRequest, NewSessionState, SessionContextRef,
};
pub use overlays::attach::AttachRequest;
pub use overlays::change_folder::{ChangeFolderFocus, ChangeFolderState};
pub use overlays::confirm::{RenameFocus, RenameModalState};
pub use overlays::filters::ModalState;
pub use overlays::message::{MessageDialog, MessageKind};
pub use overlays::theme::ThemeSelectState;
pub use profile::state::{FormFocus, ProfileFormState};
pub use session::state::Focus;

use folders::folder_names_by_latest;
pub(crate) use folders::{count_by, cwd_counts, cwds_by_latest, folder_name_key};

use crate::config::Config;
use crate::filter::{self, Filter};
use crate::model::Session;
use crate::models::ModelCatalog;
use crate::profile::ProfileStore;
use crate::usage::UsageState;
use background::BackgroundState;
use std::collections::HashMap;
use std::path::PathBuf;

/// Global application state.
pub struct App {
    pub cfg: Config,
    /// List of profiles (vector order corresponds to UI/header index numbers).
    pub profiles: ProfileStore,
    /// `profiles.json` that edits are committed to and `ctrl+u` reloads from.
    /// `None` in unit tests, which never touch the user's file.
    pub(crate) profiles_path: Option<PathBuf>,
    pub sessions: Vec<Session>,
    pub(crate) bookmarks: crate::bookmarks::BookmarkStore,
    pub(crate) bookmarks_path: PathBuf,
    /// Live sessions per profile (`claude agents --json`, agy presence locks):
    /// held by Claude Code's daemon or open in another terminal. Keyed by
    /// profile id then lowercase session id. Display-only.
    pub(crate) agent_status: HashMap<String, crate::agent_status::StatusMap>,
    /// When the next periodic agent-status sweep is due. Starts at launch time,
    /// so the first loop pass runs the launch sweep.
    pub(crate) agent_status_due: std::time::Instant,
    /// User-defined session scopes; `active` narrows every session list.
    pub(crate) workspaces: crate::workspaces::WorkspaceStore,
    /// `None` disables saving (unit tests). A store that failed to load stays
    /// on disk untouched: every save re-reads it and refuses to overwrite it.
    pub(crate) workspaces_path: Option<PathBuf>,
    /// Workspace pane `[NEW WORKSPACE]` row and the workspace edit dialog.
    pub workspace: workspace::WorkspaceScreenState,
    /// Workspace index pending deletion (present when mode == WorkspaceDeleteConfirm).
    pub pending_workspace_delete: Option<usize>,
    pub all_folders: Vec<String>,

    pub filter: Filter,
    /// Byte offset of the search input cursor (relative to filter.keyword).
    pub keyword_cursor: usize,
    pub mode: UiMode,
    /// Current main screen (Session/Profile).
    pub screen: Screen,
    pub focus: Focus,

    /// List of session indices that passed the current filters.
    pub filtered: Vec<usize>,
    /// Selected index within `filtered`.
    pub selected: usize,
    /// Left scroll offset for the right preview panel (lines).
    pub preview_scroll: u16,
    /// Maximum scroll offset for the preview panel (lines). Calculated and saved during render
    /// based on actual lines and viewport height (0 if all contents fit, making it unscrollable).
    pub preview_max_scroll: std::cell::Cell<u16>,
    /// Whether the Session preview ("Prompt") panel expands every user turn to full length,
    /// bypassing the `preview_turn_lines` omission. Toggled via `.` while the preview is focused;
    /// reset to false whenever the selected session changes.
    pub preview_expanded: bool,
    /// Return stack for context-source jumps (`ctrl+o`), consumed by the palette's Back action.
    /// Holds only jump origins — never ordinary cursor movement (`ui/context_jump.rs`).
    pub(crate) context_jump_origins: Vec<context_jump::JumpOrigin>,

    pub agent_modal: Option<ModalState>,
    pub folder_modal: Option<ModalState>,
    pub rename_modal: Option<RenameModalState>,
    /// Target session index for renaming (valid while the rename modal is open).
    /// Kept independent of the main table selection to operate safely within details screens as well.
    pub rename_target: Option<usize>,
    /// Active folder change dialog (present when mode == ChangeFolder).
    pub change_folder: Option<ChangeFolderState>,
    /// Target session index for the folder change, captured when the dialog
    /// opens so the main table cursor can move without affecting it.
    pub change_folder_target: Option<usize>,
    /// Active message dialog (present when mode == Message).
    pub message: Option<MessageDialog>,
    /// Target session index pending deletion.
    pub pending_delete: Option<usize>,
    /// Focused button in the session deletion confirmation modal: Delete (true) or Cancel (false).
    pub delete_ok_focused: bool,
    /// Background session awaiting the Attach/Cancel answer.
    pub pending_attach: Option<overlays::attach::PendingAttach>,
    /// Focused button in the attach confirmation: Attach (true) or Cancel (false).
    pub attach_ok_focused: bool,
    /// Incremental search keyword in the folder filter modal.
    pub folder_query: String,
    /// Folder indices in the stable order captured when the folder modal opens.
    /// Selected folders lead, while both selected and unselected groups retain
    /// the latest-activity order from `all_folders`.
    folder_order: Vec<usize>,
    /// Mapping: folder modal label index <-> index in `all_folders` (reflects search filtering).
    folder_visible: Vec<usize>,
    /// Sessions per folder name that every active condition except the folder
    /// filter keeps: what selecting that folder alone would list. Captured when
    /// the folder modal opens, like `folder_order`.
    pub(crate) folder_counts: HashMap<String, usize>,

    pub scan_info: String,
    pub status_msg: Option<String>,

    /// Table viewport state (scroll offsets). Retained across frames to ensure selection moves
    /// inside the viewport and scrolls only at the edges. Recreating it on every frame resets
    /// offsets to 0, locking the cursor to the bottom while scrolling the list.
    pub table_state: std::cell::RefCell<ratatui::widgets::TableState>,

    /// Selected row index in the profiles list.
    pub profile_selected: usize,
    /// Profile table viewport state.
    pub profile_table_state: std::cell::RefCell<ratatui::widgets::TableState>,
    /// Active profile form (present when mode == ProfileForm).
    pub profile_form: Option<ProfileFormState>,
    /// Target profile index pending deletion.
    pub pending_profile_delete: Option<usize>,
    /// Focused button in the config directory creation confirmation modal: OK (true) or Cancel (false).
    /// Shared by ProfileDirConfirm and ProjectDirConfirm (only one confirm modal is open at a time).
    pub dir_create_ok_focused: bool,
    /// Project folder pending creation (present when mode == ProjectDirConfirm).
    pub project_dir_pending: Option<PathBuf>,
    /// Active new session folder input/select dialog (present when mode == NewSession).
    pub new_session: Option<NewSessionState>,
    /// Active Quick Command palette state (present when mode == QuickCommand).
    pub quick: Option<quick::QuickState>,
    /// Active color theme (every render color derives from this).
    pub theme: crate::theme::Theme,
    /// Active theme selection dialog state (present when mode == ThemeSelect).
    pub theme_select: Option<ThemeSelectState>,
    /// Execution history of Quick Commands (most recent first, persisted in file).
    pub quick_history: Vec<String>,
    /// Execution history of terminal commands (most recent first, persisted in file).
    pub terminal_history: Vec<String>,
    /// Active session detail screen state (present when screen == Detail).
    pub detail: Option<SessionDetailState>,
    /// Whether to display tool calls/results in the right panel of the details screen (defaults to hidden, toggled via `.`).
    pub detail_show_tools: bool,

    pub should_quit: bool,
    pub quit_armed: bool,
    /// Ignores exit keys (q/Ctrl+C) until this instant. Prevents subsequent Ctrl+C keypresses
    /// from an exited agent from triggering s7s's "double press to exit" before the user realizes s7s is restored.
    pub quit_grace_until: Option<std::time::Instant>,
    /// Request to resume the session at the specified sessions index, if set.
    pub resume_request: Option<usize>,
    /// Request to attach to a background Claude session, if set.
    pub attach_request: Option<AttachRequest>,
    /// Request to start a new session in the specified profile/folder, if set.
    pub new_session_request: Option<NewSessionRequest>,
    /// Request to execute the agent for initial setup (login) under the specified profile ID, if set.
    pub login_request: Option<String>,
    /// Request to run a shell command in a session folder, if set.
    pub terminal_request: Option<TerminalRequest>,
    /// Pending in-place effect (rescan / rename / delete) requested by a key
    /// handler, executed at the `App` boundary via [`App::apply_effect`] while
    /// the TUI stays mounted. Unlike the handover `*_request` fields above, these
    /// do not unmount the terminal.
    pub(crate) pending_effect: Option<effect::AppEffect>,
    /// Global-refresh (Ctrl+U) cycle state: prepare at the effect boundary,
    /// spawn the scan after the next draw, and merge repeats until completion
    /// (`effect::RefreshAllPhase`).
    pub(crate) refresh_all: effect::RefreshAllPhase,

    /// Usage (remaining %) display status per profile.
    pub usage: UsageState,
    /// Cache of model catalogs per profile (persisted in models.json). Used in the new session model dropdown.
    pub models: ModelCatalog,
    /// Coordination state for background usage/model probe jobs (receivers and
    /// the model-loading dedup guard). The result caches above stay on `App`;
    /// this owns only the job plumbing (`ui/background.rs`).
    pub(crate) background: BackgroundState,
    /// Full-content search mode (`ui/full_search/`); `None` while closed.
    pub(crate) full_search: Option<full_search::state::FullSearchState>,
}

impl App {
    pub fn new(
        cfg: Config,
        profiles: ProfileStore,
        sessions: Vec<Session>,
        scan_info: String,
    ) -> Self {
        let all_folders = folder_names_by_latest(&sessions);

        let filtered: Vec<usize> = (0..sessions.len()).collect();
        let bookmarks_path = crate::config::bookmarks_path();
        let (bookmarks, bookmark_error) = if cfg!(test) {
            (crate::bookmarks::BookmarkStore::default(), None)
        } else {
            match crate::bookmarks::BookmarkStore::load(&bookmarks_path) {
                Ok(store) => (store, None),
                Err(err) => (
                    crate::bookmarks::BookmarkStore::default(),
                    Some(format!("Bookmarks unavailable: {err}")),
                ),
            }
        };
        let (workspaces, workspaces_path, workspace_error) = if cfg!(test) {
            (crate::workspaces::WorkspaceStore::default(), None, None)
        } else {
            let path = crate::config::workspaces_path();
            match crate::workspaces::WorkspaceStore::load_at_startup(&path) {
                Ok(store) => (store, Some(path), None),
                Err(err) => (
                    crate::workspaces::WorkspaceStore::default(),
                    Some(path),
                    Some(format!(
                        "Workspaces unavailable (changes are not saved): {err}"
                    )),
                ),
            }
        };
        let status_msg = match (bookmark_error, workspace_error) {
            (Some(a), Some(b)) => Some(format!("{a} · {b}")),
            (a, b) => a.or(b),
        };
        let mut app = App {
            cfg,
            profiles,
            profiles_path: (!cfg!(test)).then(crate::profile::profiles_file_path),
            sessions,
            bookmarks,
            bookmarks_path,
            agent_status: HashMap::new(),
            agent_status_due: std::time::Instant::now(),
            workspaces,
            workspaces_path,
            workspace: workspace::WorkspaceScreenState::default(),
            pending_workspace_delete: None,
            all_folders,
            filter: Filter::default(),
            keyword_cursor: 0,
            mode: UiMode::Table,
            screen: Screen::Session,
            focus: Focus::Table,
            filtered,
            selected: 0,
            preview_scroll: 0,
            preview_max_scroll: std::cell::Cell::new(0),
            preview_expanded: false,
            context_jump_origins: Vec::new(),
            agent_modal: None,
            folder_modal: None,
            rename_modal: None,
            rename_target: None,
            change_folder: None,
            change_folder_target: None,
            detail_show_tools: false,
            message: None,
            pending_delete: None,
            delete_ok_focused: false,
            pending_attach: None,
            attach_ok_focused: false,
            folder_query: String::new(),
            folder_order: Vec::new(),
            folder_visible: Vec::new(),
            folder_counts: HashMap::new(),
            scan_info,
            status_msg,
            table_state: std::cell::RefCell::new(ratatui::widgets::TableState::default()),
            profile_selected: 0,
            profile_table_state: std::cell::RefCell::new(ratatui::widgets::TableState::default()),
            profile_form: None,
            pending_profile_delete: None,
            dir_create_ok_focused: false,
            project_dir_pending: None,
            new_session: None,
            quick: None,
            theme: crate::theme::current(),
            theme_select: None,
            quick_history: quick::load_history(),
            terminal_history: quick::load_terminal_history(),
            detail: None,
            should_quit: false,
            quit_armed: false,
            quit_grace_until: None,
            resume_request: None,
            attach_request: None,
            new_session_request: None,
            login_request: None,
            terminal_request: None,
            pending_effect: None,
            refresh_all: effect::RefreshAllPhase::default(),
            usage: UsageState::new(),
            // Unit tests do not load the actual models.json to prevent non-deterministic failures
            // in dropdown initial selections driven by system state.
            models: if cfg!(test) {
                ModelCatalog::default()
            } else {
                ModelCatalog::load()
            },
            background: BackgroundState::default(),
            full_search: None,
        };
        app.recompute();
        app
    }

    /// Rebuilds the visible list (filters AND the open workspace), keeping
    /// activity order within each bookmark group.
    fn rebuild_filtered(&mut self) {
        let mut filtered = filter::apply_with_bookmarks(&self.sessions, &self.filter, |session| {
            self.bookmarks.contains(session)
        });
        self.retain_workspace_scope(&mut filtered);
        self.retain_full_search_matches(&mut filtered);
        self.filtered = filtered;
        self.filtered
            .sort_by_cached_key(|&idx| !self.bookmarks.contains(&self.sessions[idx]));
    }

    /// Applies active filters and bookmark priority, then resets selection / scroll positions.
    pub fn recompute(&mut self) {
        self.rebuild_filtered();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
        self.preview_scroll = 0;
        self.preview_expanded = false;
        // Reset table viewport scroll to top when filters change (in case the result set shrinks significantly).
        *self.table_state.borrow_mut().offset_mut() = 0;
    }

    /// Returns the currently selected session.
    pub fn current(&self) -> Option<&Session> {
        self.filtered.get(self.selected).map(|&i| &self.sessions[i])
    }

    /// Session the active screen operates on: the list cursor on Session, the
    /// detail target on Detail. The Profile screen has no focused session.
    pub(crate) fn focused_session_index(&self) -> Option<usize> {
        match self.screen {
            Screen::Session => self.filtered.get(self.selected).copied(),
            Screen::Detail => self.detail.as_ref().map(|d| d.session_idx),
            Screen::Profile => None,
        }
    }

    /// Returns whether any background job (usage, models, sessions) is in progress (used to determine polling frequency).
    pub fn background_in_flight(&self) -> bool {
        self.background.in_flight()
    }

    /// Polls and applies all background job results. Returns true if updates occurred (triggering a redraw).
    pub fn poll_background(&mut self) -> bool {
        let usage_updated = self.poll_usage();
        let models_updated = self.poll_models();
        let sessions_updated = self.poll_refresh_scan();
        let status_updated = self.poll_agent_status();
        let search_updated = self.poll_full_search();
        self.tick_agent_status();
        usage_updated || models_updated || sessions_updated || status_updated || search_updated
    }

    fn rebuild_all_folders(&mut self) {
        self.invalidate_workspace_membership();
        self.all_folders = folder_names_by_latest(&self.sessions);
        self.refresh_workspace_folders();
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
