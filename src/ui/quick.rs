//! Quick Command window (`:` palette / `!` terminal): command registry, alias searching,
//! terminal command history, and execution history.
//!
//! Palette mode (`:`):
//! - Matching: Splits input by whitespace. Matches if all words are present as substrings
//!   (case-insensitive) in the label, aliases, or shortcut keys (multi-word AND).
//! - Sorting: Enabled commands on the active screen appear at the top, disabled below.
//!   Within each group, sorted by most recently used first, falling back to registry order.
//! - History: Saved to `~/.config/s7s/quick_history.json` to persist execution keys
//!   across application restarts.
//!
//! Terminal mode (`!`):
//! - Runs a user shell command in the selected session's folder (main loop handover).
//! - The list below the input shows terminal command history (most recent first) filtered
//!   by the typed text; moving the selection recalls the command into the editable input,
//!   and Enter always runs the input content.
//! - History: Saved to `~/.config/s7s/terminal_history.json` as raw command strings.
//!
//! Mode switching: pressing `:`/`!` while the input is EMPTY switches window mode
//! (both characters are ordinary typeable characters otherwise).

use super::{App, Screen, TerminalKind, TerminalRequest, TextInput, UiMode};

/// Maximum rows visible in the viewport. Exceeding this triggers cursor-following scroll.
pub const VIEWPORT: usize = 10;

/// Maximum number of recently executed keys preserved in the history file.
const MAX_HISTORY: usize = 20;

/// Window mode: command palette (`:`) or terminal command (`!`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickMode {
    Palette,
    Terminal,
}

/// Commands exposed in the palette, mapping 1:1 with registry entries (`COMMANDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    OpenSessionWindow,
    OpenProfileWindow,
    ResumeSession,
    NewSession,
    NewSessionWithContext,
    RenameSession,
    DeleteSession,
    TerminalCommand,
    CreateProfile,
    EditProfile,
    DeleteProfile,
    ToggleProfileShortcut,
    SearchSessions,
    FilterByAgent,
    FilterByFolder,
    ClearFilters,
    RefreshAll,
    ToggleToolLogs,
    EditConfig,
    ChangeTheme,
    OpenHelp,
    ExitApp,
}

/// Command specification. `key` serves as a stable identifier for history serialization.
pub struct CommandSpec {
    pub id: CommandId,
    pub key: &'static str,
    pub label: &'static str,
    /// Original keyboard shortcut notation (None if palette-only command).
    pub shortcut: Option<&'static str>,
    /// Searchable synonyms (lowercase).
    pub aliases: &'static [&'static str],
    /// Single-line description (only provided for complex commands).
    pub description: Option<&'static str>,
}

/// Command registry. Array order defines default layout presentation.
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::OpenSessionWindow,
        key: "open-session-window",
        label: "Open Session Window",
        shortcut: None,
        aliases: &["go", "switch", "view", "list", "screen"],
        description: None,
    },
    CommandSpec {
        id: CommandId::OpenProfileWindow,
        key: "open-profile-window",
        label: "Open Profile Window",
        shortcut: None,
        aliases: &["go", "switch", "view", "list", "screen"],
        description: None,
    },
    CommandSpec {
        id: CommandId::ResumeSession,
        key: "resume-session",
        label: "Resume Session",
        shortcut: Some("enter"),
        aliases: &["continue", "open", "attach"],
        description: None,
    },
    CommandSpec {
        id: CommandId::NewSession,
        key: "new-session",
        label: "New Session",
        shortcut: Some("ctrl+n"),
        aliases: &["create", "add", "start"],
        description: Some("Open the new session dialog (pick profile and folder)"),
    },
    CommandSpec {
        id: CommandId::NewSessionWithContext,
        key: "new-session-with-context",
        label: "New Session with Context",
        shortcut: Some("ctrl+shift+n"),
        aliases: &["context", "reference", "from-session", "attach-session"],
        description: Some("Start a new session using the selected session as historical context"),
    },
    CommandSpec {
        id: CommandId::RenameSession,
        key: "rename-session",
        label: "Rename Session",
        shortcut: Some("ctrl+r"),
        aliases: &["title", "name", "change"],
        description: None,
    },
    CommandSpec {
        id: CommandId::DeleteSession,
        key: "delete-session",
        label: "Delete Session",
        shortcut: Some("ctrl+d"),
        aliases: &["remove", "rm", "del"],
        description: Some("Delete the selected session's transcript files from disk"),
    },
    CommandSpec {
        id: CommandId::TerminalCommand,
        key: "terminal-command",
        label: "Terminal Command",
        shortcut: Some("!"),
        aliases: &["shell", "run", "exec", "execute", "cmd", "bash"],
        description: Some("Run a shell command in the selected session's folder"),
    },
    CommandSpec {
        id: CommandId::CreateProfile,
        key: "create-profile",
        label: "Create Profile",
        shortcut: Some("+"),
        aliases: &["add", "new"],
        description: None,
    },
    CommandSpec {
        id: CommandId::EditProfile,
        key: "edit-profile",
        label: "Edit Profile",
        shortcut: Some("ctrl+e"),
        aliases: &["modify", "change", "config"],
        description: None,
    },
    CommandSpec {
        id: CommandId::DeleteProfile,
        key: "delete-profile",
        label: "Delete Profile",
        shortcut: Some("ctrl+d"),
        aliases: &["remove", "rm", "del"],
        description: Some("Delete the selected profile from s7s (config folder is kept)"),
    },
    CommandSpec {
        id: CommandId::ToggleProfileShortcut,
        key: "toggle-profile-active",
        label: "Toggle Profile Shortcut",
        shortcut: Some("space"),
        aliases: &["enable", "disable", "activate", "deactivate", "order"],
        description: Some("Add the selected profile at the end, or remove its shortcut"),
    },
    CommandSpec {
        id: CommandId::SearchSessions,
        key: "search-sessions",
        label: "Search Sessions",
        shortcut: Some("/"),
        aliases: &["find", "keyword", "filter"],
        description: None,
    },
    CommandSpec {
        id: CommandId::FilterByAgent,
        key: "filter-by-agent",
        label: "Filter by Agent",
        shortcut: Some("a"),
        aliases: &["filter", "claude", "codex", "antigravity"],
        description: None,
    },
    CommandSpec {
        id: CommandId::FilterByFolder,
        key: "filter-by-folder",
        label: "Filter by Folder",
        shortcut: Some("f"),
        aliases: &["filter", "directory", "path"],
        description: None,
    },
    CommandSpec {
        id: CommandId::ClearFilters,
        key: "clear-filters",
        label: "Clear Filters",
        shortcut: Some("0"),
        aliases: &["reset", "remove"],
        description: Some("Clear keyword, agent, folder and profile filters"),
    },
    CommandSpec {
        id: CommandId::RefreshAll,
        key: "refresh-all",
        label: "Refresh Usage & Sessions",
        shortcut: Some("ctrl+u"),
        aliases: &["update", "reload", "sync", "rescan"],
        description: Some("Rescan sessions and re-fetch usage for all profiles"),
    },
    CommandSpec {
        id: CommandId::ToggleToolLogs,
        key: "toggle-tool-logs",
        label: "Toggle Tool Logs",
        shortcut: Some("."),
        aliases: &["show", "hide", "call", "result"],
        description: Some("Show/hide tool calls and results in the detail view"),
    },
    CommandSpec {
        id: CommandId::EditConfig,
        key: "edit-config",
        label: "Edit Config",
        shortcut: None,
        aliases: &["settings", "editor", "config.toml", "preferences", "open"],
        description: Some("Open ~/.config/s7s/config.toml in the default editor"),
    },
    CommandSpec {
        id: CommandId::ChangeTheme,
        key: "change-theme",
        label: "Change Theme",
        shortcut: None,
        aliases: &[
            "color",
            "colors",
            "colour",
            "skin",
            "dark",
            "light",
            "appearance",
        ],
        description: Some("Pick a color theme (live preview; custom themes: ~/.config/s7s/themes)"),
    },
    CommandSpec {
        id: CommandId::OpenHelp,
        key: "open-help",
        label: "Open Help",
        shortcut: Some("?"),
        aliases: &["shortcuts", "keys", "guide", "manual"],
        description: None,
    },
    CommandSpec {
        id: CommandId::ExitApp,
        key: "exit-s7s",
        label: "Quit",
        shortcut: Some("q"),
        aliases: &["exit", "close", "terminate"],
        description: None,
    },
];

/// Presentation items in the palette (registry index and enablement state on active screen).
#[derive(Debug, Clone, Copy)]
pub struct QuickItem {
    /// Index in `COMMANDS` registry.
    pub spec_idx: usize,
    pub enabled: bool,
}

impl QuickItem {
    pub fn spec(&self) -> &'static CommandSpec {
        &COMMANDS[self.spec_idx]
    }
}

/// Quick Command modal state (populated if UiMode is QuickCommand).
pub struct QuickState {
    /// Window mode (palette or terminal). Switched via `:`/`!` on an empty input.
    pub mode: QuickMode,
    pub input: TextInput,
    /// Palette mode: active list of items matching input search, sorted top-to-bottom.
    pub items: Vec<QuickItem>,
    /// Palette mode: cursor index position in the list.
    pub cursor: usize,
    /// Scroll offset of list viewport (both modes).
    pub scroll: usize,
    /// Terminal mode: folder commands run in (captured at open/switch time; None until
    /// the window first enters terminal mode).
    pub term_folder: Option<std::path::PathBuf>,
    /// Terminal mode: history commands matching the typed text (most recent first).
    pub term_items: Vec<String>,
    /// Terminal mode: selected history row. None = focus stays on the input line.
    pub term_selected: Option<usize>,
    /// Terminal mode: last text typed by the user (filter source). A history recall
    /// replaces the input; moving back above the list restores this text.
    pub term_typed: String,
    /// Top anchor position y of modal (fixed during first render). Prevents modal top
    /// from shifting dynamically when search alters modal height.
    pub anchor_y: std::cell::Cell<Option<u16>>,
}

impl QuickState {
    fn new(mode: QuickMode, term_folder: Option<std::path::PathBuf>) -> Self {
        QuickState {
            mode,
            input: TextInput::new(String::new()),
            items: Vec::new(),
            cursor: 0,
            scroll: 0,
            term_folder,
            term_items: Vec::new(),
            term_selected: None,
            term_typed: String::new(),
            anchor_y: std::cell::Cell::new(None),
        }
    }
}

/// Evaluates if all query tokens are substrings of the command label, aliases, or shortcut keys (AND match).
fn matches(spec: &CommandSpec, tokens: &[String]) -> bool {
    let label = spec.label.to_ascii_lowercase();
    tokens.iter().all(|t| {
        label.contains(t.as_str())
            || spec.aliases.iter().any(|a| a.contains(t.as_str()))
            || spec.shortcut.is_some_and(|s| s.contains(t.as_str()))
    })
}

/// Constructs presentation items filtered by query and sorted by history and enablement state.
///
/// Sort priorities: enabled first -> most recently used (tail if absent) -> registry order.
/// Empty queries return all registered commands sorted under the same criteria.
pub fn build_items<F: Fn(CommandId) -> bool>(
    query: &str,
    history: &[String],
    enabled: F,
) -> Vec<QuickItem> {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    let mut ranked: Vec<((bool, usize, usize), QuickItem)> = COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, spec)| tokens.is_empty() || matches(spec, &tokens))
        .map(|(idx, spec)| {
            let en = enabled(spec.id);
            let hist = history
                .iter()
                .position(|k| k == spec.key)
                .unwrap_or(usize::MAX);
            (
                (!en, hist, idx),
                QuickItem {
                    spec_idx: idx,
                    enabled: en,
                },
            )
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, item)| item).collect()
}

/// Filters terminal command history by whitespace-separated tokens (case-insensitive
/// substring AND), preserving most-recent-first order. Empty queries return the full history.
pub fn build_term_items(query: &str, history: &[String]) -> Vec<String> {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();
    history
        .iter()
        .filter(|cmd| {
            let lc = cmd.to_ascii_lowercase();
            tokens.iter().all(|t| lc.contains(t.as_str()))
        })
        .cloned()
        .collect()
}

/// History serialization path: `~/.config/s7s/quick_history.json`.
fn history_path() -> std::path::PathBuf {
    crate::config::config_base_dir().join("quick_history.json")
}

/// Terminal history serialization path: `~/.config/s7s/terminal_history.json`.
fn terminal_history_path() -> std::path::PathBuf {
    crate::config::config_base_dir().join("terminal_history.json")
}

/// Loads recently executed terminal commands (most recent first) from disk.
/// Unit tests skip disk access to keep results deterministic and avoid touching user config.
pub fn load_terminal_history() -> Vec<String> {
    if cfg!(test) {
        return Vec::new();
    }
    std::fs::read_to_string(terminal_history_path())
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Saves terminal command history to disk (best-effort; no-op in unit tests).
fn save_terminal_history(history: &[String]) {
    if cfg!(test) {
        return;
    }
    let path = terminal_history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, data);
    }
}

/// Loads recently executed keys (most recent first) from disk, returning empty if absent/malformed.
pub fn load_history() -> Vec<String> {
    std::fs::read_to_string(history_path())
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

/// Saves history to disk (best-effort; failures do not disrupt application runtime).
fn save_history(history: &[String]) {
    let path = history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, data);
    }
}

impl App {
    /// `:`: Opens the Quick Command palette (empty input defaults to recently executed + all commands).
    pub(super) fn open_quick_command(&mut self) {
        self.quick = Some(QuickState::new(QuickMode::Palette, None));
        self.quick_recompute();
        self.mode = UiMode::QuickCommand;
        self.status_msg = None;
    }

    /// `!`: Opens the Quick Command window in terminal mode targeting the selected
    /// session's folder. Shows a status message instead if no target folder is available.
    pub(super) fn open_quick_terminal(&mut self) {
        let Some(folder) = self.terminal_target() else {
            self.status_msg =
                Some("Terminal command needs a session with an existing folder".to_string());
            return;
        };
        self.quick = Some(QuickState::new(QuickMode::Terminal, Some(folder)));
        self.quick_term_recompute();
        self.mode = UiMode::QuickCommand;
        self.status_msg = None;
    }

    /// Resolves the folder terminal commands run in: the selected session's cwd (Session
    /// list) or the detail target's cwd (Detail). None if no session is selected, the
    /// session has no folder, or the folder no longer exists on disk.
    pub(super) fn terminal_target(&self) -> Option<std::path::PathBuf> {
        let idx = match self.screen {
            Screen::Session => self.filtered.get(self.selected).copied(),
            Screen::Detail => self.detail.as_ref().map(|d| d.session_idx),
            Screen::Profile => None,
        }?;
        let cwd = &self.sessions.get(idx)?.cwd;
        (!cwd.as_os_str().is_empty() && cwd.is_dir()).then(|| cwd.clone())
    }

    /// Switches the open window to terminal mode (empty-input `!`), re-resolving the target
    /// folder. Keeps palette mode with a status message if no target folder is available.
    fn quick_switch_terminal(&mut self) {
        let Some(folder) = self.terminal_target() else {
            self.status_msg =
                Some("Terminal command needs a session with an existing folder".to_string());
            return;
        };
        if let Some(state) = self.quick.as_mut() {
            state.mode = QuickMode::Terminal;
            state.term_folder = Some(folder);
            state.term_typed.clear();
        }
        self.quick_term_recompute();
    }

    /// Switches the open window to palette mode (empty-input `:`).
    fn quick_switch_palette(&mut self) {
        if let Some(state) = self.quick.as_mut() {
            state.mode = QuickMode::Palette;
        }
        self.quick_recompute();
    }

    /// Re-evaluates list matches and resets cursor / scroll positions on input changes.
    fn quick_recompute(&mut self) {
        let Some(state) = self.quick.as_ref() else {
            return;
        };
        let query = state.input.value.clone();
        let items = build_items(&query, &self.quick_history, |id| self.quick_enabled(id));
        if let Some(state) = self.quick.as_mut() {
            state.items = items;
            state.cursor = 0;
            state.scroll = 0;
        }
    }

    /// Re-evaluates the terminal history list and resets selection / scroll positions.
    fn quick_term_recompute(&mut self) {
        let Some(query) = self.quick.as_ref().map(|s| s.term_typed.clone()) else {
            return;
        };
        let items = build_term_items(&query, &self.terminal_history);
        if let Some(state) = self.quick.as_mut() {
            state.term_items = items;
            state.term_selected = None;
            state.scroll = 0;
        }
    }

    /// Evaluates if the command is enabled on the active screen (drives dim rendering and sorting).
    fn quick_enabled(&self, id: CommandId) -> bool {
        use CommandId::*;
        match id {
            OpenSessionWindow => self.screen != Screen::Session,
            OpenProfileWindow => self.screen != Screen::Profile,
            // Contextual New Session needs a focused source session (Session/Detail only).
            ResumeSession | NewSessionWithContext | RenameSession | DeleteSession => {
                match self.screen {
                    Screen::Session => self.filtered.get(self.selected).is_some(),
                    Screen::Detail => self.detail.is_some(),
                    Screen::Profile => false,
                }
            }
            TerminalCommand => self.terminal_target().is_some(),
            NewSession => true,
            CreateProfile => self.screen == Screen::Profile,
            EditProfile | DeleteProfile => {
                self.screen == Screen::Profile && !self.profiles.profiles.is_empty()
            }
            ToggleProfileShortcut => {
                self.screen == Screen::Profile && !self.profiles.profiles.is_empty()
            }
            SearchSessions | FilterByAgent | FilterByFolder => self.screen == Screen::Session,
            ClearFilters => self.screen == Screen::Session && self.filter.is_active(),
            ToggleToolLogs => self.screen == Screen::Detail,
            RefreshAll | EditConfig | ChangeTheme | OpenHelp | ExitApp => true,
        }
    }

    /// Handles key inputs in the Quick Command modal, dispatching by window mode.
    /// `:`/`!` on an EMPTY input switches the window mode (both are ordinary typeable
    /// characters otherwise).
    pub fn on_key_quick(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some((mode, input_empty)) = self
            .quick
            .as_ref()
            .map(|s| (s.mode, s.input.value.is_empty()))
        else {
            self.mode = UiMode::Table;
            return;
        };
        if let KeyCode::Char(c @ (':' | '!')) = key.code {
            if input_empty && !key.modifiers.contains(KeyModifiers::CONTROL) {
                match c {
                    ':' if mode == QuickMode::Terminal => self.quick_switch_palette(),
                    '!' if mode == QuickMode::Palette => self.quick_switch_terminal(),
                    // Same-mode key: consumed as a mode key, not inserted as text.
                    _ => {}
                }
                return;
            }
        }
        match mode {
            QuickMode::Palette => self.on_key_quick_palette(key),
            QuickMode::Terminal => self.on_key_quick_terminal(key),
        }
    }

    /// Palette mode keys: Esc closes, Enter runs selection, Up/Down moves cursor,
    /// and editing keys update the search text input.
    fn on_key_quick_palette(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(state) = self.quick.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.quick = None;
                self.mode = UiMode::Table;
            }
            KeyCode::Enter => {
                let Some(item) = state.items.get(state.cursor).copied() else {
                    return;
                };
                if !item.enabled {
                    return;
                }
                self.quick = None;
                self.mode = UiMode::Table;
                self.quick_record_history(item.spec().key);
                self.quick_execute(item.spec().id);
            }
            KeyCode::Up => Self::quick_move_cursor(state, -1),
            KeyCode::Down => Self::quick_move_cursor(state, 1),
            KeyCode::PageUp => Self::quick_move_cursor(state, -(VIEWPORT as isize)),
            KeyCode::PageDown => Self::quick_move_cursor(state, VIEWPORT as isize),
            KeyCode::Left => state.input.move_left(),
            KeyCode::Right => state.input.move_right(),
            KeyCode::Home => state.input.home(),
            KeyCode::End => state.input.end(),
            KeyCode::Backspace => {
                state.input.backspace();
                self.quick_recompute();
            }
            KeyCode::Delete => {
                state.input.delete();
                self.quick_recompute();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.input.insert_char(c);
                self.quick_recompute();
            }
            _ => {}
        }
    }

    /// Terminal mode keys: Esc closes, Enter runs the input content, Up/Down moves the
    /// history selection (recalling the command into the editable input), and editing
    /// keys update the input (detaching any recall and re-filtering the history list).
    fn on_key_quick_terminal(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};
        let Some(state) = self.quick.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        match key.code {
            KeyCode::Esc => {
                self.quick = None;
                self.mode = UiMode::Table;
            }
            KeyCode::Enter => {
                let command = state.input.value.trim().to_string();
                if command.is_empty() {
                    return;
                }
                let Some(cwd) = state.term_folder.clone() else {
                    return;
                };
                self.quick = None;
                self.mode = UiMode::Table;
                self.record_terminal_history(&command);
                self.terminal_request = Some(TerminalRequest {
                    cwd,
                    command,
                    kind: TerminalKind::Command,
                });
            }
            KeyCode::Up => Self::term_move_selection(state, -1),
            KeyCode::Down => Self::term_move_selection(state, 1),
            KeyCode::PageUp => Self::term_move_selection(state, -(VIEWPORT as isize)),
            KeyCode::PageDown => Self::term_move_selection(state, VIEWPORT as isize),
            KeyCode::Left => state.input.move_left(),
            KeyCode::Right => state.input.move_right(),
            KeyCode::Home => state.input.home(),
            KeyCode::End => state.input.end(),
            KeyCode::Backspace => {
                state.input.backspace();
                self.term_after_edit();
            }
            KeyCode::Delete => {
                state.input.delete();
                self.term_after_edit();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.input.insert_char(c);
                self.term_after_edit();
            }
            _ => {}
        }
    }

    /// Editing detaches any history recall: the input becomes the typed filter text again.
    fn term_after_edit(&mut self) {
        if let Some(state) = self.quick.as_mut() {
            state.term_typed = state.input.value.clone();
        }
        self.quick_term_recompute();
    }

    /// Moves the history selection, recalling the selected command into the editable input.
    /// Moving above the first row returns focus to the input line and restores the typed text.
    fn term_move_selection(state: &mut QuickState, delta: isize) {
        if state.term_items.is_empty() {
            return;
        }
        let last = state.term_items.len() as isize - 1;
        // -1 represents "no selection" (focus on the input line).
        let cur = state.term_selected.map(|i| i as isize).unwrap_or(-1);
        let next = (cur + delta).clamp(-1, last);
        state.term_selected = (next >= 0).then_some(next as usize);
        match state.term_selected {
            Some(i) => {
                state.input.value = state.term_items[i].clone();
                state.scroll = if i < state.scroll {
                    i
                } else if i >= state.scroll + VIEWPORT {
                    i + 1 - VIEWPORT
                } else {
                    state.scroll
                };
            }
            None => {
                state.input.value = state.term_typed.clone();
                state.scroll = 0;
            }
        }
        state.input.cursor = state.input.value.len();
    }

    /// Records an executed terminal command to the head of history and persists it to disk.
    fn record_terminal_history(&mut self, command: &str) {
        self.terminal_history.retain(|c| c != command);
        self.terminal_history.insert(0, command.to_string());
        self.terminal_history.truncate(MAX_HISTORY);
        save_terminal_history(&self.terminal_history);
    }

    /// Moves cursor index, shifting the viewport (max 10 rows) to keep cursor visible.
    fn quick_move_cursor(state: &mut QuickState, delta: isize) {
        if state.items.is_empty() {
            return;
        }
        let last = state.items.len() - 1;
        state.cursor = (state.cursor as isize + delta).clamp(0, last as isize) as usize;
        if state.cursor < state.scroll {
            state.scroll = state.cursor;
        } else if state.cursor >= state.scroll + VIEWPORT {
            state.scroll = state.cursor + 1 - VIEWPORT;
        }
    }

    /// Records executed key to the head of history and serializes changes to disk.
    fn quick_record_history(&mut self, key: &str) {
        self.quick_history.retain(|k| k != key);
        self.quick_history.insert(0, key.to_string());
        self.quick_history.truncate(MAX_HISTORY);
        save_history(&self.quick_history);
    }

    /// Executes the selected command, delegating to the same handler method invoked by standard hotkeys.
    fn quick_execute(&mut self, id: CommandId) {
        use CommandId::*;
        // Session index mapping back to `sessions` list from search or details views.
        let session_idx = match self.screen {
            Screen::Session => self.filtered.get(self.selected).copied(),
            Screen::Detail => self.detail.as_ref().map(|d| d.session_idx),
            Screen::Profile => None,
        };
        match id {
            OpenSessionWindow => self.switch_screen(Screen::Session),
            OpenProfileWindow => self.switch_screen(Screen::Profile),
            ResumeSession => {
                if let Some(idx) = session_idx {
                    self.request_resume(idx);
                }
            }
            NewSession => {
                if self.screen == Screen::Profile {
                    self.open_new_session_modal(self.profile_selected, None, false, None);
                } else {
                    self.open_new_session_modal_for_session(session_idx, false);
                }
            }
            // Guaranteed fallback for terminals that cannot distinguish Ctrl+Shift+N.
            NewSessionWithContext => self.open_new_session_modal_for_session(session_idx, true),
            RenameSession => {
                if let Some(idx) = session_idx {
                    self.open_rename_modal_at(idx);
                }
            }
            DeleteSession => {
                if let Some(idx) = session_idx {
                    self.open_delete_confirm_at(idx);
                }
            }
            TerminalCommand => self.open_quick_terminal(),
            CreateProfile => self.open_profile_form(None),
            EditProfile => self.open_profile_form(Some(self.profile_selected)),
            DeleteProfile => self.open_profile_delete(),
            ToggleProfileShortcut => self.toggle_selected_profile_shortcut(),
            SearchSessions => {
                self.mode = UiMode::Keyword;
                self.keyword_cursor = self.filter.keyword.len();
            }
            FilterByAgent => self.open_agent_modal(),
            FilterByFolder => self.open_folder_modal(),
            ClearFilters => self.clear_all_filters(),
            RefreshAll => self.update_sessions_and_usage(),
            ToggleToolLogs => self.detail_show_tools = !self.detail_show_tools,
            EditConfig => self.request_edit_config(),
            ChangeTheme => self.open_theme_select(),
            OpenHelp => self.open_help(),
            ExitApp => self.should_quit = true,
        }
    }

    /// Opens `~/.config/s7s/config.toml` in the resolved editor via the terminal
    /// handover path. A missing or blank file is seeded with the commented template
    /// first (which also creates the config dir, so `cd` in the handover succeeds).
    fn request_edit_config(&mut self) {
        crate::config::ensure_config_template();
        let dir = crate::config::config_base_dir();
        let path = crate::config::config_file_path();
        let command = format!(
            "{} {}",
            self.cfg.editor_command(),
            crate::resume::shell_quote(&path.to_string_lossy())
        );
        self.terminal_request = Some(TerminalRequest {
            cwd: dir,
            command,
            kind: TerminalKind::EditConfig,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_of(item: &QuickItem) -> &'static CommandSpec {
        item.spec()
    }

    #[test]
    fn multi_word_and_matching() {
        let items = build_items("del prof", &[], |_| true);
        assert_eq!(items.len(), 1);
        assert_eq!(spec_of(&items[0]).label, "Delete Profile");
    }

    #[test]
    fn synonym_matching() {
        let items = build_items("exit", &[], |_| true);
        assert!(items.iter().any(|i| spec_of(i).label == "Quit"));
        let items = build_items("update", &[], |_| true);
        assert!(items
            .iter()
            .any(|i| spec_of(i).label == "Refresh Usage & Sessions"));
    }

    #[test]
    fn disabled_items_sort_below_enabled() {
        let items = build_items("session", &[], |id| id != CommandId::ResumeSession);
        let resume_pos = items
            .iter()
            .position(|i| spec_of(i).id == CommandId::ResumeSession)
            .unwrap();
        // Disabled "Resume Session" must sort below enabled matched items.
        assert!(items[..resume_pos].iter().all(|i| i.enabled));
        assert!(!items[resume_pos].enabled);
    }

    #[test]
    fn empty_query_lists_recent_first_then_rest() {
        let history = vec!["exit-s7s".to_string(), "refresh-all".to_string()];
        let items = build_items("", &history, |_| true);
        assert_eq!(items.len(), COMMANDS.len());
        assert_eq!(spec_of(&items[0]).key, "exit-s7s");
        assert_eq!(spec_of(&items[1]).key, "refresh-all");
        // Remaining items preserve default registry order.
        assert_eq!(spec_of(&items[2]).key, COMMANDS[0].key);
    }

    #[test]
    fn term_items_match_multi_token_case_insensitive() {
        let history = vec![
            "git status".to_string(),
            "Cargo build --release".to_string(),
            "ls -al".to_string(),
        ];
        // Empty query returns the full history in order.
        assert_eq!(build_term_items("", &history), history);
        // Multi-token AND matching, case-insensitive.
        assert_eq!(
            build_term_items("car rel", &history),
            vec!["Cargo build --release".to_string()]
        );
        assert_eq!(
            build_term_items("GIT", &history),
            vec!["git status".to_string()]
        );
        assert!(build_term_items("git cargo", &history).is_empty());
    }

    #[test]
    fn edit_config_matches_editor_and_settings_aliases() {
        for query in ["editor", "settings", "config"] {
            let items = build_items(query, &[], |_| true);
            assert!(
                items.iter().any(|i| spec_of(i).id == CommandId::EditConfig),
                "query {query:?} should match Edit Config"
            );
        }
    }

    #[test]
    fn recent_but_disabled_still_sorts_below_enabled() {
        let history = vec!["toggle-tool-logs".to_string()];
        let items = build_items("", &history, |id| id != CommandId::ToggleToolLogs);
        assert!(!items.last().unwrap().enabled);
        assert_eq!(spec_of(items.last().unwrap()).key, "toggle-tool-logs");
    }
}
