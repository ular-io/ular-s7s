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
mod tests {
    use super::effect::AppEffect;
    use super::new_session::state::is_bare_project_name;
    use super::quick::QuickMode;
    use super::{
        App, DetailFocus, Focus, MessageKind, NewSessionFocus, Screen, TerminalKind, UiMode,
    };
    use crate::config::Config;
    use crate::model::{Agent, Session};
    use crate::models::LastSelection;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// Returns an empty mock ProfileStore for unit testing (prevents disk scanning / file saving).
    fn test_profiles() -> crate::profile::ProfileStore {
        crate::profile::ProfileStore {
            profiles: Vec::new(),
        }
    }

    fn empty_app() -> App {
        App::new(
            Config::load(),
            test_profiles(),
            Vec::new(),
            "0 sessions · reparsed 0/0".to_string(),
        )
    }

    fn app_with_session() -> App {
        App::new(
            Config::load(),
            test_profiles(),
            vec![Session {
                agent: Agent::Codex,
                profile_id: String::new(),
                id: "session-1".to_string(),
                source_path: None,
                cwd: PathBuf::from("/tmp"),
                folder: "tmp".to_string(),
                mtime_ms: 0,
                ctime_ms: 0,
                size_bytes: 0,
                user_turns: vec!["hello".to_string()],
                search_blob: "hello".to_string(),
                assistant_blob: String::new(),
                title_hint: Some("hello".to_string()),
                title_fixed: false,
            }],
            "1 sessions · reparsed 0/0".to_string(),
        )
    }

    #[test]
    fn profile_form_save_with_missing_dir_opens_create_confirm() {
        let mut app = empty_app();
        app.screen = Screen::Profile;
        app.open_profile_form(None);
        assert_eq!(app.mode, UiMode::ProfileForm);

        let missing = std::env::temp_dir().join("s7s-test-missing-config-dir-xyz");
        assert!(!missing.exists());
        let form = app.profile_form.as_mut().unwrap();
        form.name.value = "Team".to_string();
        form.path.value = missing.to_string_lossy().into_owned();

        app.confirm_profile_form();

        // Instead of saving, directory creation confirmation modal must be triggered (OK button focused).
        assert_eq!(app.mode, UiMode::ProfileDirConfirm);
        assert!(app.dir_create_ok_focused);
        assert!(app.profiles.profiles.is_empty());

        // Esc key returns back to form, preserving input values.
        app.on_key_profile_dir_confirm(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::ProfileForm);
        assert!(app.profile_form.is_some());
    }

    #[test]
    fn confirm_profile_form_enqueues_profile_saved_effect() {
        let mut app = empty_app();
        app.screen = Screen::Profile;
        app.open_profile_form(None);

        // An existing directory skips the create-confirm modal and commits directly.
        let dir = std::env::temp_dir().join(format!(
            "s7s-test-existing-cfg-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let form = app.profile_form.as_mut().unwrap();
        form.name.value = "Team".to_string();
        form.path.value = dir.to_string_lossy().into_owned();

        app.confirm_profile_form();

        // The store is mutated in memory, but persistence + rescan are deferred to
        // the effect, so the form stays open until the boundary runs it.
        match app.pending_effect.as_ref() {
            Some(AppEffect::ProfileSaved {
                name,
                request_login,
                ..
            }) => {
                assert_eq!(name, "Team");
                assert!(!request_login);
            }
            other => panic!("expected ProfileSaved effect, got {other:?}"),
        }
        assert!(app.profiles.profiles.iter().any(|p| p.name == "Team"));
        assert!(app.profile_form.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bang_opens_terminal_mode_and_enter_requests_command() {
        let mut app = app_with_session(); // session cwd /tmp exists on disk
        app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::QuickCommand);
        assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Terminal);

        for c in "echo hi".chars() {
            app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::Table);
        let req = app.terminal_request.take().expect("terminal request");
        assert_eq!(req.cwd, PathBuf::from("/tmp"));
        assert_eq!(req.command, "echo hi");
        // `!` commands keep the post-exit keypress wait (output must stay readable).
        assert_eq!(req.kind, TerminalKind::Command);
        assert_eq!(
            app.terminal_history.first().map(String::as_str),
            Some("echo hi")
        );
    }

    #[test]
    fn edit_config_requests_editor_without_pause() {
        let mut app = empty_app();
        app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::QuickCommand);
        for c in "config.toml".chars() {
            app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::Table);
        let req = app.terminal_request.take().expect("terminal request");
        assert_eq!(req.cwd, crate::config::config_base_dir());
        assert!(req.command.ends_with("config.toml'"), "{}", req.command);
        // Interactive editors leave no output to read: return without a keypress;
        // failures offer the vim fallback in the handover.
        assert_eq!(req.kind, TerminalKind::EditConfig);
    }

    #[test]
    fn bang_requires_selected_session_with_existing_folder() {
        let mut app = empty_app();
        app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.quick.is_none());
        assert!(app.status_msg.is_some());
    }

    #[test]
    fn quick_mode_switch_only_on_empty_input() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Palette);

        // Empty-input `!` switches palette -> terminal.
        app.on_key_quick(key(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Terminal);

        // With text present, `:` is an ordinary character (no switch).
        app.on_key_quick(key(KeyCode::Char('x'), KeyModifiers::NONE));
        app.on_key_quick(key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Terminal);
        assert_eq!(app.quick.as_ref().unwrap().input.value, "x:");

        // Clearing the input re-enables switching terminal -> palette.
        app.on_key_quick(key(KeyCode::Backspace, KeyModifiers::NONE));
        app.on_key_quick(key(KeyCode::Backspace, KeyModifiers::NONE));
        app.on_key_quick(key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().mode, QuickMode::Palette);
    }

    #[test]
    fn terminal_history_recall_fills_input_and_restores_typed_text() {
        let mut app = app_with_session();
        app.terminal_history = vec!["git status".to_string(), "ls".to_string()];
        app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));

        // Down recalls the most recent command into the editable input.
        app.on_key_quick(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().term_selected, Some(0));
        assert_eq!(app.quick.as_ref().unwrap().input.value, "git status");
        app.on_key_quick(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().input.value, "ls");

        // Moving back above the list restores the (empty) typed text.
        app.on_key_quick(key(KeyCode::Up, KeyModifiers::NONE));
        app.on_key_quick(key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.quick.as_ref().unwrap().term_selected, None);
        assert_eq!(app.quick.as_ref().unwrap().input.value, "");

        // Typing filters the history list and detaches the selection.
        app.on_key_quick(key(KeyCode::Char('g'), KeyModifiers::NONE));
        let state = app.quick.as_ref().unwrap();
        assert_eq!(state.term_items, vec!["git status".to_string()]);
        assert_eq!(state.term_selected, None);
    }

    #[test]
    fn ctrl_r_opens_rename_modal() {
        let mut app = app_with_session();

        app.on_key_table(key(KeyCode::Char('r'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, UiMode::Rename);
    }

    #[test]
    fn session_profile_root_resolves_owning_profile_path() {
        let mut app = app_with_session();
        app.profiles.profiles.push(crate::profile::Profile {
            id: "profile-team".to_string(),
            agent: Agent::Codex,
            name: "Team".to_string(),
            path: PathBuf::from("/tmp/codex-team"),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        });
        app.sessions[0].profile_id = "profile-team".to_string();

        assert_eq!(
            app.session_profile_root(&app.sessions[0]),
            Some(PathBuf::from("/tmp/codex-team"))
        );

        // Unknown profile id must resolve to None (no default-root fallback:
        // that would write title metadata into the wrong account store).
        app.sessions[0].profile_id = "ghost".to_string();
        assert_eq!(app.session_profile_root(&app.sessions[0]), None);
    }

    #[test]
    fn rename_fails_without_owning_profile() {
        let mut app = app_with_session(); // profile store is empty -> no owning profile
        app.on_key_table(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::Rename);

        app.confirm_rename();

        // Must abort before touching any metadata store and keep the modal open.
        assert_eq!(app.mode, UiMode::Rename);
        assert!(app.pending_effect.is_none());
        assert!(matches!(
            app.status_msg.as_deref(),
            Some(msg) if msg.starts_with("Rename failed: session profile not found")
        ));
    }

    #[test]
    fn confirm_rename_enqueues_effect_when_valid() {
        let mut app = app_with_session();
        // Give the session an owning profile so pre-flight validation passes.
        app.profiles.profiles.push(crate::profile::Profile {
            id: "p1".to_string(),
            agent: Agent::Codex,
            name: "P1".to_string(),
            path: PathBuf::from("/tmp/codex-p1"),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        });
        app.sessions[0].profile_id = "p1".to_string();

        app.on_key_table(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::Rename);
        // The modal pre-fills the current title; confirm it unchanged.
        let expected_title = app.sessions[0].title();
        // Move focus to the buttons and confirm with the OK button focused.
        app.on_key_rename_modal(key(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key_rename_modal(key(KeyCode::Enter, KeyModifiers::NONE));

        // The handler only enqueues; the dialog stays open until the boundary
        // runs the effect (and closes it only on rename success).
        assert_eq!(
            app.pending_effect,
            Some(AppEffect::RenameSession {
                idx: 0,
                title: expected_title,
            })
        );
        assert_eq!(app.mode, UiMode::Rename);
        assert!(app.rename_modal.is_some());
    }

    #[test]
    fn ctrl_u_updates_sessions_without_entering_rename_mode() {
        let mut app = empty_app();

        app.on_key_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));

        // The handler only enqueues the effect; the rescan/status run at the boundary.
        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));

        app.apply_effect();
        assert!(app.pending_effect.is_none());
        assert!(matches!(
            app.status_msg.as_deref(),
            Some(msg) if msg.starts_with("session update complete · ")
        ));
    }

    #[test]
    fn quit_requires_two_presses() {
        let mut app = app_with_session();

        app.on_key_table(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.should_quit);
        assert_eq!(
            app.status_msg.as_deref(),
            Some("Press q or ctrl+c again to quit")
        );

        app.on_key_table(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit);
    }

    #[test]
    fn quit_keys_are_ignored_during_grace_after_handover() {
        let mut app = app_with_session();
        app.begin_quit_grace();
        let initial_grace = app.quit_grace_until.unwrap();

        // During the grace period, exits do not trigger even on rapid key spam (Ctrl+C x 2), extending grace period.
        app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.quit_armed);
        assert!(!app.should_quit);
        assert!(app.quit_grace_until.unwrap() >= initial_grace);

        // Restores normal "press twice to exit" behavior after grace period expires (user halts spamming).
        app.quit_grace_until =
            Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
        app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.quit_armed);
        app.on_key_table(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn tab_no_longer_toggles_focus_in_session_table() {
        let mut app = app_with_session();

        app.on_key_table(key(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn arrow_keys_move_column_focus_in_session_screen() {
        let mut app = app_with_session();
        assert_eq!(app.focus, Focus::Table);

        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Preview);

        app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Table);
        assert_eq!(app.screen, Screen::Session);
    }

    #[test]
    fn ctrl_u_is_global_across_profile_and_detail_screens() {
        // Profile view: refreshes both session list and usage statistics.
        let mut app = empty_app();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));
        app.apply_effect();
        assert!(matches!(
            app.status_msg.as_deref(),
            Some(msg) if msg.starts_with("session update complete · ")
        ));

        // Details view: returning to search view if target session vanishes (e.g. empty profile rescan) post-update.
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Detail);
        app.on_key_detail(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.pending_effect, Some(AppEffect::RefreshAll));
        app.apply_effect();
        assert!(matches!(
            app.status_msg.as_deref(),
            Some(msg) if msg.starts_with("session update complete · ")
        ));
        assert_eq!(app.screen, Screen::Session);
        assert!(app.detail.is_none());
    }

    #[test]
    fn left_right_switch_between_profile_and_session_screens() {
        let mut app = app_with_session();

        // Session list (with table focus) -> Left key -> Profile list screen.
        app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Profile);

        // Profile view -> Right key -> Session list screen (independent of selected profile).
        app.on_key_profile_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Session);
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn right_key_on_preview_opens_detail_screen() {
        let mut app = app_with_session();

        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE)); // focus preview
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE)); // enter details

        assert_eq!(app.screen, Screen::Detail);
        let d = app.detail.as_ref().expect("detail state");
        assert_eq!(d.focus, DetailFocus::Questions);
        assert_eq!(d.turns.len(), 1);
        assert_eq!(d.turns[0].user, "hello");
    }

    #[test]
    fn detail_question_selection_and_work_scroll() {
        let mut app = app_with_session();
        app.sessions[0]
            .user_turns
            .push("second question".to_string());
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().turns.len(), 2);

        // Left column (questions): Down key moves selection.
        app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().selected, 1);

        // Right key focuses right details column; Up/Down keys scroll details panel.
        app.on_key_detail(key(KeyCode::Right, KeyModifiers::NONE));
        {
            let d = app.detail.as_ref().unwrap();
            assert_eq!(d.focus, DetailFocus::Work);
            d.right_max_scroll.set(10); // Simulate calculations performed during render frame.
        }
        app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().right_scroll.get(), 1);

        // Right column focus -> Left key -> focuses left panel; left panel focus -> Left key -> returns to search view (table focus).
        app.on_key_detail(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.detail.as_ref().unwrap().focus, DetailFocus::Questions);
        app.on_key_detail(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Session);
        assert!(app.detail.is_none());
        assert_eq!(app.focus, Focus::Table);
    }

    #[test]
    fn detail_dot_toggles_tool_visibility() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert!(!app.detail_show_tools); // Hidden by default.

        app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
        assert!(app.detail_show_tools);

        app.on_key_detail(key(KeyCode::Char('.'), KeyModifiers::NONE));
        assert!(!app.detail_show_tools);
    }

    #[test]
    fn detail_enter_requests_resume() {
        // Since cwd "/tmp" exists in app_with_session, resume request should be configured.
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));

        app.on_key_detail(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.resume_request, Some(0));
    }

    #[test]
    fn detail_ctrl_r_opens_rename_for_detail_session() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));

        app.on_key_detail(key(KeyCode::Char('r'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, UiMode::Rename);
        assert_eq!(app.rename_target, Some(0));
        // Esc cancels and returns back to details view (table mode), clearing target session index.
        app.on_key_rename_modal(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.screen, Screen::Detail);
        assert_eq!(app.rename_target, None);
    }

    /// Instantiates App with two sessions linked to temporary source files (for testing deletion).
    fn app_with_two_deletable_sessions() -> (App, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "s7s-ui-delete-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let make = |name: &str| {
            let path = root.join(name);
            std::fs::write(&path, "{}").expect("write source");
            Session {
                agent: Agent::Codex,
                profile_id: String::new(),
                id: name.to_string(),
                source_path: Some(path),
                cwd: PathBuf::from("/tmp"),
                folder: "tmp".to_string(),
                mtime_ms: 0,
                ctime_ms: 0,
                size_bytes: 0,
                user_turns: vec!["hello".to_string()],
                search_blob: "hello".to_string(),
                assistant_blob: String::new(),
                title_hint: Some(name.to_string()),
                title_fixed: false,
            }
        };
        let app = App::new(
            Config::load(),
            test_profiles(),
            vec![make("s1.jsonl"), make("s2.jsonl")],
            "2 sessions".to_string(),
        );
        (app, root)
    }

    #[test]
    fn detail_delete_returns_to_session_screen_with_next_selected() {
        let (mut app, root) = app_with_two_deletable_sessions();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Detail);

        // Ctrl+D -> opens session deletion confirmation modal -> moves focus to Delete button -> confirm.
        app.on_key_detail(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::DeleteConfirm);
        app.on_key_delete_confirm(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_delete_confirm(key(KeyCode::Enter, KeyModifiers::NONE));

        // Confirm only enqueues the effect; the filesystem removal and screen
        // return happen when the boundary runs it.
        assert_eq!(
            app.pending_effect,
            Some(AppEffect::DeleteSession { idx: 0 })
        );
        app.apply_effect();

        // Returns to search screen, selecting the next session (s2).
        assert_eq!(app.screen, Screen::Session);
        assert!(app.detail.is_none());
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.sessions[0].id, "s2.jsonl");
        assert_eq!(app.selected, 0);
        assert_eq!(app.focus, Focus::Table);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn detail_selection_change_resets_work_scroll() {
        let mut app = app_with_session();
        app.sessions[0]
            .user_turns
            .push("second question".to_string());
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        {
            let d = app.detail.as_ref().unwrap();
            d.right_max_scroll.set(10);
            d.right_scroll.set(5);
        }

        app.on_key_detail(key(KeyCode::Down, KeyModifiers::NONE));

        let d = app.detail.as_ref().unwrap();
        assert_eq!(d.selected, 1);
        assert_eq!(d.right_scroll.get(), 0);
    }

    #[test]
    fn detail_esc_stays_on_detail_screen() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Detail);

        app.on_key_detail(key(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(app.screen, Screen::Detail);
        assert!(app.detail.is_some());
    }

    #[test]
    fn search_tab_closes_prompt_and_focuses_preview() {
        let mut app = app_with_session();
        app.mode = UiMode::Keyword;
        app.filter.keyword = "hello".to_string();

        app.on_key_keyword(key(KeyCode::Tab, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.focus, Focus::Preview);
        assert_eq!(app.filter.keyword, "hello");
    }

    #[test]
    fn search_backtab_closes_prompt_and_focuses_table() {
        let mut app = app_with_session();
        app.mode = UiMode::Keyword;
        app.focus = Focus::Preview;
        app.filter.keyword = "hello".to_string();

        app.on_key_keyword(key(KeyCode::BackTab, KeyModifiers::SHIFT));

        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.focus, Focus::Table);
        assert_eq!(app.filter.keyword, "hello");
    }

    #[test]
    fn search_arrow_keys_move_cursor_and_edit_mid_string() {
        let mut app = app_with_session();
        app.mode = UiMode::Keyword;
        app.filter.keyword = "helo".to_string();
        app.keyword_cursor = app.filter.keyword.len();

        // Move cursor left twice, insert missing 'l' -> resolves to "hello".
        app.on_key_keyword(key(KeyCode::Left, KeyModifiers::NONE));
        app.on_key_keyword(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.keyword_cursor, 2);
        app.on_key_keyword(key(KeyCode::Char('l'), KeyModifiers::NONE));
        assert_eq!(app.filter.keyword, "hello");
        assert_eq!(app.keyword_cursor, 3);

        // Verify Home/End navigation and Backspace/Delete actions at current cursor positions.
        app.on_key_keyword(key(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(app.keyword_cursor, 0);
        app.on_key_keyword(key(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.filter.keyword, "ello");
        app.on_key_keyword(key(KeyCode::End, KeyModifiers::NONE));
        app.on_key_keyword(key(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.filter.keyword, "ell");
        assert_eq!(app.keyword_cursor, app.filter.keyword.len());
    }

    fn app_with_cwd(cwd: &str) -> App {
        App::new(
            Config::load(),
            test_profiles(),
            vec![Session {
                agent: Agent::Codex,
                profile_id: String::new(),
                id: "session-1".to_string(),
                source_path: None,
                cwd: PathBuf::from(cwd),
                folder: "x".to_string(),
                mtime_ms: 0,
                ctime_ms: 0,
                size_bytes: 0,
                user_turns: vec!["hi".to_string()],
                search_blob: "hi".to_string(),
                assistant_blob: String::new(),
                title_hint: Some("hi".to_string()),
                title_fixed: false,
            }],
            "1 sessions · reparsed 0/0".to_string(),
        )
    }

    #[test]
    fn resume_blocked_when_folder_missing_shows_message() {
        let mut app = app_with_cwd("/no/such/dir/s7s-xyz");

        app.on_key_table(key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(app.resume_request.is_none());
        assert_eq!(app.mode, UiMode::Message);
        assert!(app.message.is_some());
    }

    #[test]
    fn resume_allowed_when_folder_exists() {
        // Since "/tmp" exists on disk, resume request should be configured.
        let mut app = app_with_cwd("/tmp");

        app.on_key_table(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.resume_request, Some(0));
        assert_eq!(app.mode, UiMode::Table);
    }

    #[test]
    fn message_dialog_dismisses_to_previous_mode() {
        let mut app = app_with_session();
        app.show_message("t", vec!["body".to_string()], MessageKind::Error);
        assert_eq!(app.mode, UiMode::Message);

        app.on_key_message(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::Table);
        assert!(app.message.is_none());
    }

    /// Returns App with a built-in Claude profile, one custom profile, and one session per profile.
    fn app_with_profiles() -> App {
        use crate::profile::{Profile, ProfileStore};
        let profiles = ProfileStore {
            profiles: vec![
                Profile {
                    id: "builtin-claude".to_string(),
                    agent: Agent::Claude,
                    name: "Claude".to_string(),
                    path: PathBuf::from("/tmp"),
                    oauth_token: None,
                    active: true,
                    shortcut: Some(1),
                    builtin: true,
                },
                Profile {
                    id: "profile-x".to_string(),
                    agent: Agent::Claude,
                    name: "Team".to_string(),
                    path: PathBuf::from("/tmp/team"),
                    oauth_token: None,
                    active: true,
                    shortcut: Some(2),
                    builtin: false,
                },
            ],
        };
        let session = |pid: &str, id: &str, cwd: &str| Session {
            agent: Agent::Claude,
            profile_id: pid.to_string(),
            id: id.to_string(),
            source_path: None,
            cwd: PathBuf::from(cwd),
            folder: PathBuf::from(cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| cwd.to_string()),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hi".to_string()],
            search_blob: "hi".to_string(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        };
        App::new(
            Config::load(),
            profiles,
            vec![
                session("builtin-claude", "s1", "/"),
                session("profile-x", "s2", "/tmp"),
            ],
            "2 sessions · reparsed 0/0".to_string(),
        )
    }

    /// Opens the new-session dialog from the Session view, then normalizes focus to the
    /// (closed) Profile dropdown — the shared starting point for focus/dropdown flow tests.
    /// The Session view itself now opens on the OK button; that behavior has its own
    /// regression test (`ctrl_n_in_session_screen_focuses_ok_button`).
    fn open_new_session_at_profile_focus(app: &mut App) {
        app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        app.new_session.as_mut().expect("new session dialog").focus = NewSessionFocus::Profile;
    }

    #[test]
    fn colon_opens_quick_command() {
        let mut app = app_with_profiles();

        app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::QuickCommand);
        assert!(app.quick.is_some());

        // Esc closes the palette and restores prior table mode.
        app.on_key_quick(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.quick.is_none());

        // Screen transitions are driven by ←/→ keys; Esc does not change the active screen.
        app.on_key_table(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Profile);
        app.on_key_profile_table(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Profile);
        app.on_key_profile_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Session);
    }

    #[test]
    fn question_mark_opens_help_without_joining_screen_rotation() {
        let mut app = app_with_profiles();

        app.on_key_table(key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Help);
        assert_eq!(app.screen, Screen::Session);

        app.on_key_help(key(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Help);
        assert_eq!(app.screen, Screen::Session);

        app.on_key_help(key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.screen, Screen::Session);
    }

    #[test]
    fn profile_screen_question_mark_help_returns_to_profile_table() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;

        app.on_key_profile_table(key(KeyCode::Char('?'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Help);
        assert_eq!(app.screen, Screen::Profile);

        app.on_key_help(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.screen, Screen::Profile);
    }

    #[test]
    fn space_on_unnumbered_profile_shows_error_when_all_slots_are_full() {
        let mut app = app_with_profiles();
        let template = app.profiles.profiles[1].clone();
        for slot in 3..=5 {
            let mut profile = template.clone();
            profile.id = format!("profile-{slot}");
            profile.name = format!("Profile {slot}");
            profile.path = PathBuf::from(format!("/tmp/profile-{slot}"));
            profile.shortcut = Some(slot);
            app.profiles.profiles.push(profile);
        }
        let mut unnumbered = template;
        unnumbered.id = "profile-6".to_string();
        unnumbered.name = "Profile 6".to_string();
        unnumbered.path = PathBuf::from("/tmp/profile-6");
        unnumbered.active = false;
        unnumbered.shortcut = None;
        app.profiles.profiles.push(unnumbered);
        app.screen = Screen::Profile;
        app.profile_selected = 5;

        app.on_key_profile_table(key(KeyCode::Char(' '), KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::Message);
        assert!(app
            .message
            .as_ref()
            .is_some_and(|message| message.kind == MessageKind::Error
                && message
                    .lines
                    .iter()
                    .any(|line| line.contains("already assigned"))));
        assert_eq!(app.profiles.numbered_profiles().len(), 5);
        assert!(!app.profiles.profiles[5].active);
    }

    #[test]
    fn number_key_filters_by_active_profile() {
        let mut app = app_with_profiles();

        // <2> key: filters only by the second active profile (profile-x).
        app.on_key_table(key(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.filtered.len(), 1);
        assert_eq!(app.sessions[app.filtered[0]].profile_id, "profile-x");

        // <0> key: clears active filters.
        app.on_key_table(key(KeyCode::Char('0'), KeyModifiers::NONE));
        assert_eq!(app.filtered.len(), 2);
    }

    #[test]
    fn builtin_profile_delete_blocked_and_normal_confirmed() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;

        // Deletion of built-in profiles must be blocked with an alert dialog.
        app.profile_selected = 0;
        app.on_key_profile_table(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::Message);
        app.on_key_message(key(KeyCode::Enter, KeyModifiers::NONE));

        // Deletion of normal profiles opens the deletion confirmation modal.
        app.profile_selected = 1;
        app.on_key_profile_table(key(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::ProfileDeleteConfirm);
        // Esc cancels (skip saving validation verification tests).
        app.on_key_profile_delete_confirm(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert_eq!(app.profiles.profiles.len(), 2);
    }

    #[test]
    fn new_session_folder_dropdown_selects_then_ok_starts() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.profile_selected = 1;

        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::NewSession);
        {
            // Profile screen: starts with Folder focused (empty path) and dropdown open.
            let state = app.new_session.as_ref().expect("new session dialog");
            assert_eq!(state.focus, NewSessionFocus::Folder);
            assert!(state.dropdown_open);
            assert_eq!(state.folder_cursor, Some(0));
            assert_eq!(state.input.value, "");
            assert!(state.folders.contains(&PathBuf::from("/")));
        }

        // Down key navigates to "/tmp" option.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));

        // Enter key (dropdown open): commits selection and closes dropdown; does not trigger session launch yet.
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.new_session_request.is_none());
        {
            let state = app.new_session.as_ref().unwrap();
            assert!(!state.dropdown_open);
            assert_eq!(state.input.value, "/tmp");
        }

        // Down key (dropdown closed): moves focus from Folder to Buttons (OK focused). Enter launches session.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Buttons);
            assert!(state.ok_focused);
        }
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        let req = app
            .new_session_request
            .as_ref()
            .expect("new session request");
        assert_eq!(req.profile_id, "profile-x");
        assert_eq!(
            req.cwd,
            std::fs::canonicalize("/tmp").expect("canonicalize /tmp")
        );
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.new_session.is_none());
    }

    #[test]
    fn new_session_space_selects_folder_and_keeps_dropdown_open() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;

        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        // Profile screen: starts with Folder focused (dropdown open at cursor 0). Down key moves cursor to 1 (/tmp).
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Char(' '), KeyModifiers::NONE));

        let state = app.new_session.as_ref().expect("new session dialog");
        assert!(state.dropdown_open); // Space key keeps dropdown open.
        assert_eq!(state.input.value, "/tmp"); // Synced back to input text box immediately.
                                               // Reordering post-selection must keep the cursor tracking same folder.
        let cursor_folder = state
            .folder_cursor
            .and_then(|c| state.ordered.get(c))
            .and_then(|&i| state.folders.get(i));
        assert_eq!(cursor_folder, Some(&PathBuf::from("/tmp")));
    }

    #[test]
    fn new_session_right_does_not_complete_when_dropdown_open() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.profile_selected = 1;

        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        // Profile screen: starts with Folder focused (dropdown open at cursor 0). Down key moves cursor to 1 (/tmp).
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE));

        // → selection & completion removed: dropdown stays open and input text is unchanged.
        let state = app.new_session.as_ref().expect("new session dialog");
        assert!(state.dropdown_open);
        assert_eq!(state.input.value, "");
        assert_eq!(state.folder_cursor, Some(1));
    }

    #[test]
    fn new_session_right_opens_both_dropdowns_when_closed() {
        let mut app = app_with_profiles();
        app.selected = 1; // directory populated -> focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );

        // → key (Profile closed): opens dropdown and places cursor on current selection.
        app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert!(state.dropdown_open);
            assert_eq!(state.profile_cursor, state.profile_idx);
        }

        // Esc closes it, then move to Folder focus and reopen using →.
        app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
        assert!(!app.new_session.as_ref().unwrap().dropdown_open);

        // → key (Folder closed): opens dropdown and highlights the first option.
        app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE));
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Folder);
        assert!(state.dropdown_open);
        assert_eq!(state.folder_cursor, Some(0));
    }

    #[test]
    fn new_session_typing_autoopens_and_reorders_matches_first() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;

        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        app.on_key_new_session(key(KeyCode::Char('t'), KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Char('m'), KeyModifiers::NONE));

        let state = app.new_session.as_ref().expect("new session dialog");
        assert_eq!(state.input.value, "tm");
        assert!(state.dropdown_open); // Auto-opens dropdown on typing.
                                      // Matching option ("/tmp") at top, non-matching ("/") preserved at bottom.
        assert_eq!(state.match_count, 1);
        assert_eq!(state.ordered.len(), state.folders.len());
        assert_eq!(state.folders[state.ordered[0]], PathBuf::from("/tmp"));
        assert_eq!(state.folders[state.ordered[1]], PathBuf::from("/"));
    }

    #[test]
    fn new_session_tab_toggles_focus_and_closes_dropdown() {
        let mut app = app_with_profiles();
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );

        // Tab key with dropdown open -> closes dropdown and moves focus to Model.
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.new_session.as_ref().unwrap().dropdown_open);
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Model);
            assert!(!state.dropdown_open);
        }

        // Shift+Tab key returns focus back to Profile.
        app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );
    }

    #[test]
    fn new_session_tab_commits_dropdown_selection_before_moving_focus() {
        let mut app = app_with_profiles();
        app.selected = 1; // profile-x, /tmp — focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);

        // Profile dropdown: moves cursor to 0 (builtin-claude) then Tab ->
        // commits selection and shifts focus to Model.
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.profile_idx, 0);
            assert_eq!(state.focus, NewSessionFocus::Model);
            assert!(!state.dropdown_open);
        }

        // Folder dropdown: committed selection reflects on text box, shifting focus back via Shift+Tab.
        // Sort order by input "/tmp": ["/tmp" (match), "/"] - second item is "/".
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 0=/tmp)
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // cursor 1=/
        app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.input.value, "/");
        assert_eq!(state.focus, NewSessionFocus::Model);
        assert!(!state.dropdown_open);
    }

    #[test]
    fn new_session_buttons_row_ok_and_cancel() {
        let mut app = app_with_profiles();
        app.selected = 1; // input "/tmp", focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);

        // Tab x 3: Profile -> Model -> Folder -> OK (first). Tab again -> Cancel.
        // Shift+Tab -> OK.
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Buttons);
            assert!(state.ok_focused); // OK is the initial button stop.
        }
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        assert!(!app.new_session.as_ref().unwrap().ok_focused); // -> Cancel
        app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert!(app.new_session.as_ref().unwrap().ok_focused); // returns to OK.

        // Shifts to Cancel then Enter -> closes dialog without starting.
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // OK -> Cancel
        assert!(!app.new_session.as_ref().unwrap().ok_focused);
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.new_session.is_none());
        assert!(app.new_session_request.is_none());
        assert_eq!(app.mode, UiMode::Table);

        // Reopens and Shift+Tab wraps back: Profile -> Cancel (the last stop).
        open_new_session_at_profile_focus(&mut app);
        app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Buttons);
            assert!(!state.ok_focused);
        }

        // Cycles forward via Tab to OK, then Enter -> launches session.
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Cancel -> Profile
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Folder -> OK
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Buttons);
            assert!(state.ok_focused);
        }
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        let req = app
            .new_session_request
            .as_ref()
            .expect("new session request");
        assert_eq!(req.profile_id, "profile-x");
    }

    #[test]
    fn new_session_profile_dropdown_space_enter_up_flow() {
        let mut app = app_with_profiles();
        app.selected = 1; // s2: profile-x -> default profile_idx 1, focus normalized to Profile.
        open_new_session_at_profile_focus(&mut app);

        // Enter: opens dropdown placing cursor on the active profile selection.
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert!(state.dropdown_open);
            assert_eq!(state.profile_cursor, 1);
        }

        // Up key moves to top item, then Space: selects it while keeping dropdown open.
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Char(' '), KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert!(state.dropdown_open);
            assert_eq!(state.profile_idx, 0);
        }

        // Up key on top item (cursor 0): cycles to bottom item without closing dropdown.
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert!(state.dropdown_open); // Stays open.
            assert_eq!(state.profile_cursor, 1); // 0 -> bottom index.
        }

        // Down key on bottom item (cursor 1): cycles back to top (0), then Down key returns to 1.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.new_session.as_ref().unwrap().profile_cursor, 0);
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.new_session.as_ref().unwrap().profile_cursor, 1);

        // Enter: commits selection and closes dropdown (does not start session).
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        let state = app.new_session.as_ref().unwrap();
        assert!(!state.dropdown_open);
        assert_eq!(state.profile_idx, 1);
        assert!(app.new_session_request.is_none()); // Enter selection does not launch session.
    }

    #[test]
    fn new_session_esc_closes_dropdown_first_then_dialog() {
        let mut app = app_with_profiles();
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);

        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // move cursor
        app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().expect("dialog stays open");
            assert!(!state.dropdown_open);
            assert_eq!(state.profile_idx, 1); // Esc key does not commit active cursor selection.
        }

        app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.new_session.is_none());
    }

    #[test]
    fn new_session_folder_updown_wraps_around() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile; // initially focused on Folder (empty folders).
        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        // Profile screen: starts dropdown open (cursor 0). Up key cycles from top to bottom.
        assert_eq!(app.new_session.as_ref().unwrap().folder_cursor, Some(0));
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        let last = {
            let state = app.new_session.as_ref().unwrap();
            assert!(state.dropdown_open);
            state.ordered.len() - 1
        };
        assert_eq!(app.new_session.as_ref().unwrap().folder_cursor, Some(last));

        // Down key cycles from bottom back to top (0).
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.new_session.as_ref().unwrap().folder_cursor, Some(0));
    }

    #[test]
    fn new_session_text_input_allows_plain_k_character() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;

        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        app.on_key_new_session(key(KeyCode::Char('k'), KeyModifiers::NONE));

        let state = app.new_session.as_ref().expect("new session dialog");
        assert_eq!(state.input.value, "k");
    }

    #[test]
    fn profile_enter_no_longer_opens_new_session_dialog() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;

        app.on_key_profile_table(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::Table);
        assert!(app.new_session.is_none());
    }

    #[test]
    fn ctrl_n_in_profile_screen_opens_dialog_with_selected_profile_and_empty_folder() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.profile_selected = 1;

        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, UiMode::NewSession);
        let state = app.new_session.as_ref().expect("new session dialog");
        assert_eq!(state.profile_idx, 1);
        assert_eq!(state.input.value, "");
        // Starts with Folder focused and dropdown open due to empty folders.
        assert_eq!(state.focus, NewSessionFocus::Folder);
        assert!(state.dropdown_open);
        assert_eq!(state.folder_cursor, Some(0));
    }

    #[test]
    fn ctrl_n_in_session_screen_focuses_ok_button() {
        let mut app = app_with_profiles();
        // Selects the second session (s2: profile-x, /tmp).
        app.selected = 1;

        app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, UiMode::NewSession);
        let state = app.new_session.as_ref().expect("new session dialog");
        assert_eq!(state.profile_idx, 1); // profile-x
        assert_eq!(state.input.value, "/tmp");
        // Session view opens with the OK button focused for a quick start.
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(state.ok_focused);
        assert!(!state.dropdown_open);
    }

    #[test]
    fn ctrl_n_in_detail_screen_defaults_to_detail_session_profile() {
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Detail);

        app.on_key_detail(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        assert_eq!(app.mode, UiMode::NewSession);
        let state = app.new_session.as_ref().expect("new session dialog");
        assert_eq!(state.profile_idx, 1);
        assert_eq!(state.input.value, "/tmp");
        // Detail view keeps the Profile dropdown focused (unlike the Session view).
        assert_eq!(state.focus, NewSessionFocus::Profile);
    }

    #[test]
    fn ctrl_n_opens_ordinary_dialog_without_context() {
        let mut app = app_with_profiles();
        app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        let state = app.new_session.as_ref().expect("dialog");
        assert!(state.context.is_none());
    }

    #[test]
    fn ctrl_shift_n_opens_contextual_dialog_with_selected_source() {
        let mut app = app_with_profiles();
        app.selected = 1; // s2: profile-x, /tmp

        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));

        assert_eq!(app.mode, UiMode::NewSession);
        let state = app.new_session.as_ref().expect("dialog");
        let ctx = state.context.as_ref().expect("context captured");
        assert_eq!(ctx.agent, Agent::Claude);
        assert_eq!(ctx.profile_id, "profile-x");
        assert_eq!(ctx.session_id, "s2");
        // Target defaults mirror ordinary New Session (source session's profile/cwd).
        assert_eq!(state.profile_idx, 1);
        assert_eq!(state.input.value, "/tmp");
    }

    #[test]
    fn ctrl_shift_n_accepts_uppercase_char_form() {
        // Enhanced-keyboard terminals may report the chord as Char('N').
        let mut app = app_with_profiles();
        app.on_key_table(key(
            KeyCode::Char('N'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(app
            .new_session
            .as_ref()
            .is_some_and(|s| s.context.is_some()));
    }

    #[test]
    fn ctrl_shift_n_without_focused_session_is_rejected() {
        let mut app = empty_app();
        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.new_session.is_none());
        assert_eq!(app.status_msg.as_deref(), Some("Select a session first"));
    }

    #[test]
    fn profile_screen_ctrl_shift_n_is_not_captured_as_ordinary_new_session() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(app.new_session.is_none());
    }

    #[test]
    fn detail_ctrl_shift_n_captures_detail_source_session() {
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen, Screen::Detail);

        app.on_key_detail(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));

        let ctx = app
            .new_session
            .as_ref()
            .and_then(|s| s.context.as_ref())
            .expect("context from detail source");
        assert_eq!(ctx.session_id, "s2");
    }

    #[test]
    fn changing_target_profile_preserves_source_context() {
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        let before = app.new_session.as_ref().unwrap().context.clone().unwrap();

        // Switch the target profile via the dropdown (open -> up -> enter).
        app.new_session.as_mut().unwrap().focus = NewSessionFocus::Profile;
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        // Edit the folder input as well.
        let state = app.new_session.as_mut().unwrap();
        state.focus = NewSessionFocus::Folder;
        app.on_key_new_session(key(KeyCode::Char('x'), KeyModifiers::NONE));

        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.profile_idx, 0); // target changed
        assert_eq!(state.context.as_ref(), Some(&before)); // source immutable
    }

    #[test]
    fn ok_transfers_context_into_request_and_cancel_discards_it() {
        // OK path.
        let mut app = app_with_profiles();
        app.selected = 1; // cwd /tmp exists on disk
        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert!(app.new_session.as_ref().unwrap().ok_focused);
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        let req = app.new_session_request.take().expect("request");
        let ctx = req.context.expect("context travels with the request");
        assert_eq!(ctx.session_id, "s2");
        assert_eq!(ctx.profile_id, "profile-x");

        // Cancel path.
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.new_session.is_none());
        assert!(app.new_session_request.is_none());
    }

    #[test]
    fn contextual_ok_aborts_when_source_session_disappeared() {
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        // Source disappears while the dialog is open (delete/refresh race).
        app.sessions.retain(|s| s.id != "s2");
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(app.new_session_request.is_none());
        let state = app.new_session.as_ref().expect("dialog stays open");
        assert!(state
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Source session not found")));
    }

    #[test]
    fn quick_command_invokes_contextual_opener() {
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
        for c in "attach-session".chars() {
            app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::NewSession);
        let ctx = app
            .new_session
            .as_ref()
            .and_then(|s| s.context.as_ref())
            .expect("palette opens contextual dialog");
        assert_eq!(ctx.session_id, "s2");
    }

    #[test]
    fn contextual_source_control_is_not_focusable() {
        let mut app = app_with_profiles();
        app.selected = 1;
        app.on_key_table(key(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        let state = app.new_session.as_mut().expect("contextual dialog");
        state.focus = NewSessionFocus::Profile;

        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Model
        );
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Model
        );
    }

    #[test]
    fn new_session_ctrl_n_p_no_longer_cycle_profile() {
        // Profile selection is unified into dropdown - Ctrl+N/P cycle shortcuts are removed.
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.profile_selected = 0;
        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        app.on_key_new_session(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        app.on_key_new_session(key(KeyCode::Char('p'), KeyModifiers::CONTROL));

        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.profile_idx, 0);
        assert_eq!(state.input.value, ""); // Ctrl key combinations are not written to the text input.
    }

    #[test]
    fn new_session_enter_with_empty_folder_shows_error() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        // Closes open dropdown using Esc leaving folder input empty, moves focus to OK button via Down key, then Enter.
        app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert!(app.new_session.as_ref().unwrap().ok_focused);
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(app.mode, UiMode::NewSession);
        assert!(app.new_session_request.is_none());
        let state = app.new_session.as_ref().expect("dialog stays open");
        assert_eq!(state.error.as_deref(), Some("Select a folder first"));
    }

    #[test]
    fn bare_project_name_detection() {
        assert!(is_bare_project_name("myproj"));
        assert!(is_bare_project_name("my.proj"));
        assert!(is_bare_project_name("my proj"));
        assert!(!is_bare_project_name("foo/bar"));
        assert!(!is_bare_project_name("./foo"));
        assert!(!is_bare_project_name("~/foo"));
        assert!(!is_bare_project_name("~foo"));
    }

    #[test]
    fn new_session_bare_name_missing_opens_project_dir_confirm_and_cancel_returns() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        let name = format!("s7s-test-missing-project-{}", std::process::id());
        app.new_session.as_mut().unwrap().input.value = name.clone();
        app.confirm_new_session();

        assert_eq!(app.mode, UiMode::ProjectDirConfirm);
        assert!(app.dir_create_ok_focused);
        assert_eq!(
            app.project_dir_pending.as_deref(),
            Some(crate::config::projects_dir().join(&name).as_path())
        );
        assert!(app.new_session_request.is_none());

        // Cancel returns to the New Session dialog with the typed name kept.
        app.on_key_project_dir_confirm(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::NewSession);
        assert!(app.project_dir_pending.is_none());
        assert_eq!(app.new_session.as_ref().unwrap().input.value, name);
        assert!(app.new_session_request.is_none());
    }

    #[test]
    fn project_dir_create_makes_folder_and_starts_session() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        // Pending path stands in for projects_dir()/<name> so the test never
        // touches the real user config directory.
        let dir =
            std::env::temp_dir().join(format!("s7s-test-project-create-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        app.project_dir_pending = Some(dir.clone());
        app.mode = UiMode::ProjectDirConfirm;
        app.dir_create_ok_focused = true;

        app.on_key_project_dir_confirm(key(KeyCode::Enter, KeyModifiers::NONE));

        assert!(dir.is_dir());
        let req = app.new_session_request.as_ref().expect("request issued");
        assert_eq!(req.cwd, std::fs::canonicalize(&dir).unwrap());
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.new_session.is_none());
        assert!(app.project_dir_pending.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_session_missing_path_input_still_errors() {
        let mut app = app_with_profiles();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

        // Path-form input (contains a separator) must keep the current error behavior
        // instead of offering project creation.
        app.new_session.as_mut().unwrap().input.value = "/definitely/missing/s7s-path".to_string();
        app.confirm_new_session();

        assert_eq!(app.mode, UiMode::NewSession);
        assert!(app.project_dir_pending.is_none());
        assert!(app.new_session_request.is_none());
        let state = app.new_session.as_ref().expect("dialog stays open");
        assert!(state
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("Cannot open path")));
    }

    #[test]
    fn new_session_enter_uses_profile_selected_in_dropdown() {
        let mut app = app_with_profiles();
        app.selected = 1; // s2: profile-x, /tmp - focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);

        // Changes profile to builtin-claude (index 0) in dropdown, closes it, and launches via Enter on OK button.
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Profile -> Model
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Model -> Folder
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Folder -> Buttons(OK)
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start

        let req = app
            .new_session_request
            .as_ref()
            .expect("new session request");
        assert_eq!(req.profile_id, "builtin-claude");
        assert_eq!(
            req.cwd,
            std::fs::canonicalize("/tmp").expect("canonicalize /tmp")
        );
    }

    #[test]
    fn new_session_updown_move_focus_when_dropdown_closed() {
        let mut app = app_with_profiles();
        app.selected = 1; // folder populated -> focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );

        // Down key: Profile -> Model -> Folder -> Buttons. Dropdown is not opened.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Model);
            assert!(!state.dropdown_open);
        }
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Folder
        );
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Buttons
        );
        // Down key at bottom wraps back to the top (rotation, like Tab).
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );

        // Up key at top wraps to the button row, focusing OK first.
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.focus, NewSessionFocus::Buttons);
            assert!(state.ok_focused);
        }

        // Up key: Buttons -> Folder -> Model -> Profile.
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Folder
        );
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Model
        );
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Profile
        );
    }

    #[test]
    fn new_session_down_into_buttons_always_focuses_ok() {
        let mut app = app_with_profiles();
        app.selected = 1; // folder populated -> focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);

        // Moves focus to button row, moves to Cancel, climbs back to Folder, then moves Down again.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Profile -> Model
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Model -> Folder
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Folder -> Buttons(OK)
        app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE)); // OK -> Cancel
        assert!(!app.new_session.as_ref().unwrap().ok_focused);
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // Buttons -> Folder
        assert_eq!(
            app.new_session.as_ref().unwrap().focus,
            NewSessionFocus::Folder
        );

        // Entering button row again -> focuses OK first, independent of the prior Cancel selection.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(state.ok_focused);
    }

    fn model_entry(value: &str) -> crate::models::ModelEntry {
        crate::models::ModelEntry {
            value: value.to_string(),
            label: value.to_string(),
            note: String::new(),
        }
    }

    fn profile_models(
        agent: Agent,
        values: &[&str],
        default_model: Option<&str>,
    ) -> crate::models::ProfileModels {
        crate::models::ProfileModels {
            agent,
            cli_version: None,
            models: values.iter().map(|v| model_entry(v)).collect(),
            default_model: default_model.map(str::to_string),
            last_selected: None,
        }
    }

    #[test]
    fn new_session_model_dropdown_selects_and_passes_model() {
        let mut app = app_with_profiles();
        app.models.insert(
            "profile-x".to_string(),
            profile_models(Agent::Claude, &["opus", "fable", "sonnet"], Some("fable")),
        );
        app.selected = 1; // s2: profile-x, /tmp - focus normalized to Profile for this flow.
        open_new_session_at_profile_focus(&mut app);

        // Initial selection = default model from settings ("fable", index 2 including "Default").
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.model_options.len(), 4); // Default + 3
            assert_eq!(state.model_idx, 2);
            assert_eq!(state.model_options[2].value.as_deref(), Some("fable"));
        }

        // Selecting "opus" in Model dropdown then OK -> injected model configuration included in the request.
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 2)
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 1 (opus)
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
        assert_eq!(app.new_session.as_ref().unwrap().model_idx, 1);
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Model -> Folder
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Folder -> Buttons(OK)
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start
        let req = app.new_session_request.as_ref().expect("request");
        assert_eq!(req.model.as_deref(), Some("opus"));
    }

    #[test]
    fn new_session_default_model_passes_no_model() {
        let mut app = app_with_profiles();
        // If default model is absent from cache, initial selection points to "Default" (no injection).
        app.models.insert(
            "profile-x".to_string(),
            profile_models(Agent::Claude, &["opus", "sonnet"], None),
        );
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        assert_eq!(app.new_session.as_ref().unwrap().model_idx, 0);

        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Model
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Folder
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Buttons(OK)
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        let req = app.new_session_request.as_ref().expect("request");
        assert_eq!(req.model, None);
    }

    #[test]
    fn new_session_missing_default_model_disables_ok_until_reselected() {
        let mut app = app_with_profiles();
        // Default model ("legacy") absent from options catalog -> placeholder item "missing" is selected.
        app.models.insert(
            "profile-x".to_string(),
            profile_models(Agent::Claude, &["opus", "sonnet"], Some("legacy")),
        );
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        {
            let state = app.new_session.as_ref().unwrap();
            assert_eq!(state.model_idx, 1);
            assert!(state.model_options[1].missing);
        }

        // Enter on OK while "missing" is selected must block execution and only show error message.
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // -> Model
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // -> Folder
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // -> OK
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.new_session_request.is_none());
        {
            let state = app.new_session.as_ref().expect("dialog stays open");
            assert!(state.error.as_deref().unwrap().contains("Model"));
        }

        // Re-selecting to "Default" (no injection) enables execution again.
        app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT)); // OK -> Folder
        app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT)); // Folder -> Model
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0 (Default)
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Folder -> OK
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
        let req = app.new_session_request.as_ref().expect("request");
        assert_eq!(req.model, None);
    }

    #[test]
    fn new_session_profile_change_rebuilds_model_options() {
        let mut app = app_with_profiles();
        // Only builtin-claude has cached models. profile-x has no cache -> falls back to embedded Claude models
        // (fable / opus / sonnet / haiku).
        app.models.insert(
            "builtin-claude".to_string(),
            profile_models(Agent::Claude, &["opus"], Some("opus")),
        );
        app.selected = 1; // profile-x
        open_new_session_at_profile_focus(&mut app);
        assert_eq!(app.new_session.as_ref().unwrap().model_options.len(), 5);

        // Swapping profile to builtin-claude -> model options list is reconstructed.
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.profile_idx, 0);
        assert_eq!(state.model_options.len(), 2); // Default + opus
        assert_eq!(state.model_idx, 1); // Initial selection default model "opus".
    }

    #[test]
    fn new_session_last_selected_model_overrides_cli_default() {
        let mut app = app_with_profiles();
        let mut pm = profile_models(Agent::Claude, &["opus", "fable", "sonnet"], Some("fable"));
        pm.last_selected = Some(LastSelection::Model("sonnet".to_string()));
        app.models.insert("profile-x".to_string(), pm);
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        let state = app.new_session.as_ref().unwrap();
        // "sonnet" (index 3: Default + opus/fable/sonnet) beats CLI default "fable" (index 2).
        assert_eq!(state.model_idx, 3);
        assert_eq!(state.model_options[3].value.as_deref(), Some("sonnet"));
    }

    #[test]
    fn new_session_last_selected_default_overrides_cli_default() {
        let mut app = app_with_profiles();
        // CLI default "fable" would normally select index 2, but a remembered "Default" pick
        // must select index 0 and never surface a placeholder.
        let mut pm = profile_models(Agent::Claude, &["opus", "fable"], Some("fable"));
        pm.last_selected = Some(LastSelection::Default);
        app.models.insert("profile-x".to_string(), pm);
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        assert_eq!(app.new_session.as_ref().unwrap().model_idx, 0);
    }

    #[test]
    fn new_session_stale_last_selected_falls_back_to_cli_default() {
        let mut app = app_with_profiles();
        // Last pick "legacy" is no longer in the list -> skip it and use CLI default "opus".
        let mut pm = profile_models(Agent::Claude, &["opus", "sonnet"], Some("opus"));
        pm.last_selected = Some(LastSelection::Model("legacy".to_string()));
        app.models.insert("profile-x".to_string(), pm);
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.model_idx, 1); // Default + opus -> opus at 1
        assert_eq!(state.model_options[1].value.as_deref(), Some("opus"));
        assert!(state.model_options.iter().all(|o| !o.missing));
    }

    #[test]
    fn new_session_launch_records_last_selected() {
        let mut app = app_with_profiles();
        app.models.insert(
            "profile-x".to_string(),
            profile_models(Agent::Claude, &["opus", "fable", "sonnet"], Some("fable")),
        );
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        // Pick "opus" and launch.
        app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 2 = fable)
        app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 1 = opus
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Folder
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> OK
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start
        assert_eq!(
            app.new_session_request.as_ref().unwrap().model.as_deref(),
            Some("opus")
        );
        // The pick is remembered in the in-memory catalog for next time.
        assert_eq!(
            app.models
                .for_profile(&app.profiles.profiles[1])
                .and_then(|m| m.last_selected.clone()),
            Some(LastSelection::Model("opus".to_string()))
        );
    }

    #[test]
    fn new_session_launch_default_records_default_selection() {
        let mut app = app_with_profiles();
        app.models.insert(
            "profile-x".to_string(),
            profile_models(Agent::Claude, &["opus", "sonnet"], None),
        );
        app.selected = 1;
        open_new_session_at_profile_focus(&mut app);
        // Initial selection is Default (idx 0) because no CLI default; launch as-is.
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Model
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Folder
        app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> OK
        app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start
        assert_eq!(app.new_session_request.as_ref().unwrap().model, None);
        assert_eq!(
            app.models
                .for_profile(&app.profiles.profiles[1])
                .and_then(|m| m.last_selected.clone()),
            Some(LastSelection::Default)
        );
    }

    #[test]
    fn profile_form_add_blocks_antigravity_agent() {
        let mut app = empty_app();
        app.open_profile_form(None);
        let form = app.profile_form.as_mut().unwrap();
        assert!(!form.agy_allowed);
        // Agent enum order: [Claude, Codex, Antigravity] - cycling bypasses Antigravity option.
        form.cycle_agent(1);
        assert_eq!(Agent::all()[form.agent_idx], Agent::Codex);
        form.cycle_agent(1);
        assert_eq!(Agent::all()[form.agent_idx], Agent::Claude);
        form.cycle_agent(-1);
        assert_eq!(Agent::all()[form.agent_idx], Agent::Codex);
    }

    #[test]
    fn profile_form_save_rejects_new_antigravity_profile() {
        let mut app = empty_app();
        app.open_profile_form(None);
        let form = app.profile_form.as_mut().unwrap();
        // Force-selects Antigravity bypassing radio button restrictions (defensive validation test).
        form.agent_idx = 2;
        form.name.value = "Agy2".to_string();
        form.path.value = "/tmp".to_string();
        app.confirm_profile_form();
        let form = app.profile_form.as_ref().expect("form stays open");
        assert!(form.error.as_deref().unwrap().contains("Antigravity"));
        assert!(app.profiles.profiles.is_empty());
    }

    #[test]
    fn search_cursor_moves_over_multibyte_chars() {
        let mut app = app_with_session();
        app.mode = UiMode::Keyword;
        app.filter.keyword = "한글".to_string();
        app.keyword_cursor = app.filter.keyword.len();

        // Must traverse by character boundary step (3 bytes per Hangul char).
        app.on_key_keyword(key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.keyword_cursor, "한".len());
        app.on_key_keyword(key(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.filter.keyword, "한x글");
    }

    #[test]
    fn theme_select_previews_on_move_and_esc_restores_original() {
        let mut app = empty_app();
        let original = app.theme.key.clone();
        app.open_theme_select();
        assert_eq!(app.mode, UiMode::ThemeSelect);
        // Cursor starts on the active theme within its own category list.
        let state = app.theme_select.as_ref().unwrap();
        assert_eq!(state.themes[state.visible()[state.cursor]].key, original);

        // Down applies the next theme immediately (live preview).
        app.on_key_theme_select(key(KeyCode::Down, KeyModifiers::NONE));
        assert_ne!(app.theme.key, original);

        // Esc restores the theme active at open time and closes the dialog.
        app.on_key_theme_select(key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.theme.key, original);
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.theme_select.is_none());
    }

    #[test]
    fn theme_select_enter_keeps_previewed_theme_and_closes() {
        let mut app = empty_app();
        app.open_theme_select();
        app.on_key_theme_select(key(KeyCode::Down, KeyModifiers::NONE));
        let previewed = app.theme.key.clone();

        app.on_key_theme_select(key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.theme.key, previewed);
        assert_eq!(app.mode, UiMode::Table);
        assert!(app.theme_select.is_none());
        assert!(app
            .status_msg
            .as_deref()
            .is_some_and(|m| m.starts_with("Theme: ")));
    }

    #[test]
    fn theme_select_up_from_top_stays_clamped() {
        let mut app = empty_app();
        app.open_theme_select();
        // Move to the very top, then Up again must not wrap to the last item.
        app.on_key_theme_select(key(KeyCode::Home, KeyModifiers::NONE));
        app.on_key_theme_select(key(KeyCode::Up, KeyModifiers::NONE));
        let state = app.theme_select.as_ref().unwrap();
        assert_eq!(state.cursor, 0);
        assert_eq!(app.theme.key, state.themes[state.visible()[0]].key);
    }

    #[test]
    fn theme_select_down_from_bottom_stays_clamped() {
        let mut app = empty_app();
        app.open_theme_select();
        let vlen = app.theme_select.as_ref().unwrap().visible().len();
        app.on_key_theme_select(key(KeyCode::End, KeyModifiers::NONE));
        app.on_key_theme_select(key(KeyCode::Down, KeyModifiers::NONE));
        let state = app.theme_select.as_ref().unwrap();
        assert_eq!(state.cursor, vlen - 1);
    }

    #[test]
    fn theme_select_left_right_swaps_the_whole_list() {
        let mut app = empty_app();
        app.open_theme_select();
        // Right shows the Light list: every visible theme is light, cursor resets.
        app.on_key_theme_select(key(KeyCode::Right, KeyModifiers::NONE));
        let state = app.theme_select.as_ref().unwrap();
        assert!(!state.dark_view);
        assert_eq!(state.cursor, 0);
        assert!(state.visible().iter().all(|&i| !state.themes[i].dark));
        assert!(!app.theme.dark, "preview switched to a light theme");

        // Left shows the Dark list again.
        app.on_key_theme_select(key(KeyCode::Left, KeyModifiers::NONE));
        let state = app.theme_select.as_ref().unwrap();
        assert!(state.dark_view);
        assert!(state.visible().iter().all(|&i| state.themes[i].dark));
        assert!(app.theme.dark, "preview switched to a dark theme");
    }
}
