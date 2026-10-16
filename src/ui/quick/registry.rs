//! Quick Command palette registry: the command enumeration, the static
//! specification table (`COMMANDS`), and the query-driven match/rank logic that
//! turns a search string into ordered presentation items.

/// Commands exposed in the palette, mapping 1:1 with registry entries (`COMMANDS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    OpenSessionWindow,
    OpenWorkspaceWindow,
    OpenProfileWindow,
    ResumeSession,
    NewSession,
    NewSessionWithContext,
    GoToContextSource,
    BackToPreviousSession,
    ToggleBookmark,
    RenameSession,
    ChangeFolder,
    DeleteSession,
    TerminalCommand,
    CreateProfile,
    EditProfile,
    DeleteProfile,
    ToggleProfileShortcut,
    SearchSessions,
    FullSearchSessions,
    FilterByAgent,
    FilterByFolder,
    FilterBookmarkedSessions,
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
        id: CommandId::OpenWorkspaceWindow,
        key: "open-workspace-window",
        label: "Open Workspace Window",
        shortcut: None,
        aliases: &["go", "switch", "view", "list", "screen"],
        description: Some("Show the workspace list beside the sessions"),
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
        id: CommandId::GoToContextSource,
        key: "go-to-context-source",
        label: "Go to Context Source",
        shortcut: Some("ctrl+o"),
        aliases: &["context", "source", "origin", "parent", "jump"],
        description: Some("Move to the session the selected session was launched from"),
    },
    CommandSpec {
        id: CommandId::BackToPreviousSession,
        key: "back-to-previous-session",
        label: "Back to Previous Session",
        shortcut: None,
        aliases: &["return", "previous", "origin", "history", "undo"],
        description: Some("Return along the context source jumps already made"),
    },
    CommandSpec {
        id: CommandId::ToggleBookmark,
        key: "toggle-bookmark",
        label: "Toggle Bookmark",
        shortcut: Some("ctrl+b"),
        aliases: &["star", "favorite", "flag", "mark", "unbookmark"],
        description: Some("Add or remove the selected session's bookmark"),
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
        id: CommandId::ChangeFolder,
        key: "change-folder",
        label: "Change Folder",
        shortcut: None,
        aliases: &[
            "folder",
            "move",
            "cwd",
            "directory",
            "dir",
            "path",
            "relocate",
        ],
        description: Some(
            "Open this session in a different folder from now on · no file is moved · not available for Antigravity",
        ),
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
        id: CommandId::FullSearchSessions,
        key: "full-search-sessions",
        label: "Full Search Sessions",
        shortcut: Some("ctrl+/"),
        aliases: &["find", "content", "grep", "tool", "output", "answer"],
        description: Some("Search every message, tool call, and tool result"),
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
        id: CommandId::FilterBookmarkedSessions,
        key: "filter-bookmarked-sessions",
        label: "Filter Bookmarked Sessions",
        shortcut: None,
        aliases: &["bookmark", "star", "favorite", "marked", "filter"],
        description: Some("Toggle the bookmark filter, combined with the current search and filters"),
    },
    CommandSpec {
        id: CommandId::ClearFilters,
        key: "clear-filters",
        label: "Clear Filters",
        shortcut: Some("0"),
        aliases: &["reset", "remove"],
        description: Some("Clear keyword, agent, folder, profile and bookmark filters"),
    },
    CommandSpec {
        id: CommandId::RefreshAll,
        key: "refresh-all",
        label: "Refresh Usage & Sessions",
        shortcut: Some("ctrl+u"),
        aliases: &["update", "reload", "sync", "rescan"],
        description: Some(
            "Rescan sessions, re-fetch usage, and reload workspaces/bookmarks/profiles saved elsewhere",
        ),
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

/// What a palette row runs: a registry command, or a workspace scope switch
/// generated from the user's workspaces (not part of `COMMANDS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickAction {
    /// Index in `COMMANDS` registry.
    Command(usize),
    /// `Open Workspace <name>` (`Some(index)`) or `Close Workspace` (`None`).
    Workspace(Option<usize>),
    UnassignedWorkspace,
}

/// Presentation items in the palette (action, label, and enablement state on active screen).
#[derive(Debug, Clone)]
pub struct QuickItem {
    pub action: QuickAction,
    /// Display label; a registry command's is its `CommandSpec::label`.
    pub label: String,
    pub enabled: bool,
}

impl QuickItem {
    /// Registry spec of a command row (`None` for workspace rows).
    pub fn spec(&self) -> Option<&'static CommandSpec> {
        match self.action {
            QuickAction::Command(idx) => Some(&COMMANDS[idx]),
            QuickAction::Workspace(_) | QuickAction::UnassignedWorkspace => None,
        }
    }

    pub fn shortcut(&self) -> Option<&'static str> {
        self.spec().and_then(|s| s.shortcut)
    }

    /// Footer text: the command's description, or why the row is disabled.
    pub fn description(&self) -> &'static str {
        match (&self.action, self.enabled) {
            (QuickAction::Workspace(None), false) => "No workspace is open",
            (_, false) => "Not available in this window",
            (QuickAction::Command(_), true) => {
                self.spec().and_then(|s| s.description).unwrap_or("")
            }
            (QuickAction::Workspace(Some(_)), true) => {
                "Show only this workspace's sessions in the session list"
            }
            (QuickAction::Workspace(None), true) => "Show all sessions (no workspace)",
            (QuickAction::UnassignedWorkspace, true) => {
                "Show sessions matching none of the saved workspaces"
            }
        }
    }
}

/// Label of the palette row that returns to the "All" scope.
pub const CLOSE_WORKSPACE_LABEL: &str = "Close Workspace";
/// Prefix of the per-workspace palette rows (`Open Workspace <name>`).
pub const OPEN_WORKSPACE_PREFIX: &str = "Open Workspace";
/// Query `ctrl+w` prefills; it lists every workspace row first.
pub const WORKSPACE_QUERY: &str = "open workspace ";

/// Close Workspace answers the `open workspace` query as well, so it sits
/// beside the open rows it undoes.
const CLOSE_WORKSPACE_ALIASES: &[&str] = &["open", "all", "sessions", "exit"];

/// Evaluates if all query tokens are substrings of the command label, aliases, or shortcut keys (AND match).
fn matches(spec: &CommandSpec, tokens: &[String]) -> bool {
    let label = spec.label.to_ascii_lowercase();
    tokens.iter().all(|t| {
        label.contains(t.as_str())
            || spec.aliases.iter().any(|a| a.contains(t.as_str()))
            || spec.shortcut.is_some_and(|s| s.contains(t.as_str()))
    })
}

/// Workspace rows for a non-empty query (an empty `:` palette is unchanged):
/// `Close Workspace` first, then Unassigned and saved workspace rows, each kept
/// when every query token matches its label (or Close's aliases). A
/// disabled Close (no workspace open) sorts after the open rows.
pub fn build_workspace_items(query: &str, names: &[&str], scope_open: bool) -> Vec<QuickItem> {
    let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let hit = |label: &str, aliases: &[&str]| {
        let label = label.to_lowercase();
        tokens
            .iter()
            .all(|t| label.contains(t.as_str()) || aliases.iter().any(|a| a.contains(t.as_str())))
    };
    let close = hit(CLOSE_WORKSPACE_LABEL, CLOSE_WORKSPACE_ALIASES).then(|| QuickItem {
        action: QuickAction::Workspace(None),
        label: CLOSE_WORKSPACE_LABEL.to_string(),
        enabled: scope_open,
    });
    let unassigned_label = format!(
        "{OPEN_WORKSPACE_PREFIX} {}",
        crate::workspaces::UNASSIGNED_WORKSPACE_NAME
    );
    let unassigned = hit(&unassigned_label, &[]).then_some(QuickItem {
        action: QuickAction::UnassignedWorkspace,
        label: unassigned_label,
        enabled: true,
    });
    let opens = unassigned
        .into_iter()
        .chain(names.iter().enumerate().filter_map(|(i, name)| {
            let label = format!("{OPEN_WORKSPACE_PREFIX} {name}");
            hit(&label, &[]).then_some(QuickItem {
                action: QuickAction::Workspace(Some(i)),
                label,
                enabled: true,
            })
        }));
    let mut items: Vec<QuickItem> = Vec::new();
    match close {
        Some(close) if close.enabled => {
            items.push(close);
            items.extend(opens);
        }
        close => {
            items.extend(opens);
            items.extend(close);
        }
    }
    items
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
                    action: QuickAction::Command(idx),
                    label: spec.label.to_string(),
                    enabled: en,
                },
            )
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked.into_iter().map(|(_, item)| item).collect()
}
