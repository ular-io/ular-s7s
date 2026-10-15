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
    ContextCompleteness, ContextEntry, ContextEntryKind, ContextSourceRef, ContextSourceState,
    ContextTurn, SessionContext, SessionContextSource,
};

use crate::model::{Agent, Session};
use crate::parser::is_noise_turn;
use redact::redact;
use serde_json::Value;

/// Loads the session context. Never fails outright: when detailed parsing is not
/// possible the turns fall back to the pre-extracted `Session::user_turns` and
/// `completeness` records why, so consumers can state that assistant/work entries
/// are unavailable instead of silently pretending the context is complete.
///
/// A recorded context source is carried through unverified. Use
/// [`load_in_index`] where the scanned session set is at hand so the output can
/// also say whether that source is still reachable.
pub fn load(session: &Session) -> SessionContext {
    build(session, None)
}

/// [`load`] plus resolution of the session's context source against `index`
/// (matched by agent + id, the rule `ui::context_jump` uses), so a deleted or
/// unscanned source is reported as unavailable instead of as a live reference.
pub fn load_in_index(session: &Session, index: &[Session]) -> SessionContext {
    build(session, Some(index))
}

fn build(session: &Session, index: Option<&[Session]>) -> SessionContext {
    let source = SessionContextSource {
        agent: session.agent,
        profile_id: session.profile_id.clone(),
        session_id: session.id.clone(),
        title: session.title(),
        cwd: session.cwd.clone(),
        context_source: session
            .context_source
            .clone()
            .map(|source| ContextSourceRef {
                state: context_source_state(&source, index),
                source,
            }),
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

    if session.agent == Agent::Antigravity {
        attach_antigravity_timestamps(session, &mut turns);
    }

    for turn in &mut turns {
        strip_final_answer_echo(turn);
    }

    SessionContext {
        source,
        completeness,
        turns,
    }
}

/// Resolves one recorded context source against the index it was loaded with.
/// Without an index the state stays `Unverified`: the reference is reported, and
/// nothing is claimed about whether it can still be read.
fn context_source_state(
    source: &crate::model::ContextSource,
    index: Option<&[Session]>,
) -> ContextSourceState {
    match index {
        None => ContextSourceState::Unverified,
        Some(sessions)
            if sessions
                .iter()
                .any(|s| s.agent == source.agent && s.id == source.id) =>
        {
            ContextSourceState::Present
        }
        Some(_) => ContextSourceState::Missing,
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
            if !source.is_file()
                && (session.agent != Agent::Codex
                    || !crate::parser::codex::rollout::resolve_path(source).is_file())
            {
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
                Some(path) => match antigravity::parse_turns_with_activity_for_db(&path, source) {
                    Ok((turns, _)) => DetailedParse::Parsed(turns),
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
        .enumerate()
        .map(|(index, user)| ContextTurn {
            user: cleanup_user_text(user),
            submitted_at_ms: session.user_turn_timestamp_ms(index),
            last_assistant_text: None,
            entries: Vec::new(),
        })
        .collect()
}

/// Antigravity detail content comes from its transcript while submit times were
/// captured from the conversation DB by the list parser. Attach by turn order
/// only when both stores agree on the full turn count; partial pairing would
/// show a plausible but wrong time.
fn attach_antigravity_timestamps(session: &Session, turns: &mut [ContextTurn]) {
    if session.user_turn_timestamps_ms.len() != turns.len() {
        return;
    }
    for (turn, timestamp) in turns.iter_mut().zip(&session.user_turn_timestamps_ms) {
        turn.submitted_at_ms = *timestamp;
    }
}

// ---- Shared turn-building helpers (used by all three detailed parsers) ----

/// Promotes Q&As (agent question -> user answer) to a virtual user turn formatted as
/// `· Question -> Answer`. The session list parser formats turns the same way, so the
/// detailed views must promote identically to keep turn numbers consistent.
pub(crate) fn promote_qa_turn(
    turns: &mut Vec<ContextTurn>,
    current: &mut Option<ContextTurn>,
    qa: &str,
    submitted_at_ms: Option<i64>,
) {
    if qa.trim().is_empty() || is_noise_turn(qa) {
        return;
    }
    if let Some(done) = current.take() {
        turns.push(done);
    }
    *current = Some(ContextTurn {
        user: cleanup_user_text(qa),
        submitted_at_ms,
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
mod tests;
