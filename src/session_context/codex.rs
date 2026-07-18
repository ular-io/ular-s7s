//! OpenAI Codex detailed turn parser (session context).
//!
//! Backtrack parity: `event_msg` `thread_rolled_back {num_turns:N}` markers drop
//! the last N user turns (with their attached QA/entries) using the same boundary
//! accounting as the list parser (`parser::codex`), so rolled-back turns never
//! appear in detailed context either. Boundaries are recorded even for
//! noise-filtered user messages because `num_turns` counts real CLI turns.

use super::model::{ContextEntryKind, ContextTurn};
use super::{cleanup_user_text, compact_json, promote_qa_turn, push_entry, set_last_assistant};
use crate::parser::{clean_turn, is_noise_turn, turn};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

pub fn parse_turns(path: &Path) -> Result<Vec<ContextTurn>> {
    let content = std::fs::read_to_string(path)?;
    let mut turns: Vec<ContextTurn> = Vec::new();
    let mut current: Option<ContextTurn> = None;
    // Completed-turn count in `turns` at each user-message boundary; used to
    // truncate the last N turns on a rollback marker (list-parser parity).
    let mut turn_starts: Vec<usize> = Vec::new();

    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ty = v.get("type").and_then(Value::as_str);
        let payload = v.get("payload").unwrap_or(&v);

        if let Some(n) = rolled_back_turns(&v) {
            // Flush the in-progress turn so truncation sees every completed turn.
            if let Some(done) = current.take() {
                turns.push(done);
            }
            let keep = turn_starts.len().saturating_sub(n);
            let cut = turn_starts.get(keep).copied().unwrap_or(turns.len());
            turns.truncate(cut);
            turn_starts.truncate(keep);
            continue;
        }

        // AskUserQuestion response is promoted to a virtual user turn, matching the list view.
        if let Some(qa) = turn::extract_question_answers(&v) {
            promote_qa_turn(&mut turns, &mut current, &qa);
            continue;
        }

        match ty {
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("user_message") =>
            {
                if let Some(done) = current.take() {
                    turns.push(done);
                }
                // Boundary recorded even for noise-filtered messages (rollback parity).
                turn_starts.push(turns.len());
                if let Some(text) = payload.get("message").and_then(Value::as_str) {
                    // Turn acceptance mirrors the list parser exactly (`clean_turn`
                    // gate): image-only inputs arrive as empty user_message events
                    // and must not open a turn, or CLI turn numbers would drift
                    // from the list's Q count.
                    if !is_noise_turn(text) && clean_turn(text).is_some() {
                        current = Some(ContextTurn {
                            user: cleanup_user_text(text),
                            last_assistant_text: None,
                            entries: Vec::new(),
                        });
                    }
                }
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("agent_message") =>
            {
                if let Some(text) = payload.get("message").and_then(Value::as_str) {
                    set_last_assistant(&mut current, text);
                    push_entry(
                        &mut current,
                        ContextEntryKind::AssistantText,
                        text.to_string(),
                    );
                }
            }
            Some("response_item") => match payload.get("type").and_then(Value::as_str) {
                Some("message")
                    if payload.get("role").and_then(Value::as_str) == Some("assistant") =>
                {
                    if let Some(text) = codex_message_text(payload) {
                        set_last_assistant(&mut current, &text);
                        push_entry(&mut current, ContextEntryKind::AssistantText, text);
                    }
                }
                Some("function_call") | Some("custom_tool_call") => {
                    push_entry(
                        &mut current,
                        ContextEntryKind::ToolCall,
                        compact_json(payload),
                    );
                }
                Some("function_call_output") | Some("custom_tool_call_output") => {
                    push_entry(
                        &mut current,
                        ContextEntryKind::ToolResult,
                        compact_json(payload),
                    );
                }
                _ => {}
            },
            _ => {}
        }
    }

    if let Some(done) = current {
        turns.push(done);
    }
    Ok(turns)
}

/// Returns `num_turns` when the line is a backtrack marker
/// (`event_msg` with `payload.type=="thread_rolled_back"`).
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

fn codex_message_text(payload: &Value) -> Option<String> {
    let content = payload.get("content")?;
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item
                    .get("text")
                    .or_else(|| item.get("input_text"))
                    .or_else(|| item.get("output_text"))
                    .and_then(Value::as_str)
                {
                    parts.push(text.to_string());
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

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "s7s-ctx-codex-{}-{}.jsonl",
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
    fn rollback_removes_user_assistant_and_tool_events() {
        let content = r#"
{"type":"session_meta","payload":{"id":"x1","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"첫 답"}}
{"type":"event_msg","payload":{"type":"user_message","message":"버려질 질문"}}
{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"rm -rf x\"}"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"버려질 답"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"수정된 답"}}
"#;
        let path = write_temp("rollback", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user, "첫 질문");
        assert_eq!(turns[1].user, "수정된 질문");
        assert_eq!(turns[1].last_assistant_text.as_deref(), Some("수정된 답"));
        let joined: String = turns
            .iter()
            .flat_map(|t| t.entries.iter())
            .map(|e| e.text.clone())
            .collect();
        assert!(!joined.contains("버려질 답"));
        assert!(!joined.contains("rm -rf x"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rollback_drops_promoted_qa_with_its_turn() {
        let content = r#"
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"response_item","payload":{"toolUseResult":{"questions":[{"question":"진행할까요?"}],"answers":{"진행할까요?":"네"}}}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
        let path = write_temp("rollback-qa", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user, "수정된 질문");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rollback_counts_noise_user_messages_as_boundaries() {
        // "/usage" is filtered as noise but still counts as one CLI turn for num_turns.
        let content = r#"
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"event_msg","payload":{"type":"user_message","message":"/usage"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
        let path = write_temp("rollback-noise", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(
            turns.iter().map(|t| t.user.clone()).collect::<Vec<_>>(),
            vec!["첫 질문", "수정된 질문"]
        );
        let _ = std::fs::remove_file(path);
    }
}
