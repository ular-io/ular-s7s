//! OpenAI Codex detailed turn parser (session context).
//!
//! Record decoding and the `thread_rolled_back` backtrack marker are shared with
//! the list parser (`parser::codex::events` — R13): turn-acceptance gates, the
//! user-turn form (both `event_msg` and `response_item`), and rollback
//! identification can no longer drift between the two views. This parser owns
//! only the rollback boundary accounting and the detailed tool call/result
//! payloads it serializes from the raw records the shared decoder hands back.
//!
//! Backtrack parity: a `thread_rolled_back {num_turns:N}` marker drops the last
//! N user turns (with their attached QA/entries) using the same boundary
//! accounting as the list parser, so rolled-back turns never appear in detailed
//! context either. Boundaries are recorded even for noise-filtered user messages
//! because `num_turns` counts real CLI turns.

use super::model::{ContextEntryKind, ContextTurn};
use super::{cleanup_user_text, compact_json, promote_qa_turn, push_entry, set_last_assistant};
use crate::parser::codex::events::{self, CodexRecord, UserTextKind};
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

    let decoder = events::Decoder::new(&content);
    for (line_no, line) in content.lines().enumerate() {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match decoder.decode(line_no, &v) {
            CodexRecord::RolledBack(n) => {
                // Flush the in-progress turn so truncation sees every completed turn.
                if let Some(done) = current.take() {
                    turns.push(done);
                }
                let keep = turn_starts.len().saturating_sub(n);
                let cut = turn_starts.get(keep).copied().unwrap_or(turns.len());
                turns.truncate(cut);
                turn_starts.truncate(keep);
            }
            // AskUserQuestion response is promoted to a virtual user turn, matching the list view.
            CodexRecord::Qa {
                text,
                submitted_at_ms,
            } => promote_qa_turn(&mut turns, &mut current, &text, submitted_at_ms),
            CodexRecord::User(u) => {
                if let Some(done) = current.take() {
                    turns.push(done);
                }
                // Boundary recorded even for noise-filtered messages (rollback parity);
                // an indexable turn opens only when it survives the shared gate (image-only
                // inputs arrive as empty user messages and must not open a turn).
                turn_starts.push(turns.len());
                if let UserTextKind::Turn { .. } = u.kind {
                    current = Some(ContextTurn {
                        user: cleanup_user_text(&u.text),
                        submitted_at_ms: u.submitted_at_ms,
                        last_assistant_text: None,
                        entries: Vec::new(),
                    });
                }
            }
            CodexRecord::Assistant { text, .. } => {
                set_last_assistant(&mut current, &text);
                push_entry(&mut current, ContextEntryKind::AssistantText, text);
            }
            CodexRecord::ToolCall(payload) => {
                push_entry(
                    &mut current,
                    ContextEntryKind::ToolCall,
                    compact_json(payload),
                );
            }
            CodexRecord::ToolResult(payload) => {
                push_entry(
                    &mut current,
                    ContextEntryKind::ToolResult,
                    compact_json(payload),
                );
            }
            CodexRecord::Meta { .. }
            | CodexRecord::Title(_)
            | CodexRecord::TurnCompleted { .. }
            | CodexRecord::Other => {}
        }
    }

    if let Some(done) = current {
        turns.push(done);
    }
    Ok(turns)
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
    fn migrated_standalone_assistant_survives_in_context_and_search() {
        let legacy = r#"
{"type":"event_msg","payload":{"type":"user_message","message":"test migration"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"initial answer"}]}}
{"type":"event_msg","payload":{"type":"agent_message","message":"completion notice"}}
"#;
        let migrated = legacy.replace(
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"completion notice"}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"completion notice"}]}}}"#,
        );
        for (name, content) in [
            ("legacy-assistant", legacy),
            ("migrated-assistant", &migrated),
        ] {
            let path = write_temp(name, content);
            let session = crate::parser::codex::parse_file(&path, 0, None).expect("listed");
            let context = crate::session_context::load(&session);
            assert_eq!(session.user_turns.len(), 1);
            assert_eq!(context.turns.len(), 1);
            assert_eq!(
                context.turns[0].last_assistant_text.as_deref(),
                Some("completion notice")
            );
            assert!(session.assistant_blob.contains("completion notice"));
            let rendered = crate::session_context::render::render_reference(&context, false);
            assert!(rendered.contains("completion notice"));
            assert_eq!(context.turns[0].entries.len(), 1);
            assert_eq!(context.turns[0].entries[0].text, "initial answer");
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn migrated_assistant_mirrors_do_not_duplicate_or_replace_later_answers() {
        let user =
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"test mirrors"}}"#;
        let response = r#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"progress report"}]}}"#;
        let mirror = r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"progress report"}]}}}"#;
        let tool = r#"{"type":"response_item","payload":{"type":"function_call","name":"test","arguments":"{}"}}"#;
        let final_answer = r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"final answer"}]}}}"#;
        for records in [
            vec![user, mirror, tool, response, final_answer],
            vec![user, response, tool, final_answer, mirror],
        ] {
            let path = write_temp("assistant-mirrors", &records.join("\n"));
            let turns = parse_turns(&path).unwrap();
            let assistants: Vec<_> = turns[0]
                .entries
                .iter()
                .filter(|entry| entry.kind == ContextEntryKind::AssistantText)
                .map(|entry| entry.text.as_str())
                .collect();
            assert_eq!(assistants, ["progress report", "final answer"]);
            assert_eq!(
                turns[0].last_assistant_text.as_deref(),
                Some("final answer")
            );
            let session = crate::parser::codex::parse_file(&path, 0, None).unwrap();
            assert!(session.assistant_blob.contains("final answer"));
            assert!(!session.assistant_blob.contains("progress report"));
            std::fs::remove_file(path).unwrap();
        }
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

    #[test]
    fn keeps_user_and_promoted_qa_submit_times() {
        let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"first question"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"response_item","payload":{"toolUseResult":{"questions":[{"question":"Continue?"}],"answers":{"Continue?":"Yes"}}}}
"#;
        let path = write_temp("timestamps", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].submitted_at_ms, Some(1_784_768_523_456));
        assert_eq!(turns[1].submitted_at_ms, Some(1_784_772_184_567));
        let _ = std::fs::remove_file(path);
    }
}
