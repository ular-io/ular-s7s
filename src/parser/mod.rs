//! Session file parsers.
//!
//! Refines and extracts **user turns** from raw logs (JSONL, SQLite, etc.) of each agent.
//! Interaction Q&As where the agent asks and the user responds are promoted to virtual user turns
//! represented in the format `· question → answer`. Excludes system messages, AI replies, and tool call logs.
//! (Full parsing of tasks and replies per turn is handled by [`crate::handoff`].)

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod turn;

use crate::model::Session;
use crate::normalize;

/// Computes and populates a search blob (NFC-normalized, lowercase concatenated string)
/// from user turns and the folder name.
pub fn finalize(session: &mut Session) {
    let mut joined = session.user_turns.join("\n");
    append_folder(&mut joined, &session.folder);
    session.search_blob = normalize::nfc_lower(&joined);
}

/// Recomputes the search blob by combining user turns, the final resolved title, and the folder name.
///
/// Called after title metadata (rename/preview) is settled and the final title is determined.
/// Always calculated from scratch using `user_turns`, making it idempotent.
/// Searching evaluates only this blob, incurring no additional overhead at query time.
pub fn reindex_search_blob(session: &mut Session) {
    let mut joined = session.user_turns.join("\n");
    let title = session.title();
    if !title.is_empty() {
        joined.push('\n');
        joined.push_str(&title);
    }
    append_folder(&mut joined, &session.folder);
    session.search_blob = normalize::nfc_lower(&joined);
}

/// Appends the folder name to the blob source on its own line when present.
fn append_folder(joined: &mut String, folder: &str) {
    if !folder.is_empty() {
        joined.push('\n');
        joined.push_str(folder);
    }
}

/// NFC-normalizes the text and trims leading/trailing whitespaces. Returns None if empty.
pub fn clean_turn(raw: &str) -> Option<String> {
    let normalized = normalize::nfc(raw);
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Identifies if a user input line is noise rather than a question (e.g. slash/local commands, system injections, skill documents).
pub fn is_noise_turn(text: &str) -> bool {
    let t = text.trim_start();
    let trimmed = text.trim();

    // Exclude single-token slash commands without arguments (e.g. /usage, /exit, /status, /clear).
    // Note that skill invocations with arguments (e.g. "/tde confluence docs...") are retained as questions.
    if trimmed.starts_with('/') && !trimmed.contains(char::is_whitespace) {
        return true;
    }

    t.starts_with("<command-name>")
        || t.starts_with("<command-message>")
        || t.starts_with("<local-command")
        || t.starts_with("Caveat:")
        || t.starts_with("[Request interrupted")
        || t.starts_with("<system-reminder>")
        // Background task completion notices are appended as user-role entries by Claude Code.
        || t.starts_with("<task-notification>")
        // s7s-injected bootstrap prompt for "New Session with Context" launches.
        || t.starts_with("<s7s-context-bootstrap>")
        // Skip cases where a skill's SKILL.md body gets recorded as user input (not a question).
        || t.starts_with("Base directory for this skill:")
        || matches!(
            trimmed,
            "exit" | "quit" | "\\q" | "\\quit" | ":q" | ":wq" | "q"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Agent, Session};

    fn session(folder: &str, turns: &[&str]) -> Session {
        Session {
            agent: Agent::Claude,
            profile_id: String::new(),
            id: "id".to_string(),
            source_path: None,
            cwd: format!("/tmp/{folder}").into(),
            folder: folder.to_string(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: turns.iter().map(|t| t.to_string()).collect(),
            search_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        }
    }

    #[test]
    fn finalize_includes_folder_in_blob() {
        let mut s = session("MyProject", &["hello world"]);
        finalize(&mut s);
        assert!(s.search_blob.contains("hello world"));
        assert!(s.search_blob.contains("myproject"));
    }

    #[test]
    fn reindex_includes_folder_in_blob() {
        let mut s = session("MyProject", &["hello world"]);
        reindex_search_blob(&mut s);
        assert!(s.search_blob.contains("myproject"));
    }

    #[test]
    fn empty_folder_leaves_no_trailing_marker() {
        let mut s = session("", &["hello"]);
        finalize(&mut s);
        assert_eq!(s.search_blob, "hello");
    }

    #[test]
    fn task_notification_is_noise() {
        assert!(is_noise_turn(
            "<task-notification>\n<task-id>ab23526d041845ac3</task-id>\n</task-notification>"
        ));
        assert!(is_noise_turn(
            "  <task-notification><tool-use-id>toolu_x</tool-use-id></task-notification>"
        ));
        assert!(!is_noise_turn("task notification about the deploy"));
    }
}
