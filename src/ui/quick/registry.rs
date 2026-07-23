//! Quick Command palette registry: the command enumeration, the static
//! specification table (`COMMANDS`), and the query-driven match/rank logic that
//! turns a search string into ordered presentation items.

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
