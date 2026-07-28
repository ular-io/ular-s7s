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

use crate::model::{Agent, ContextSource, Session};
use crate::normalize;
use serde_json::Value;

/// Marker that opens the s7s "New Session with Context" bootstrap envelope.
pub(crate) const BOOTSTRAP_MARKER: &str = "<s7s-context-bootstrap>";

/// Extracts the source-session reference from a bootstrap envelope turn.
///
/// "New Session with Context" injects the source as the launch prompt, e.g.
/// `... session show '<id>' --agent <agent> --profile '<profile>' --bootstrap`.
/// The envelope is otherwise filtered as noise ([`is_noise_turn`]); this recovers
/// the source reference so the derivation can be surfaced without re-showing the
/// envelope. Returns `None` for any turn that is not a bootstrap envelope or is
/// missing a field. Tolerant of an outer `<USER_REQUEST>` wrapper (Antigravity).
pub(crate) fn parse_context_bootstrap(text: &str) -> Option<ContextSource> {
    if !text.contains(BOOTSTRAP_MARKER) {
        return None;
    }
    let id = single_quoted_after(text, "session show ")?;
    let agent = match word_after(text, "--agent ")? {
        "claude" => Agent::Claude,
        "codex" => Agent::Codex,
        "antigravity" => Agent::Antigravity,
        _ => return None,
    };
    let profile = single_quoted_after(text, "--profile ")?;
    Some(ContextSource { id, agent, profile })
}

/// Contents of the first `'...'` group following `marker` (ASCII marker).
fn single_quoted_after(text: &str, marker: &str) -> Option<String> {
    let rest = text[text.find(marker)? + marker.len()..].strip_prefix('\'')?;
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// Whitespace-delimited token following `marker` (ASCII marker).
fn word_after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let rest = text[text.find(marker)? + marker.len()..].trim_start();
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    (end > 0).then_some(&rest[..end])
}

/// Converts a top-level RFC 3339 record timestamp to Unix epoch milliseconds.
pub(crate) fn record_timestamp_ms(record: &Value) -> Option<i64> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|timestamp| timestamp.timestamp_millis())
}

/// Resolves the semantic session activity time from active conversation events.
///
/// Physical storage mtime remains the cache-freshness signal, but must not move
/// a session when an agent merely resumes and closes it without a new query.
pub(crate) fn session_updated_at_ms(
    user_turn_timestamps_ms: &[Option<i64>],
    last_response_completed_at_ms: Option<i64>,
    source_mtime_ms: i64,
) -> i64 {
    user_turn_timestamps_ms
        .iter()
        .flatten()
        .copied()
        .chain(last_response_completed_at_ms)
        .filter(|timestamp| *timestamp > 0)
        .max()
        .unwrap_or(source_mtime_ms)
}

/// Computes and populates a search blob (NFC-normalized, lowercase concatenated string)
/// from user turns and the folder name.
pub fn finalize(session: &mut Session) {
    session
        .user_turn_timestamps_ms
        .resize(session.user_turns.len(), None);
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

/// Builds the assistant search blob from the last assistant answer of each active turn.
///
/// Redacts known secrets, then NFC-normalizes and lowercases (same normalization as
/// `search_blob`). Only the last assistant text per turn is indexed — intermediate
/// progress messages are intentionally dropped to bound cache growth. Rewind/rollback
/// abandoned answers and the bootstrap ready response are excluded upstream by each
/// agent's turn extraction, so callers pass only the answers that should be searchable.
pub fn build_assistant_blob(per_turn_last: &[String]) -> String {
    if per_turn_last.is_empty() {
        return String::new();
    }
    let joined = per_turn_last.join("\n");
    normalize::nfc_lower(&crate::session_context::redact::redact(&joined))
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
        || t.starts_with(BOOTSTRAP_MARKER)
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
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: turns.iter().map(|t| t.to_string()).collect(),
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
            context_source: None,
        }
    }

    #[test]
    fn parse_context_bootstrap_extracts_source_ref() {
        let text = "<s7s-context-bootstrap>\nRun `'/path/s7s' session show '019f92bb-1' \
            --agent codex --profile 'builtin-codex' --bootstrap`.\n</s7s-context-bootstrap>";
        let src = parse_context_bootstrap(text).expect("expected source ref");
        assert_eq!(src.id, "019f92bb-1");
        assert_eq!(src.agent, Agent::Codex);
        assert_eq!(src.profile, "builtin-codex");
    }

    #[test]
    fn parse_context_bootstrap_tolerates_user_request_wrapper() {
        // Antigravity records the envelope wrapped in <USER_REQUEST>.
        let text = "<USER_REQUEST>\n<s7s-context-bootstrap>\nRun `'/p/s7s' session show 'abc-2' \
            --agent claude --profile 'builtin-claude' --bootstrap`.\n</s7s-context-bootstrap>\n</USER_REQUEST>";
        let src = parse_context_bootstrap(text).expect("expected source ref");
        assert_eq!(src.id, "abc-2");
        assert_eq!(src.agent, Agent::Claude);
        assert_eq!(src.profile, "builtin-claude");
    }

    #[test]
    fn parse_context_bootstrap_ignores_non_bootstrap_and_unknown_agent() {
        assert!(parse_context_bootstrap("just a normal question").is_none());
        let bad_agent = "<s7s-context-bootstrap> session show 'x' --agent gemini \
            --profile 'p'</s7s-context-bootstrap>";
        assert!(parse_context_bootstrap(bad_agent).is_none());
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
    fn build_assistant_blob_redacts_normalizes_and_handles_empty() {
        let out = build_assistant_blob(&[
            "API_KEY=sk-secret1234567890".to_string(),
            "Final Answer TEXT".to_string(),
        ]);
        // NFC-lowercased for case-insensitive matching.
        assert!(out.contains("final answer text"));
        // Secrets are redacted before indexing (never cached in the clear).
        assert!(!out.contains("sk-secret"));
        assert!(out.contains("[redacted]"));
        // Empty input yields an empty blob (no work for user-only sessions).
        assert!(build_assistant_blob(&[]).is_empty());
    }

    #[test]
    fn session_activity_uses_latest_query_or_response_and_only_falls_back_to_mtime() {
        let turns = vec![Some(1_000), None, Some(3_000)];
        assert_eq!(session_updated_at_ms(&turns, Some(2_000), 9_000), 3_000);
        assert_eq!(session_updated_at_ms(&turns, Some(4_000), 9_000), 4_000);
        assert_eq!(session_updated_at_ms(&[], None, 9_000), 9_000);
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
