//! Quick Command window state (palette / terminal), the terminal history filter,
//! and on-disk serialization of the execution and terminal command histories.

use super::registry::QuickItem;
use crate::ui::TextInput;

/// Window mode: command palette (`:`) or terminal command (`!`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickMode {
    Palette,
    Terminal,
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
    pub(super) fn new(mode: QuickMode, term_folder: Option<std::path::PathBuf>) -> Self {
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
pub(super) fn save_terminal_history(history: &[String]) {
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
pub(super) fn save_history(history: &[String]) {
    let path = history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
