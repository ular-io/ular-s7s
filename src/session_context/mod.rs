//! Shared session-context architecture.
//!
//! Parses a session's raw transcript into a neutral [`SessionContext`] consumed by
//! the TUI Detail screen (via a thin `handoff` adapter), the Markdown handoff
//! exporter, and the `s7s session` CLI.
//!
//! Parity requirement: the detailed parsers here MUST agree with the list parsers
//! (`crate::parser`) about which turns are active — Claude `/rewind` dead branches
//! (`parentUuid` chain) and Codex `thread_rolled_back` markers are filtered with
//! the same logic, so the session list Q count, Detail screen turn numbers, and
//! CLI turn numbers stay equal.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod excerpt;
pub mod model;
pub mod redact;
pub mod render;
pub mod resolve;

pub use model::{
    ContextCompleteness, ContextEntry, ContextEntryKind, ContextTurn, SessionContext,
    SessionContextSource,
};

use crate::model::{Agent, Session};
use crate::parser::is_noise_turn;
use redact::redact;
use serde_json::Value;

/// Loads the session context. Never fails outright: when detailed parsing is not
/// possible the turns fall back to the pre-extracted `Session::user_turns` and
/// `completeness` records why, so consumers can state that assistant/work entries
/// are unavailable instead of silently pretending the context is complete.
pub fn load(session: &Session) -> SessionContext {
    let source = SessionContextSource {
        agent: session.agent,
        profile_id: session.profile_id.clone(),
        session_id: session.id.clone(),
        title: session.title(),
        cwd: session.cwd.clone(),
    };

    let (mut turns, completeness) = match parse_detailed(session) {
        // Antigravity transcripts rotate: a "full" transcript can start mid-session
        // (observed: transcript_full.jsonl beginning at step_index 125). When the
        // transcript carries fewer turns than the DB-derived list, fall back to
        // user turns so CLI/Detail turn numbers never silently diverge from the
        // list Q count — and bootstrap fails instead of claiming full context.
        DetailedParse::Parsed(turns)
            if session.agent == Agent::Antigravity && turns.len() < session.user_turns.len() =>
        {
            (fallback_turns(session), ContextCompleteness::UserTurnsOnly)
        }
        DetailedParse::Parsed(turns) if !turns.is_empty() => (turns, ContextCompleteness::Full),
        DetailedParse::Parsed(_) | DetailedParse::Failed => {
            (fallback_turns(session), ContextCompleteness::ParseFailed)
        }
        DetailedParse::SourceMissing => (
            fallback_turns(session),
            ContextCompleteness::SourceUnavailable,
        ),
        DetailedParse::UserTurnsOnly => {
            (fallback_turns(session), ContextCompleteness::UserTurnsOnly)
        }
    };

    for turn in &mut turns {
        strip_final_answer_echo(turn);
    }

    SessionContext {
        source,
        completeness,
        turns,
    }
}

/// Removes the last `AssistantText` entry when it is a verbatim echo of
/// `last_assistant_text`. The parsers record every assistant text both as a
/// work entry and as the turn's last assistant text, so without this pass the
/// final answer is rendered twice by every consumer (TUI Detail, handoff
/// Markdown, CLI `--turn`). Earlier assistant texts — including ones that
/// happen to repeat the final answer mid-turn — are kept as genuine
/// intermediate work.
fn strip_final_answer_echo(turn: &mut ContextTurn) {
    let Some(answer) = turn.last_assistant_text.as_deref() else {
        return;
    };
    if let Some(idx) = turn
        .entries
        .iter()
        .rposition(|e| e.kind == ContextEntryKind::AssistantText)
    {
        if turn.entries[idx].text == answer {
            turn.entries.remove(idx);
        }
    }
}

enum DetailedParse {
    Parsed(Vec<ContextTurn>),
    /// The source transcript path is unknown or the file no longer exists.
    SourceMissing,
    /// Reading/parsing the source failed.
    Failed,
    /// Detailed content is not available for this session by design
    /// (e.g. Antigravity conversation without a readable transcript log).
    UserTurnsOnly,
}

fn parse_detailed(session: &Session) -> DetailedParse {
    let Some(source) = session.source_path.as_deref() else {
        return DetailedParse::SourceMissing;
    };
    match session.agent {
        Agent::Claude | Agent::Codex => {
            if !source.is_file() {
                return DetailedParse::SourceMissing;
            }
            let parsed = match session.agent {
                Agent::Claude => claude::parse_turns(source),
                Agent::Codex => codex::parse_turns(source),
                Agent::Antigravity => unreachable!(),
            };
            match parsed {
                Ok(turns) => DetailedParse::Parsed(turns),
                Err(_) => DetailedParse::Failed,
            }
        }
        // Antigravity stores conversation bodies in protobuf payloads inside
        // SQLite, but a readable JSONL transcript is also written under
        // brain/<id>/.system_generated/logs/. Parse that instead of the DB.
        Agent::Antigravity => {
            if !source.exists() {
                return DetailedParse::SourceMissing;
            }
            match antigravity::transcript_path(source, &session.id) {
                Some(path) => match antigravity::parse_turns(&path) {
                    Ok(turns) => DetailedParse::Parsed(turns),
                    Err(_) => DetailedParse::Failed,
                },
                None => DetailedParse::UserTurnsOnly,
            }
        }
    }
}

/// Fallback turns built from the list parser's pre-extracted user turns
/// (no assistant text or work entries).
fn fallback_turns(session: &Session) -> Vec<ContextTurn> {
    session
        .user_turns
        .iter()
        .map(|user| ContextTurn {
            user: cleanup_user_text(user),
            last_assistant_text: None,
            entries: Vec::new(),
        })
        .collect()
}

// ---- Shared turn-building helpers (used by all three detailed parsers) ----

/// Promotes Q&As (agent question -> user answer) to a virtual user turn formatted as
/// `· Question -> Answer`. The session list parser formats turns the same way, so the
/// detailed views must promote identically to keep turn numbers consistent.
pub(crate) fn promote_qa_turn(
    turns: &mut Vec<ContextTurn>,
    current: &mut Option<ContextTurn>,
    qa: &str,
) {
    if qa.trim().is_empty() || is_noise_turn(qa) {
        return;
    }
    if let Some(done) = current.take() {
        turns.push(done);
    }
    *current = Some(ContextTurn {
        user: cleanup_user_text(qa),
        last_assistant_text: None,
        entries: Vec::new(),
    });
}

/// Appends a work entry to the current turn (redacted, consecutive duplicates skipped).
/// Entries arriving while no turn is open (e.g. bootstrap-noise boundaries) are dropped.
pub(crate) fn push_entry(current: &mut Option<ContextTurn>, kind: ContextEntryKind, text: String) {
    if text.trim().is_empty() {
        return;
    }
    if let Some(turn) = current.as_mut() {
        let text = redact(&text);
        if turn
            .entries
            .last()
            .map(|last| last.kind == kind && last.text == text)
            .unwrap_or(false)
        {
            return;
        }
        turn.entries.push(ContextEntry { kind, text });
    }
}

/// Records the (redacted) text as the turn's last assistant text.
pub(crate) fn set_last_assistant(current: &mut Option<ContextTurn>, text: &str) {
    let clean = redact(text);
    if clean.trim().is_empty() {
        return;
    }
    if let Some(turn) = current.as_mut() {
        turn.last_assistant_text = Some(clean);
    }
}

/// Compresses IDE-injected metadata blocks in user text and redacts secrets.
pub(crate) fn cleanup_user_text(turn: &str) -> String {
    let mut out = Vec::new();
    let mut lines = turn.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        match trimmed {
            "# Context from my IDE setup:" | "## My request for Codex:" => continue,
            "## Active file:" => {
                if let Some(value) = next_nonempty(&mut lines) {
                    out.push(format!("Active file: {value}"));
                }
            }
            "## Active selection of the file:" => {
                let selection = collect_until_heading(&mut lines);
                if !selection.is_empty() {
                    out.push(format!(
                        "Active selection: {}",
                        trim_chars(&one_line(&selection.join(" ")), 220)
                    ));
                }
            }
            "## Open tabs:" => {
                let tabs = collect_until_heading(&mut lines);
                let tabs: Vec<String> = tabs
                    .into_iter()
                    .map(|t| t.trim_start_matches("- ").to_string())
                    .filter(|t| !t.is_empty())
                    .take(8)
                    .collect();
                if !tabs.is_empty() {
                    out.push(format!("Open tabs: {}", tabs.join(", ")));
                }
            }
            _ => out.push(line.to_string()),
        }
    }

    redact(&out.join("\n"))
}

fn next_nonempty<'a, I>(lines: &mut std::iter::Peekable<I>) -> Option<String>
where
    I: Iterator<Item = &'a str>,
{
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            lines.next();
        } else {
            break;
        }
    }
    lines.next().map(|line| line.trim().to_string())
}

fn collect_until_heading<'a, I>(lines: &mut std::iter::Peekable<I>) -> Vec<String>
where
    I: Iterator<Item = &'a str>,
{
    let mut out = Vec::new();
    while let Some(line) = lines.peek() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            break;
        }
        out.push(line.to_string());
        lines.next();
    }
    out
}

pub(crate) fn compact_json(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Unicode-safe char-count truncation with a trailing ellipsis.
fn trim_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: ContextEntryKind, text: &str) -> ContextEntry {
        ContextEntry {
            kind,
            text: text.to_string(),
        }
    }

    #[test]
    fn strip_final_answer_echo_removes_trailing_duplicate() {
        let mut turn = ContextTurn {
            user: "질문".to_string(),
            last_assistant_text: Some("최종 답변".to_string()),
            entries: vec![
                entry(ContextEntryKind::AssistantText, "중간 설명"),
                entry(ContextEntryKind::ToolCall, "cargo test"),
                entry(ContextEntryKind::AssistantText, "최종 답변"),
            ],
        };
        strip_final_answer_echo(&mut turn);
        assert_eq!(turn.entries.len(), 2);
        assert!(turn.entries.iter().all(|e| e.text != "최종 답변"));
        assert_eq!(turn.last_assistant_text.as_deref(), Some("최종 답변"));
    }

    #[test]
    fn strip_final_answer_echo_removes_even_when_tools_follow() {
        // task-notification flow: the last assistant text is not the last entry.
        let mut turn = ContextTurn {
            user: "질문".to_string(),
            last_assistant_text: Some("답변".to_string()),
            entries: vec![
                entry(ContextEntryKind::AssistantText, "답변"),
                entry(ContextEntryKind::ToolResult, "<task-notification>done"),
            ],
        };
        strip_final_answer_echo(&mut turn);
        assert_eq!(turn.entries.len(), 1);
        assert_eq!(turn.entries[0].kind, ContextEntryKind::ToolResult);
    }

    #[test]
    fn strip_final_answer_echo_keeps_earlier_identical_text() {
        // A mid-turn repetition of the final answer text is genuine work; only
        // the trailing echo (the entry that produced last_assistant_text) goes.
        let mut turn = ContextTurn {
            user: "질문".to_string(),
            last_assistant_text: Some("같은 문장".to_string()),
            entries: vec![
                entry(ContextEntryKind::AssistantText, "같은 문장"),
                entry(ContextEntryKind::ToolCall, "ls"),
                entry(ContextEntryKind::AssistantText, "같은 문장"),
            ],
        };
        strip_final_answer_echo(&mut turn);
        assert_eq!(turn.entries.len(), 2);
        assert_eq!(turn.entries[0].kind, ContextEntryKind::AssistantText);
        assert_eq!(turn.entries[0].text, "같은 문장");
    }

    #[test]
    fn strip_final_answer_echo_no_op_without_match() {
        let mut turn = ContextTurn {
            user: "질문".to_string(),
            last_assistant_text: None,
            entries: vec![entry(ContextEntryKind::AssistantText, "텍스트")],
        };
        strip_final_answer_echo(&mut turn);
        assert_eq!(turn.entries.len(), 1);

        let mut turn = ContextTurn {
            user: "질문".to_string(),
            last_assistant_text: Some("다른 답".to_string()),
            entries: vec![entry(ContextEntryKind::AssistantText, "텍스트")],
        };
        strip_final_answer_echo(&mut turn);
        assert_eq!(turn.entries.len(), 1);
    }

    /// Manual parity audit over the machine's real sessions: for every session
    /// whose detailed parse succeeds, the context turn count must equal the list
    /// parser's Q count (list == Detail == CLI invariant). Antigravity is
    /// report-only in the strict assertion: its list (SQLite DB) and detail
    /// (transcript log) come from different stores, and the DB list parser can
    /// undercount newer payload shapes — detail >= list is tolerated there
    /// (transcript truncation already falls back to UserTurnsOnly in `load`).
    /// Run explicitly with:
    /// `cargo test real_data_turn_parity -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_data_turn_parity() {
        let profiles = crate::profile::ProfileStore::load();
        let result = crate::scan::scan(&profiles.profiles, false);
        let mut checked = 0usize;
        let mut mismatches = 0usize;
        for s in &result.sessions {
            let ctx = load(s);
            if ctx.completeness != ContextCompleteness::Full {
                continue;
            }
            checked += 1;
            if ctx.turns.len() == s.user_turns.len() {
                continue;
            }
            if s.agent == Agent::Antigravity && ctx.turns.len() > s.user_turns.len() {
                eprintln!(
                    "INFO agy detail>list (DB list undercount) {} list={} ctx={}",
                    s.id,
                    s.user_turns.len(),
                    ctx.turns.len()
                );
                continue;
            }
            mismatches += 1;
            eprintln!(
                "MISMATCH {} {} list={} ctx={}",
                s.agent.key(),
                s.id,
                s.user_turns.len(),
                ctx.turns.len()
            );
        }
        eprintln!("checked {checked} full-context sessions, {mismatches} mismatches");
        assert_eq!(mismatches, 0, "turn-count parity violated");
    }
}
