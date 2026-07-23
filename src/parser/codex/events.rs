//! Shared Codex rollout record decoding and backtrack reduction (R13).
//!
//! One decoder serves both consumers of the Codex rollout JSONL format — the
//! lightweight list indexer (`parser::codex`) and the detailed context parser
//! (`session_context::codex`) — so a storage-format change cannot silently
//! diverge the two views (list/detail turn parity).
//!
//! Codex is decoded in a single streaming pass: each line classifies into one
//! [`CodexRecord`], and the `thread_rolled_back` backtrack (esc-esc "edit
//! previous message") truncates the most recent turns in file order. There is
//! no leaf-known-at-end pre-pass as in Claude, so records borrow from the
//! caller's per-line `Value` and are consumed within the same iteration; the
//! rollback boundary accounting stays in each consumer's accumulator (the list
//! tracks `(indexable, answer)` per turn, the context tracks `ContextTurn`s).
//!
//! Detailed tool call/result payloads are never materialized here — the decoder
//! hands back the raw `Value` and the context parser serializes it itself, so
//! list indexing stays lightweight (§5.5, §15.3).

use crate::parser::{clean_turn, is_noise_turn, record_timestamp_ms, turn};
use serde_json::Value;

/// One decoded rollout line. Classification is mutually exclusive by line
/// `type` / `payload.type`, so a line maps to exactly one record.
pub(crate) enum CodexRecord<'a> {
    /// `session_meta` — session identity (list uses it; context ignores it).
    Meta {
        id: Option<&'a str>,
        cwd: Option<&'a str>,
    },
    /// `ai-title` body event with non-empty trimmed text (list-only title hint).
    Title(&'a str),
    /// `thread_rolled_back` backtrack marker: drop the most recent `num_turns`
    /// turns. Boundaries are counted for every [`CodexRecord::User`] (including
    /// noise), so `num_turns` counts real CLI turns.
    RolledBack(usize),
    /// A user turn in either supported form (unified in R13 — see [`decode`]).
    User(UserRecord),
    /// An `AskUserQuestion`/`ask_question` response, formatted `· question →
    /// answer`. Each consumer applies its own turn gate / promotion.
    Qa {
        text: String,
        submitted_at_ms: Option<i64>,
    },
    /// One assistant answer's text (already empty-filtered).
    Assistant(String),
    /// `response_item` tool call; carries the raw payload so only the context
    /// parser pays for JSON serialization.
    ToolCall(&'a Value),
    /// `response_item` tool result; carries the raw payload (see `ToolCall`).
    ToolResult(&'a Value),
    /// Any other line; contributes nothing to either consumer.
    Other,
}

/// One decoded user turn. `text` is the raw extracted text (the context parser
/// runs its own IDE-block cleanup on it); `kind` is the shared turn-acceptance
/// classification used by both consumers.
pub(crate) struct UserRecord {
    pub text: String,
    pub kind: UserTextKind,
    pub submitted_at_ms: Option<i64>,
}

pub(crate) enum UserTextKind {
    /// Countable question: passed the noise filter and `clean_turn`. The payload
    /// is the cleaned (NFC-normalized, trimmed) text used by the list index; the
    /// context parser keeps working from the raw `text` field instead.
    Turn { cleaned: String },
    /// Extractable user text that is noise (slash command, bootstrap prompt) or
    /// blank after cleaning. It still records a rollback boundary in both
    /// consumers but opens no turn.
    Boundary,
}

/// Decodes one rollout line into its single contributing record.
///
/// The user-turn form is unified in R13: both consumers now accept a turn via
/// [`turn::extract_user_text`], which covers the `event_msg` `user_message`
/// form *and* the `response_item` `role == "user"` form. The context parser
/// previously read only the `event_msg` form, so a `response_item` user turn
/// would have diverged the list Q count from the detail turn count; the shared
/// decoder removes that divergence.
pub(crate) fn decode(v: &Value) -> CodexRecord<'_> {
    match v.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            let payload = v.get("payload");
            return CodexRecord::Meta {
                id: payload.and_then(|p| p.get("id")).and_then(Value::as_str),
                cwd: payload.and_then(|p| p.get("cwd")).and_then(Value::as_str),
            };
        }
        Some("ai-title") => {
            return match v
                .get("aiTitle")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                Some(t) => CodexRecord::Title(t),
                None => CodexRecord::Other,
            };
        }
        Some("event_msg") | Some("response_item") => {}
        _ => return CodexRecord::Other,
    }

    if let Some(n) = rolled_back_turns(v) {
        return CodexRecord::RolledBack(n);
    }
    if let Some(text) = turn::extract_user_text(v) {
        let kind = match (!is_noise_turn(&text)).then(|| clean_turn(&text)).flatten() {
            Some(cleaned) => UserTextKind::Turn { cleaned },
            None => UserTextKind::Boundary,
        };
        return CodexRecord::User(UserRecord {
            text,
            kind,
            submitted_at_ms: record_timestamp_ms(v),
        });
    }
    if let Some(qa) = turn::extract_question_answers(v) {
        return CodexRecord::Qa {
            text: qa,
            submitted_at_ms: record_timestamp_ms(v),
        };
    }
    if let Some(text) = assistant_text(v) {
        return CodexRecord::Assistant(text);
    }
    // Detailed tool call/result records are recognized only under `response_item`
    // (the only line type that carries them), matching the context parser.
    if v.get("type").and_then(Value::as_str) == Some("response_item") {
        if let Some(payload) = v.get("payload") {
            match payload.get("type").and_then(Value::as_str) {
                Some("function_call") | Some("custom_tool_call") => {
                    return CodexRecord::ToolCall(payload);
                }
                Some("function_call_output") | Some("custom_tool_call_output") => {
                    return CodexRecord::ToolResult(payload);
                }
                _ => {}
            }
        }
    }
    CodexRecord::Other
}

/// Returns `num_turns` when the line is a backtrack marker
/// (`event_msg` with `payload.type == "thread_rolled_back"`).
fn rolled_back_turns(v: &Value) -> Option<usize> {
    let payload = v.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("thread_rolled_back") {
        return None;
    }
    Some(
        payload
            .get("num_turns")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
    )
}

/// Extracts assistant answer text from an `event_msg`/`response_item` line, or
/// None when the line is not a (non-empty) assistant message. Empty text is
/// filtered here so both consumers agree (unified in R13 — the context parser's
/// downstream `push_entry`/`set_last_assistant` already dropped empty text, so
/// this only makes the shared rule explicit).
fn assistant_text(v: &Value) -> Option<String> {
    let payload = v.get("payload").unwrap_or(v);
    match payload.get("type").and_then(Value::as_str) {
        Some("agent_message") => payload
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty()),
        Some("message") if payload.get("role").and_then(Value::as_str) == Some("assistant") => {
            message_text(payload)
        }
        _ => None,
    }
}

/// Joins the non-empty text parts of a `response_item` assistant message.
fn message_text(payload: &Value) -> Option<String> {
    match payload.get("content")? {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item
                    .get("text")
                    .or_else(|| item.get("input_text"))
                    .or_else(|| item.get("output_text"))
                    .and_then(Value::as_str)
                {
                    if !text.trim().is_empty() {
                        parts.push(text.to_string());
                    }
                }
            }
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(json: &str) -> Value {
        serde_json::from_str(json).expect("valid json")
    }

    #[test]
    fn decode_classifies_user_turn_and_boundary() {
        let turn =
            val(r#"{"type":"event_msg","payload":{"type":"user_message","message":"질문"}}"#);
        let noise =
            val(r#"{"type":"event_msg","payload":{"type":"user_message","message":"/usage"}}"#);

        let CodexRecord::User(u) = decode(&turn) else {
            panic!("expected user turn");
        };
        assert!(matches!(u.kind, UserTextKind::Turn { cleaned } if cleaned == "질문"));

        let CodexRecord::User(u) = decode(&noise) else {
            panic!("expected user boundary");
        };
        assert!(matches!(u.kind, UserTextKind::Boundary));
    }

    #[test]
    fn decode_accepts_response_item_user_form() {
        // Unified R13 rule: the context parser used to miss this form, which
        // would drift the detail turn count from the list Q count.
        let v = val(
            r#"{"type":"response_item","payload":{"type":"message","role":"user","content":"질문"}}"#,
        );
        let CodexRecord::User(u) = decode(&v) else {
            panic!("expected user turn from response_item form");
        };
        assert!(matches!(u.kind, UserTextKind::Turn { cleaned } if cleaned == "질문"));
    }

    #[test]
    fn decode_reads_rollback_marker() {
        let v =
            val(r#"{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":2}}"#);
        assert!(matches!(decode(&v), CodexRecord::RolledBack(2)));
    }

    #[test]
    fn decode_classifies_assistant_forms() {
        let agent =
            val(r#"{"type":"event_msg","payload":{"type":"agent_message","message":"답변"}}"#);
        let item = val(
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"답변"}]}}"#,
        );
        for v in [&agent, &item] {
            let CodexRecord::Assistant(text) = decode(v) else {
                panic!("expected assistant");
            };
            assert_eq!(text, "답변");
        }
    }

    #[test]
    fn decode_classifies_tool_call_and_result() {
        let call = val(
            r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{}"}}"#,
        );
        let out = val(
            r#"{"type":"response_item","payload":{"type":"function_call_output","output":"ok"}}"#,
        );
        assert!(matches!(decode(&call), CodexRecord::ToolCall(_)));
        assert!(matches!(decode(&out), CodexRecord::ToolResult(_)));
    }

    #[test]
    fn decode_reads_meta_and_title() {
        let meta = val(r#"{"type":"session_meta","payload":{"id":"x1","cwd":"/tmp/demo"}}"#);
        let CodexRecord::Meta { id, cwd } = decode(&meta) else {
            panic!("expected meta");
        };
        assert_eq!(id, Some("x1"));
        assert_eq!(cwd, Some("/tmp/demo"));

        let title = val(r#"{"type":"ai-title","aiTitle":" 제목 "}"#);
        assert!(matches!(decode(&title), CodexRecord::Title("제목")));

        let empty_title = val(r#"{"type":"ai-title","aiTitle":"  "}"#);
        assert!(matches!(decode(&empty_title), CodexRecord::Other));
    }

    #[test]
    fn empty_assistant_text_is_filtered() {
        let v = val(r#"{"type":"event_msg","payload":{"type":"agent_message","message":"  "}}"#);
        assert!(matches!(decode(&v), CodexRecord::Other));
    }
}
