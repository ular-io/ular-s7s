//! TUI application state and state machine.

pub mod background;
pub mod components;
pub mod detail;
pub mod effect;
pub mod new_session;
pub mod overlays;
pub mod profile;
pub mod quick;
pub mod render;
pub mod session;

pub(crate) use components::input::{next_char_boundary, prev_char_boundary, TextInput};
pub use detail::state::{DetailFocus, SessionDetailState};
pub use new_session::state::{
    ModelOption, NewSessionFocus, NewSessionRequest, NewSessionState, SessionContextRef,
};
pub use overlays::confirm::{RenameFocus, RenameModalState};
pub use overlays::filters::ModalState;
pub use overlays::message::{MessageDialog, MessageKind};
pub use overlays::theme::ThemeSelectState;
pub use profile::state::{FormFocus, ProfileFormState};
pub use session::state::Focus;

use crate::config::Config;
use crate::filter::{self, Filter};
use crate::model::{Agent, Session};
use crate::models::{self, ModelCatalog};
use crate::profile::ProfileStore;
use crate::usage::{self, UsagePhase, UsageState};
use anyhow::{anyhow, Context, Result};
use background::BackgroundState;
use std::fs;
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
    /// Session renaming modal.
    Rename,
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
    pub sessions: Vec<Session>,
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

    pub agent_modal: Option<ModalState>,
    pub folder_modal: Option<ModalState>,
    pub rename_modal: Option<RenameModalState>,
    /// Target session index for renaming (valid while the rename modal is open).
    /// Kept independent of the main table selection to operate safely within details screens as well.
    pub rename_target: Option<usize>,
    /// Active message dialog (present when mode == Message).
    pub message: Option<MessageDialog>,
    /// Target session index pending deletion.
    pub pending_delete: Option<usize>,
    /// Focused button in the session deletion confirmation modal: Delete (true) or Cancel (false).
    pub delete_ok_focused: bool,
    /// Incremental search keyword in the folder filter modal.
    pub folder_query: String,
    /// Mapping: folder modal label index <-> index in `all_folders` (reflects search filtering).
    folder_visible: Vec<usize>,

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
    /// Two-phase global-refresh (Ctrl+U) cycle state: the prepare step runs at
    /// the effect boundary, the session scan runs right after the next draw,
    /// and repeat requests merge until the completion frame renders
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
        let mut all_folders: Vec<String> = sessions
            .iter()
            .map(|s| s.folder.clone())
            .filter(|f| !f.is_empty())
            .collect();
        all_folders.sort_unstable();
        all_folders.dedup();

        let filtered: Vec<usize> = (0..sessions.len()).collect();
        let mut app = App {
            cfg,
            profiles,
            sessions,
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
            agent_modal: None,
            folder_modal: None,
            rename_modal: None,
            rename_target: None,
            detail_show_tools: false,
            message: None,
            pending_delete: None,
            delete_ok_focused: false,
            folder_query: String::new(),
            folder_visible: Vec::new(),
            scan_info,
            status_msg: None,
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

    /// Applies active filters and resets selection / scroll positions.
    pub fn recompute(&mut self) {
        self.filtered = filter::apply(&self.sessions, &self.filter);
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

    /// Rescans sessions on disk to refresh the session list (utilizes mtime-based incremental cache).
    ///
    /// Low overhead as only modified or new files are parsed. Tracks current selection by (agent, id)
    /// to preserve cursor position post-refresh (even if content modifications re-order lists to the top).
    /// Shared between automatic triggers on resume return and manual Ctrl+U refreshes.
    pub fn refresh_sessions(&mut self) {
        let prev = self.current().map(|s| (s.agent, s.id.clone()));
        // If the details view is open, capture the target session's identity to rebind post-refresh.
        let detail_key = self
            .detail
            .as_ref()
            .and_then(|d| self.sessions.get(d.session_idx))
            .map(|s| (s.agent, s.id.clone()));

        let result = crate::scan::scan(&self.profiles.profiles, false);
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
        if let Some((agent, id)) = prev {
            if let Some(pos) = self
                .filtered
                .iter()
                .position(|&i| self.sessions[i].agent == agent && self.sessions[i].id == id)
            {
                self.selected = pos;
            }
        }

        // Rebind details screen: update target session index and re-parse turns
        // (reflecting updates such as messages added on resume return). Closes details view if session was deleted.
        if self.detail.is_some() {
            let found = detail_key.and_then(|(agent, id)| {
                self.sessions
                    .iter()
                    .position(|s| s.agent == agent && s.id == id)
            });
            match found {
                Some(idx) => {
                    let turns = crate::handoff::load_turns(&self.sessions[idx]);
                    if turns.is_empty() {
                        self.close_session_detail();
                    } else if let Some(d) = self.detail.as_mut() {
                        d.session_idx = idx;
                        d.selected = d.selected.min(turns.len() - 1);
                        d.turns = turns;
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
        if updated && !self.background.usage_in_flight() {
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

    /// Returns whether any background query (usage or models) is in progress (used to determine polling frequency).
    pub fn background_in_flight(&self) -> bool {
        self.background.in_flight()
    }

    /// Polls and applies all background query results. Returns true if updates occurred (triggering a redraw).
    pub fn poll_background(&mut self) -> bool {
        let usage_updated = self.poll_usage();
        let models_updated = self.poll_models();
        usage_updated || models_updated
    }

    fn rebuild_all_folders(&mut self) {
        let mut all_folders: Vec<String> = self
            .sessions
            .iter()
            .map(|s| s.folder.clone())
            .filter(|f| !f.is_empty())
            .collect();
        all_folders.sort_unstable();
        all_folders.dedup();
        self.all_folders = all_folders;
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

    fn delete_session_artifacts(&self, session: &Session) -> Result<()> {
        let Some(source_path) = session.source_path.as_ref() else {
            return Err(anyhow!("source path is missing"));
        };

        self.remove_file_best_effort(source_path)
            .with_context(|| format!("remove {}", source_path.display()))?;

        if session.agent == Agent::Antigravity {
            // Best-effort cache cleanup; skipped when the owning profile is gone
            // (never touch another profile's metadata store).
            if let Some(root) = self.session_profile_root(session) {
                let _ = self.remove_antigravity_metadata(&root, session.id.as_str());
            }
            self.remove_sqlite_sidecars(source_path);
        }

        Ok(())
    }

    /// Config root (`Profile.path`) of the profile a session belongs to.
    /// Sessions are re-stamped with live profile ids on every scan, so a miss
    /// means a stale list — callers must not fall back to the default root.
    fn session_profile_root(&self, session: &Session) -> Option<std::path::PathBuf> {
        self.profiles
            .find(&session.profile_id)
            .map(|p| p.path.clone())
    }

    fn remove_file_best_effort(&self, path: &std::path::Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn remove_sqlite_sidecars(&self, db_path: &std::path::Path) {
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sidecar = db_path.to_path_buf();
            let name = match db_path.file_name().and_then(|s| s.to_str()) {
                Some(name) => format!("{name}{suffix}"),
                None => continue,
            };
            sidecar.set_file_name(name);
            let _ = fs::remove_file(sidecar);
        }
    }

    fn remove_antigravity_metadata(&self, profile_root: &std::path::Path, id: &str) -> Result<()> {
        let path = profile_root.join("cache/conversation_metadata.json");
        let data = match fs::read_to_string(&path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        let mut root: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        if let Some(conversations) = root
            .get_mut("conversations")
            .and_then(serde_json::Value::as_object_mut)
        {
            conversations.remove(id);
            let bytes = serde_json::to_vec_pretty(&root)?;
            fs::write(&path, bytes)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
