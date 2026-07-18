//! Claude Code detailed turn parser (session context).
//!
//! Same source file as the list parser (`~/.claude/projects/<dir>/<id>.jsonl`),
//! but extracts full turns (user text, tool calls/results, assistant text).
//!
//! `/rewind` parity: entries are filtered to the active `parentUuid` chain via
//! the same helper the list parser uses (`parser::claude::active_uuid_set`), so
//! abandoned turns never appear in detailed context either.

use super::model::{ContextEntry, ContextEntryKind, ContextTurn};
use super::{cleanup_user_text, compact_json, promote_qa_turn, push_entry, set_last_assistant};
use crate::parser::claude::active_uuid_set;
use crate::parser::{clean_turn, is_noise_turn, turn};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub fn parse_turns(path: &Path) -> Result<Vec<ContextTurn>> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<Value> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|v: &Value| v.get("isSidechain").and_then(Value::as_bool) != Some(true))
        .collect();

    // Build the active-path uuid set exactly like the list parser (rewind parity).
    let mut parents: HashMap<String, Option<String>> = HashMap::new();
    let mut leaf: Option<String> = None;
    for v in &lines {
        if let Some(u) = v.get("uuid").and_then(Value::as_str) {
            let parent = v
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(str::to_string);
            parents.insert(u.to_string(), parent);
            leaf = Some(u.to_string());
        }
    }
    // None = chain broken or absent: keep everything (same fallback as the list parser).
    let active = active_uuid_set(&parents, leaf.as_deref());

    let mut turns: Vec<ContextTurn> = Vec::new();
    let mut current: Option<ContextTurn> = None;

    for v in &lines {
        if let (Some(active), Some(u)) = (&active, v.get("uuid").and_then(Value::as_str)) {
            if !active.contains(u) {
                continue; // Abandoned by /rewind.
            }
        }

        // AskUserQuestion response is promoted to a virtual user turn, matching the list view
        // (not recorded redundantly as a ToolResult).
        if let Some(qa) = turn::extract_question_answers(v) {
            promote_qa_turn(&mut turns, &mut current, &qa);
            continue;
        }

        match v.get("type").and_then(Value::as_str) {
            // Background task notifications are injected as user-role entries but are
            // not human questions: keep the current turn open so the follow-up
            // assistant answer attaches to the original question, and record the
            // notification body as a tool result.
            Some("user") if is_task_notification(v) => {
                if let Some(text) = turn::extract_user_text(v) {
                    push_entry(&mut current, ContextEntryKind::ToolResult, text);
                }
            }
            // Turn acceptance mirrors the list parser exactly: any user entry whose
            // extractable text survives the noise filter and `clean_turn` is a turn.
            // Old CLI records lack `promptSource`/`origin`, so an is-human field
            // check here would drop turns the list counts and break turn-number
            // parity between the list, Detail screen, and CLI.
            Some("user") if has_countable_user_text(v) => {
                if let Some(done) = current.take() {
                    turns.push(done);
                }
                if let Some(text) = turn::extract_user_text(v) {
                    if !is_noise_turn(&text) && clean_turn(&text).is_some() {
                        current = Some(ContextTurn {
                            user: cleanup_user_text(&text),
                            last_assistant_text: None,
                            entries: Vec::new(),
                        });
                    }
                }
            }
            Some("user") => {
                if let Some(entry) = claude_tool_result(v) {
                    push_entry(&mut current, ContextEntryKind::ToolResult, entry);
                }
            }
            Some("assistant") => {
                for entry in claude_assistant_entries(v) {
                    if matches!(entry.kind, ContextEntryKind::AssistantText) {
                        set_last_assistant(&mut current, &entry.text);
                    }
                    push_entry(&mut current, entry.kind, entry.text);
                }
            }
            _ => {}
        }
    }

    if let Some(done) = current {
        turns.push(done);
    }
    Ok(turns)
}

/// Detects background task completion notices (`origin.kind == "task-notification"`,
/// `promptSource: "system"`). The text prefix is a fallback for older CLI records.
fn is_task_notification(v: &Value) -> bool {
    if v.get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(Value::as_str)
        == Some("task-notification")
    {
        return true;
    }
    turn::extract_user_text(v)
        .map(|t| t.trim_start().starts_with("<task-notification>"))
        .unwrap_or(false)
}

/// Whether the user entry carries text the list parser would extract. Noise text
/// still matches (a noise user input ends the previous turn without opening a new
/// one); entries with no extractable text (pure tool results) do not.
fn has_countable_user_text(v: &Value) -> bool {
    turn::extract_user_text(v).is_some()
}

fn claude_tool_result(v: &Value) -> Option<String> {
    if let Some(result) = v.get("toolUseResult") {
        return Some(compact_json(result));
    }
    let content = v.get("message")?.get("content")?;
    match content {
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("tool_result") {
                    if let Some(text) = item.get("content").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    } else {
                        parts.push(compact_json(item));
                    }
                }
            }
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        _ => None,
    }
}

fn claude_assistant_entries(v: &Value) -> Vec<ContextEntry> {
    let mut out = Vec::new();
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return out;
    };
    let Value::Array(items) = content else {
        return out;
    };

    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    out.push(ContextEntry {
                        kind: ContextEntryKind::AssistantText,
                        text: text.to_string(),
                    });
                }
            }
            Some("tool_use") => out.push(ContextEntry {
                kind: ContextEntryKind::ToolCall,
                text: compact_json(item),
            }),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "s7s-ctx-claude-{}-{}.jsonl",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, content).expect("write temp file");
        path
    }

    #[test]
    fn rewind_removes_abandoned_turns_from_detailed_context() {
        // Q2/A2 were abandoned by /rewind: Q3 branches from the same parent (b).
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"origin":{"kind":"human"},"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"c","parentUuid":"b","origin":{"kind":"human"},"message":{"role":"user","content":"질문2 버려진 분기"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"content":[{"type":"text","text":"버려진 답2"}]}}
{"type":"user","uuid":"e","parentUuid":"b","origin":{"kind":"human"},"message":{"role":"user","content":"질문3 리와인드 후"}}
{"type":"assistant","uuid":"f","parentUuid":"e","message":{"content":[{"type":"text","text":"답3"}]}}
"#;
        let path = write_temp("rewind", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user, "질문1");
        assert_eq!(turns[0].last_assistant_text.as_deref(), Some("답1"));
        assert_eq!(turns[1].user, "질문3 리와인드 후");
        assert_eq!(turns[1].last_assistant_text.as_deref(), Some("답3"));
        assert!(turns
            .iter()
            .all(|t| t.entries.iter().all(|e| !e.text.contains("버려진 답2"))));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn broken_chain_keeps_all_turns() {
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"origin":{"kind":"human"},"message":{"role":"user","content":"질문1"}}
{"type":"user","uuid":"c","parentUuid":"ghost","origin":{"kind":"human"},"message":{"role":"user","content":"질문2"}}
"#;
        let path = write_temp("broken", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn bootstrap_prompt_is_not_a_context_turn() {
        // The s7s bootstrap prompt is stored as a user entry by the CLI: it must not
        // open a turn, and the ready response before the first real request must not
        // attach anywhere.
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"origin":{"kind":"human"},"message":{"role":"user","content":"<s7s-context-bootstrap>\nRun `s7s session 'x' --bootstrap`.\n</s7s-context-bootstrap>"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"text","text":"이전 세션 컨텍스트를 확인했습니다."}]}}
{"type":"user","uuid":"c","parentUuid":"b","origin":{"kind":"human"},"message":{"role":"user","content":"진짜 첫 질문"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"content":[{"type":"text","text":"답변"}]}}
"#;
        let path = write_temp("bootstrap", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user, "진짜 첫 질문");
        assert_eq!(turns[0].last_assistant_text.as_deref(), Some("답변"));
        let _ = std::fs::remove_file(path);
    }
}
