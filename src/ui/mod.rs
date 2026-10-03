//! TUI application state and state machine.

pub(crate) mod agent_status;
pub mod background;
pub(crate) mod bookmarks;
pub mod components;
pub mod context_jump;
pub mod copy;
pub mod detail;
pub mod effect;
pub mod new_session;
pub mod overlays;
pub mod paste;
pub mod profile;
pub mod quick;
pub(crate) mod refresh;
pub(crate) mod reload;
pub mod render;
pub mod session;
pub mod workspace;

pub(crate) use components::input::{
    insert_paste_at, next_grapheme_boundary, prev_grapheme_boundary, PasteOutcome, TextInput,
};
pub use detail::state::{DetailFocus, SessionDetailState};
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

use crate::config::Config;
use crate::filter::{self, Filter};
use crate::model::Session;
use crate::models::{self, ModelCatalog};
use crate::profile::ProfileStore;
use crate::usage::{self, UsagePhase, UsageState};
use background::BackgroundState;
use std::collections::HashMap;
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
        };
        app.recompute();
        app
    }

    /// Rebuilds the visible list (filters AND the open workspace), keeping
    /// activity order within each bookmark group.
    fn rebuild_filtered(&mut self) {
        self.filtered = filter::apply_with_bookmarks(&self.sessions, &self.filter, |session| {
            self.bookmarks.contains(session)
        });
        if let Some(ws) = self.workspaces.active_workspace() {
            self.filtered.retain(|&idx| ws.matches(&self.sessions[idx]));
        }
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

    /// Rescans sessions on disk to refresh the session list (utilizes mtime-based incremental cache).
    ///
    /// Only modified or new files are parsed. Tracks selection by (agent, profile, id)
    /// to preserve cursor position post-refresh (even if content modifications re-order lists to the top).
    /// Used after handovers and mutations. Ctrl+U scans on a worker and shares
    /// only result application with this synchronous path.
    pub fn refresh_sessions(&mut self) {
        self.cancel_refresh_scan();
        let result = crate::scan::scan(&self.profiles.profiles, false);
        let detail = self.detail_key().and_then(|key| {
            result
                .sessions
                .iter()
                .find(|s| key.matches(s))
                .map(|s| (key, crate::handoff::load_turns(s)))
        });
        self.apply_session_scan(result, detail);
        // A handover is where `←` most often moves a session to the background.
        self.request_agent_status();
    }

    /// Applies a completed index against the selection and detail target that
    /// exist now, including navigation performed while the worker was running.
    pub(crate) fn apply_session_scan(
        &mut self,
        result: crate::scan::ScanResult,
        detail: Option<refresh::DetailRefresh>,
    ) {
        let prev = self.current().map(refresh::SessionKey::of);
        let detail_key = self.detail_key();
        let preview = (
            self.preview_scroll,
            self.preview_expanded,
            self.table_state.borrow().offset(),
        );
        self.scan_info = format!(
            "{} sessions · reparsed {}/{}",
            result.sessions.len(),
            result.reparsed_files,
            result.scanned_files
        );
        self.sessions = result.sessions;
        self.rebuild_all_folders();
        self.recompute();

        // Restore selection: if the same session still passes the filters, move cursor to its new index.
        if let Some(key) = prev {
            if let Some(pos) = self
                .filtered
                .iter()
                .position(|&i| key.matches(&self.sessions[i]))
            {
                self.selected = pos;
                self.preview_scroll = preview.0;
                self.preview_expanded = preview.1;
                *self.table_state.borrow_mut().offset_mut() = preview.2;
            }
        }

        // Rebind the current Detail target. Only replace its turns when the
        // result was loaded for that identity; navigation may have changed it.
        if self.detail.is_some() {
            let found = detail_key
                .as_ref()
                .and_then(|key| self.sessions.iter().position(|s| key.matches(s)));
            match found {
                Some(idx) => {
                    let turns = detail
                        .filter(|(key, _)| Some(key) == detail_key.as_ref())
                        .map(|(_, turns)| turns);
                    if turns.as_ref().is_some_and(Vec::is_empty) {
                        self.close_session_detail();
                    } else if let Some(d) = self.detail.as_mut() {
                        d.session_idx = idx;
                        if let Some(turns) = turns {
                            d.selected = d.selected.min(turns.len() - 1);
                            d.expanded_prompt = d.expanded_prompt.filter(|&i| i < turns.len());
                            d.turns = turns;
                        }
                    }
                }
                None => self.close_session_detail(),
            }
        }
    }

    /// Spawns parallel queries to fetch usage (remaining %) for all fetchable profiles. Ignored if queries are already in flight.
    pub fn start_usage_fetch(&mut self) {
        if self.usage_in_flight() {
            return;
        }
        let ids: Vec<String> = self
            .profiles
            .profiles
            .iter()
            .map(|p| p.id.clone())
            .collect();
        self.start_usage_fetch_for(&ids);
    }

    /// Spawns parallel queries to fetch usage for specified profiles (used for incremental refreshes on profile add/edit).
    /// Only skips profiles currently in `Loading` state. Since non-fetchable states (missing directories, unsupportive agents)
    /// are modeled as results (`UsageResult::MissingDir` / `Unavailable`), all profiles are targetable,
    /// and profiles not included in an ongoing global query can start query tasks immediately.
    pub fn start_usage_fetch_for(&mut self, profile_ids: &[String]) {
        // Prevent launching interactive CLI processes (PTY) during unit tests.
        if cfg!(test) {
            return;
        }
        let targets: Vec<crate::profile::Profile> = self
            .profiles
            .profiles
            .iter()
            .filter(|p| profile_ids.contains(&p.id))
            .filter(|p| self.usage.entry(&p.id).phase != UsagePhase::Loading)
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        for p in &targets {
            // Retain the prior successful value (`last`) while turning on the progress indicator.
            self.usage.entry_mut(&p.id).phase = UsagePhase::Loading;
        }
        self.background.spawn_usage(targets);
    }

    /// Applies background usage query results to app state. Returns true if updates occurred (triggering a redraw).
    pub fn poll_usage(&mut self) -> bool {
        if !self.background.usage_in_flight() {
            return false;
        }
        let results = self.background.drain_usage();
        let updated = !results.is_empty();
        for (profile_id, res) in results {
            let entry = self.usage.entry_mut(&profile_id);
            match res {
                usage::UsageResult::Ready(snapshot) => {
                    entry.last = Some(snapshot);
                    entry.phase = UsagePhase::Ready;
                }
                // Logged out, missing CLI, missing dir, or unavailable: clear `last` to prevent misleading displays.
                usage::UsageResult::NotLoggedIn => {
                    entry.last = None;
                    entry.phase = UsagePhase::NotLoggedIn;
                }
                usage::UsageResult::NotInstalled => {
                    entry.last = None;
                    entry.phase = UsagePhase::NotInstalled;
                }
                usage::UsageResult::MissingDir => {
                    entry.last = None;
                    entry.phase = UsagePhase::MissingDir;
                }
                usage::UsageResult::Unavailable => {
                    entry.last = None;
                    entry.phase = UsagePhase::Unavailable;
                }
                // Keep the prior value (`last`) even if the current query failed.
                usage::UsageResult::Failed(_) => entry.phase = UsagePhase::Failed,
            }
        }
        if updated
            && !self.background.usage_in_flight()
            && !matches!(
                self.refresh_all,
                effect::RefreshAllPhase::Prepared | effect::RefreshAllPhase::Scanning
            )
        {
            self.status_msg = Some("usage update complete".to_string());
        }
        updated
    }

    /// Returns whether a usage query task is in progress (used to determine polling frequency in the main loop).
    pub fn usage_in_flight(&self) -> bool {
        self.background.usage_in_flight()
    }

    /// Spawns parallel queries to fetch model catalogs for all profiles.
    ///
    /// If `force` is false (app startup), bypasses queries for profiles where the cached CLI version
    /// matches the current version (version gate to avoid expensive claude PTY spin-up costs).
    /// If `force` is true (Ctrl+U), forcefully queries all profiles. Skips profiles with active tasks.
    pub fn start_models_fetch(&mut self, force: bool) {
        let ids: Vec<String> = self
            .profiles
            .profiles
            .iter()
            .map(|p| p.id.clone())
            .collect();
        self.start_models_fetch_for(&ids, force);
    }

    /// Spawns queries to fetch model catalogs for specified profiles (used for incremental updates on profile add/edit).
    pub fn start_models_fetch_for(&mut self, profile_ids: &[String], force: bool) {
        // Prevent launching interactive CLI processes (PTY) during unit tests.
        if cfg!(test) {
            return;
        }
        let targets: Vec<crate::profile::Profile> = self
            .profiles
            .profiles
            .iter()
            .filter(|p| profile_ids.contains(&p.id))
            .filter(|p| !self.background.is_models_loading(&p.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let cached_versions = targets
            .iter()
            .filter_map(|p| self.models.cached_version(&p.id).map(|v| (p.id.clone(), v)))
            .collect();
        self.background
            .spawn_models(targets, cached_versions, force);
    }

    /// Applies background model catalog query results. Returns true if updates occurred.
    ///
    /// Preserves existing caches on query failure or unavailability. Since CLIs do not filter out
    /// invalid model names, we must not clear cached catalogs on unsuccessful updates.
    pub fn poll_models(&mut self) -> bool {
        if !self.background.models_in_flight() {
            return false;
        }
        // Draining clears the per-profile loading guard; applying the results to
        // the model cache and persisting them stays here (the cache is App-owned).
        let results = self.background.drain_models();
        let mut dirty = false;
        for (profile_id, res) in results {
            if let models::ModelsResult::Ready(pm) = res {
                self.models.insert(profile_id, pm);
                dirty = true;
            }
        }
        if dirty {
            self.models.save().ok();
        }
        dirty
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
        self.tick_agent_status();
        usage_updated || models_updated || sessions_updated || status_updated
    }

    fn rebuild_all_folders(&mut self) {
        self.all_folders = folder_names_by_latest(&self.sessions);
        self.refresh_workspace_folders();
    }

    // ---- Filter Operations ----

    /// Header number keys (`1..5`): Activates a single profile filter at the specified numbered index.
    fn set_single_profile(&mut self, idx: usize) {
        let Some((id, name)) = self
            .profiles
            .numbered_profiles()
            .get(idx)
            .map(|p| (p.id.clone(), p.name.clone()))
        else {
            return;
        };
        self.filter.profile_ids.clear();
        self.filter.profile_ids.insert(id);
        self.recompute();
        self.status_msg = Some(format!("Profile filter: {}", name));
    }

    /// Resolves profile ID to display name (used in filter descriptions).
    pub fn profile_name(&self, id: &str) -> Option<String> {
        self.profiles.find(id).map(|p| p.name.clone())
    }

    // ---- Screen Switching / Profile Screen ----

    /// Switched screen to target and reverts to table navigation mode.
    fn switch_screen(&mut self, screen: Screen) {
        self.screen = screen;
        self.mode = UiMode::Table;
        self.status_msg = None;
        if screen != Screen::Detail {
            self.detail = None;
        }
        // The workspace pane closes when its screen is left, so returning to
        // the Session screen lands on the session list unless told otherwise.
        if screen != Screen::Session && self.focus == Focus::Workspaces {
            self.focus = Focus::Table;
        }
        if screen == Screen::Profile {
            self.profile_selected = self
                .profile_selected
                .min(self.profiles.profiles.len().saturating_sub(1));
        }
    }

    fn clear_all_filters(&mut self) {
        self.filter = Filter::default();
        self.recompute();
        self.status_msg = Some("Filters cleared".to_string());
    }

    /// Requests resuming the targeted session. Triggers an alert instead of handover
    /// if the project directory no longer exists (pre-flight check).
    fn request_resume(&mut self, idx: usize) {
        let cwd = self.sessions[idx].cwd.clone();
        // Omit check if cwd is empty, as it runs without directory changes.
        if !cwd.as_os_str().is_empty() && !cwd.is_dir() {
            self.show_message(
                " Cannot Resume ",
                vec![
                    "The project folder no longer exists:".to_string(),
                    cwd.to_string_lossy().into_owned(),
                    String::new(),
                    "This session cannot be resumed.".to_string(),
                ],
                MessageKind::Error,
            );
            return;
        }
        if self.divert_live_session(idx, agent_status::LiveAction::Resume) {
            return;
        }
        self.resume_request = Some(idx);
    }

    fn arm_quit(&mut self) {
        if let Some(until) = self.quit_grace_until {
            let now = std::time::Instant::now();
            if now < until {
                // Ignore exits during grace period to defend against trailing spam; extends the grace window
                // slightly to ensure keystrokes only register after user halts spamming.
                const QUIT_GRACE_REPEAT: std::time::Duration =
                    std::time::Duration::from_millis(400);
                self.quit_grace_until = Some(until.max(now + QUIT_GRACE_REPEAT));
                return;
            }
            self.quit_grace_until = None;
        }
        if self.quit_armed {
            self.should_quit = true;
        } else {
            self.quit_armed = true;
            self.status_msg = Some("Press q or ctrl+c again to quit".to_string());
        }
    }

    /// Begins the exit key grace period upon returning from agent handover (called in main loop).
    pub fn begin_quit_grace(&mut self) {
        const QUIT_GRACE: std::time::Duration = std::time::Duration::from_millis(1200);
        self.quit_grace_until = Some(std::time::Instant::now() + QUIT_GRACE);
    }
}

/// Distinct `key` values over `sessions`, newest latest activity first and ties
/// by key: the order every folder list shows. Sessions mapped to `None` are
/// skipped.
pub(crate) fn by_latest_activity<K: Ord + Clone + std::hash::Hash>(
    sessions: &[Session],
    key: impl Fn(&Session) -> Option<K>,
) -> Vec<K> {
    let mut latest: HashMap<K, i64> = HashMap::new();
    for s in sessions {
        if let Some(k) = key(s) {
            let at = latest.entry(k).or_insert(i64::MIN);
            *at = (*at).max(s.updated_at_ms);
        }
    }
    let mut keys: Vec<(K, i64)> = latest.into_iter().collect();
    keys.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    keys.into_iter().map(|(k, _)| k).collect()
}

/// Sessions per `key`; sessions mapped to `None` are not counted.
pub(crate) fn count_by<'a, K: Eq + std::hash::Hash>(
    sessions: impl IntoIterator<Item = &'a Session>,
    key: impl Fn(&Session) -> Option<K>,
) -> HashMap<K, usize> {
    let mut counts = HashMap::new();
    for s in sessions {
        if let Some(k) = key(s) {
            *counts.entry(k).or_insert(0) += 1;
        }
    }
    counts
}

/// Full-path folder key of a session; `None` when no cwd was recorded.
fn cwd_key(s: &Session) -> Option<PathBuf> {
    (!s.cwd.as_os_str().is_empty()).then(|| s.cwd.clone())
}

/// Folder-name key (the folder filter's unit); `None` when the name is empty.
pub(crate) fn folder_name_key(s: &Session) -> Option<String> {
    (!s.folder.is_empty()).then(|| s.folder.clone())
}

/// Sessions per cwd, the count every full-path folder list shows.
pub(crate) fn cwd_counts(sessions: &[Session]) -> HashMap<PathBuf, usize> {
    count_by(sessions, cwd_key)
}

/// Session cwds by latest activity, empty cwds skipped.
pub(crate) fn cwds_by_latest(sessions: &[Session]) -> Vec<PathBuf> {
    by_latest_activity(sessions, cwd_key)
}

/// Folder names by latest activity.
fn folder_names_by_latest(sessions: &[Session]) -> Vec<String> {
    by_latest_activity(sessions, folder_name_key)
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
