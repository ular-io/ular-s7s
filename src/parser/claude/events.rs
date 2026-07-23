//! Shared Claude record decoding and active-branch reduction (R12).
//!
//! One decoder serves both consumers of the Claude JSONL storage format — the
//! lightweight list indexer (`parser::claude`) and the detailed context parser
//! (`session_context::claude`) — so a storage-format change cannot silently
//! diverge the two views (list/detail turn parity).
//!
//! The decoder owns record-shape knowledge only: line parsing, field
//! extraction, event classification, and the `parentUuid` active-branch chain.
//! What each consumer does with an event (Q counting, search blobs,
//! `ContextTurn` construction) stays in that consumer's accumulator, and
//! detailed tool payloads are never materialized here — the context parser
//! extracts them from the raw `Value` it already holds, so list indexing stays
//! lightweight.

use crate::parser::{clean_turn, is_noise_turn, record_timestamp_ms, turn};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Parses a JSONL session body into records. Blank and malformed lines are
/// skipped, never fatal — a format change must never blank a session.
pub(crate) fn parse_lines(content: &str) -> Vec<Value> {
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// One decoded storage record. `uuid` is checked against the [`ActiveFilter`]
/// by both consumers; the parent link stays inside the chain pass
/// ([`chain_filter`] via `record_link`).
pub(crate) struct DecodedRecord<'a> {
    pub uuid: Option<&'a str>,
    pub kind: RecordKind<'a>,
}

pub(crate) enum RecordKind<'a> {
    Title(TitleEvent<'a>),
    User(UserRecord),
    Assistant {
        items: Vec<AssistantItem<'a>>,
        emitted_at_ms: Option<i64>,
    },
    /// End of one Claude turn. This is the closest stored representation of
    /// response completion and is emitted after stop hooks finish.
    TurnCompleted {
        completed_at_ms: Option<i64>,
    },
    /// Any other non-sidechain record; contributes only its chain link.
    Other,
}

/// Session title events appended to the JSONL body. Precedence (explicit over
/// auto) is the consumer's business; the decoder only guarantees non-empty
/// trimmed text.
pub(crate) enum TitleEvent<'a> {
    /// `custom-title` — explicit rename (`/rename`); strongest source.
    Custom(&'a str),
    /// `agent-name` — explicit, weaker than `custom-title`.
    AgentName(&'a str),
    /// `ai-title` — CLI auto title; weakest.
    Ai(&'a str),
}

/// One decoded `type == "user"` record. The fields are independent facts so
/// each consumer can keep its own precedence where they deliberately differ
/// (see `UserTextKind::Blank`).
pub(crate) struct UserRecord {
    /// `AskUserQuestion`/`ask_question` responses formatted `· question → answer`.
    pub qa: Option<String>,
    /// Background task-completion notice. Identified by `origin.kind ==
    /// "task-notification"` with the `<task-notification>` text prefix as the
    /// fallback for older CLI records (the two parsers used to disagree here —
    /// unified in R12). Keeps the current turn open in both consumers.
    pub is_task_notification: bool,
    /// Readable text extracted from the record, if any (raw, uncleaned).
    pub text: Option<String>,
    /// Classification of `text` through the shared turn gates.
    pub text_kind: UserTextKind,
    /// Top-level RFC 3339 record timestamp converted to Unix epoch milliseconds.
    pub submitted_at_ms: Option<i64>,
}

pub(crate) enum UserTextKind {
    /// Countable question: passed the noise filter and `clean_turn`. The payload
    /// is the cleaned (NFC-normalized, trimmed) text used by the list index; the
    /// context parser keeps working from the raw `text` field instead.
    Turn { cleaned: String },
    /// Noise input (slash command, bootstrap prompt, …): closes the current turn
    /// without opening one.
    Boundary,
    /// Extractable but blank after cleaning. The consumers deliberately differ
    /// here (the list keeps the turn open, the context parser closes it) —
    /// preserved pre-R12 behavior.
    Blank,
    /// No extractable text (e.g. a pure tool-result record).
    NoText,
}

/// One ordered item of an assistant message's content array. `ToolUse` carries
/// the raw item so only the context parser pays for JSON serialization; text
/// items are not blank-filtered (each consumer drops blanks itself).
pub(crate) enum AssistantItem<'a> {
    Text(&'a str),
    ToolUse(&'a Value),
}

/// Decodes one record. `None` means the record contributes nothing to either
/// consumer: sidechain (subagent) records and title events with empty text.
pub(crate) fn decode(v: &Value) -> Option<DecodedRecord<'_>> {
    let ty = v.get("type").and_then(Value::as_str);

    // Title events never join the active-branch chain: no observed CLI version
    // writes a uuid on them, and excluding them by rule keeps both consumers
    // agreeing even if one ever appears (unified in R12 — the context parser
    // used to register any uuid-bearing line).
    match ty {
        Some("custom-title") => return title_record(v, "customTitle", TitleEvent::Custom),
        Some("agent-name") => return title_record(v, "agentName", TitleEvent::AgentName),
        Some("ai-title") => return title_record(v, "aiTitle", TitleEvent::Ai),
        _ => {}
    }

    let (uuid, _parent) = record_link(v)?;
    let kind = match ty {
        Some("user") => RecordKind::User(decode_user(v)),
        Some("assistant") => RecordKind::Assistant {
            items: assistant_items(v),
            emitted_at_ms: record_timestamp_ms(v),
        },
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("turn_duration") => {
            RecordKind::TurnCompleted {
                completed_at_ms: record_timestamp_ms(v),
            }
        }
        _ => RecordKind::Other,
    };
    Some(DecodedRecord { uuid, kind })
}

/// Builds the active-branch membership filter for a parsed session body
/// (pass 1 of 2 — the leaf is only known once every line has been seen).
pub(crate) fn chain_filter(values: &[Value]) -> ActiveFilter<'_> {
    let mut branch = ActiveBranch::default();
    for v in values {
        branch.observe(v);
    }
    branch.finish()
}

/// Accumulates uuid → parentUuid links in file order; the last observed uuid is
/// the head (leaf) of the active conversation branch.
#[derive(Default)]
struct ActiveBranch<'a> {
    parents: HashMap<&'a str, Option<&'a str>>,
    leaf: Option<&'a str>,
}

impl<'a> ActiveBranch<'a> {
    fn observe(&mut self, v: &'a Value) {
        if let Some((Some(uuid), parent)) = record_link(v) {
            self.parents.insert(uuid, parent);
            self.leaf = Some(uuid);
        }
    }

    /// Reduces the chain to the active-branch filter (leaf → root walk).
    fn finish(&self) -> ActiveFilter<'a> {
        ActiveFilter {
            active: active_uuid_set(&self.parents, self.leaf),
        }
    }
}

/// Membership filter over the active `parentUuid` branch (`/rewind` semantics).
///
/// `active == None` means the chain cannot be trusted — no leaf, or a dangling
/// parent reference — so every record must be kept (a format change must never
/// blank a session). Records without a uuid are always active.
pub(crate) struct ActiveFilter<'a> {
    active: Option<HashSet<&'a str>>,
}

impl ActiveFilter<'_> {
    pub(crate) fn is_active(&self, uuid: Option<&str>) -> bool {
        match (&self.active, uuid) {
            (Some(set), Some(u)) => set.contains(u),
            _ => true,
        }
    }
}

/// The chain link of one record, shared by pass 1 and `decode` so both apply
/// the same exclusions: sidechain records and title events contribute nothing.
fn record_link(v: &Value) -> Option<(Option<&str>, Option<&str>)> {
    match v.get("type").and_then(Value::as_str) {
        Some("custom-title") | Some("agent-name") | Some("ai-title") => return None,
        _ => {}
    }
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    Some((
        v.get("uuid").and_then(Value::as_str),
        v.get("parentUuid").and_then(Value::as_str),
    ))
}

/// Set of uuids on the active branch (leaf → root walk over `parentUuid` links).
///
/// `/rewind` leaves abandoned turns in the append-only file as a dead branch;
/// only entries reachable from the leaf are alive. Returns None when the chain
/// cannot be trusted — no leaf, or a dangling parent reference — meaning the
/// caller must keep every entry.
fn active_uuid_set<'a>(
    parents: &HashMap<&'a str, Option<&'a str>>,
    leaf: Option<&'a str>,
) -> Option<HashSet<&'a str>> {
    let leaf = leaf?;
    let mut active: HashSet<&str> = HashSet::new();
    let mut cursor = Some(leaf);
    while let Some(u) = cursor {
        if !active.insert(u) {
            break; // Cycle guard (corrupt file).
        }
        match parents.get(u) {
            Some(parent) => cursor = *parent,
            // Dangling reference: the chain cannot be trusted, keep everything.
            None => return None,
        }
    }
    Some(active)
}

fn title_record<'a>(
    v: &'a Value,
    field: &str,
    make: impl FnOnce(&'a str) -> TitleEvent<'a>,
) -> Option<DecodedRecord<'a>> {
    let text = v
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    Some(DecodedRecord {
        uuid: None,
        kind: RecordKind::Title(make(text)),
    })
}

fn decode_user(v: &Value) -> UserRecord {
    let qa = turn::extract_question_answers(v);
    let text = turn::extract_user_text(v);
    let is_task_notification = origin_is_task_notification(v)
        || text.as_deref().map(is_notification_text).unwrap_or(false);
    let text_kind = match text.as_deref() {
        None => UserTextKind::NoText,
        Some(t) if is_noise_turn(t) => UserTextKind::Boundary,
        Some(t) => match clean_turn(t) {
            Some(cleaned) => UserTextKind::Turn { cleaned },
            None => UserTextKind::Blank,
        },
    };
    UserRecord {
        qa,
        is_task_notification,
        text,
        text_kind,
        submitted_at_ms: record_timestamp_ms(v),
    }
}

fn origin_is_task_notification(v: &Value) -> bool {
    v.get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str)
        == Some("task-notification")
}

fn is_notification_text(text: &str) -> bool {
    text.trim_start().starts_with("<task-notification>")
}

/// Ordered content items of an assistant message (text and tool_use).
fn assistant_items(v: &Value) -> Vec<AssistantItem<'_>> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = v.get("message").and_then(|m| m.get("content")) {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = item.get("text").and_then(Value::as_str) {
                        out.push(AssistantItem::Text(t));
                    }
                }
                Some("tool_use") => out.push(AssistantItem::ToolUse(item)),
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(json: &str) -> Value {
        serde_json::from_str(json).expect("valid json")
    }

    #[test]
    fn decode_classifies_user_text_kinds() {
        let turn = val(r#"{"type":"user","message":{"role":"user","content":"질문"}}"#);
        let noise = val(r#"{"type":"user","message":{"role":"user","content":"/usage"}}"#);
        let blank = val(r#"{"type":"user","message":{"role":"user","content":" \n "}}"#);
        let tool = val(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
        );

        for (v, expect) in [
            (&turn, "turn"),
            (&noise, "boundary"),
            (&blank, "blank"),
            (&tool, "notext"),
        ] {
            let Some(RecordKind::User(u)) = decode(v).map(|d| d.kind) else {
                panic!("expected user record");
            };
            let got = match u.text_kind {
                UserTextKind::Turn { ref cleaned } => {
                    assert_eq!(cleaned, "질문");
                    "turn"
                }
                UserTextKind::Boundary => "boundary",
                UserTextKind::Blank => "blank",
                UserTextKind::NoText => "notext",
            };
            assert_eq!(got, expect);
        }
    }

    #[test]
    fn task_notification_identified_by_origin_kind_without_text_prefix() {
        // The unified R12 identity: `origin.kind` marks a notification even when
        // the text lacks the legacy `<task-notification>` prefix, and vice versa.
        let by_origin = val(
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"role":"user","content":"background job done"}}"#,
        );
        let by_prefix = val(
            r#"{"type":"user","message":{"role":"user","content":"<task-notification>done</task-notification>"}}"#,
        );
        let plain = val(r#"{"type":"user","message":{"role":"user","content":"질문"}}"#);

        for (v, expect) in [(&by_origin, true), (&by_prefix, true), (&plain, false)] {
            let Some(RecordKind::User(u)) = decode(v).map(|d| d.kind) else {
                panic!("expected user record");
            };
            assert_eq!(u.is_task_notification, expect);
        }
    }

    #[test]
    fn title_events_never_join_the_chain() {
        // Even a hypothetical uuid-bearing title event must not divert the leaf
        // (unified R12 rule — the chain belongs to conversation records only).
        let values = vec![
            val(
                r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문"}}"#,
            ),
            val(r#"{"type":"custom-title","uuid":"t","parentUuid":null,"customTitle":"제목"}"#),
        ];
        let filter = chain_filter(&values);
        assert!(filter.is_active(Some("a")));
        assert!(!filter.is_active(Some("t")));
    }

    #[test]
    fn sidechain_records_neither_decode_nor_join_the_chain() {
        let side = val(
            r#"{"type":"user","uuid":"s","parentUuid":null,"isSidechain":true,"message":{"role":"user","content":"사이드체인"}}"#,
        );
        assert!(decode(&side).is_none());

        let values = vec![
            val(
                r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문"}}"#,
            ),
            side,
        ];
        let filter = chain_filter(&values);
        // The trailing sidechain entry must not become the leaf.
        assert!(filter.is_active(Some("a")));
    }

    #[test]
    fn broken_chain_keeps_every_record_active() {
        let values = vec![
            val(
                r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}"#,
            ),
            val(
                r#"{"type":"user","uuid":"c","parentUuid":"ghost","message":{"role":"user","content":"질문2"}}"#,
            ),
        ];
        let filter = chain_filter(&values);
        assert!(filter.is_active(Some("a")));
        assert!(filter.is_active(Some("c")));
        assert!(filter.is_active(None));
    }

    #[test]
    fn assistant_items_preserve_content_order() {
        let v = val(
            r#"{"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"text","text":"먼저"},{"type":"tool_use","name":"Bash"},{"type":"text","text":"나중"}]}}"#,
        );
        let Some(RecordKind::Assistant { items, .. }) = decode(&v).map(|d| d.kind) else {
            panic!("expected assistant record");
        };
        let shape: Vec<&str> = items
            .iter()
            .map(|i| match i {
                AssistantItem::Text(t) => *t,
                AssistantItem::ToolUse(_) => "<tool>",
            })
            .collect();
        assert_eq!(shape, vec!["먼저", "<tool>", "나중"]);
    }

    #[test]
    fn empty_title_event_decodes_to_nothing() {
        let v = val(r#"{"type":"custom-title","customTitle":"  "}"#);
        assert!(decode(&v).is_none());
    }
}
