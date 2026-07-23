//! Claude Code detailed turn parser (session context).
//!
//! Same source file as the list parser (`~/.claude/projects/<dir>/<id>.jsonl`),
//! but extracts full turns (user text, tool calls/results, assistant text).
//!
//! Record decoding and the active `parentUuid` chain are shared with the list
//! parser (`parser::claude::events` — R12): turn acceptance gates, sidechain and
//! task-notification identity, and `/rewind` filtering can no longer drift
//! between the two views. Only the detailed payloads (tool calls/results) are
//! extracted here, from the raw records the shared decoder deliberately does
//! not materialize.

use super::model::{ContextEntryKind, ContextTurn};
use super::{cleanup_user_text, compact_json, promote_qa_turn, push_entry, set_last_assistant};
use crate::parser::claude::events::{self, AssistantItem, RecordKind, UserTextKind};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

pub fn parse_turns(path: &Path) -> Result<Vec<ContextTurn>> {
    let content = std::fs::read_to_string(path)?;
    let values = events::parse_lines(&content);

    // Active-path membership from the shared reducer (rewind parity with the list).
    let filter = events::chain_filter(&values);

    let mut turns: Vec<ContextTurn> = Vec::new();
    let mut current: Option<ContextTurn> = None;

    for v in &values {
        let Some(record) = events::decode(v) else {
            continue; // Sidechain (subagent) records.
        };
        if !filter.is_active(record.uuid) {
            continue; // Abandoned by /rewind.
        }

        match record.kind {
            RecordKind::User(u) => {
                // AskUserQuestion response is promoted to a virtual user turn, matching
                // the list view (not recorded redundantly as a ToolResult).
                if let Some(qa) = u.qa {
                    promote_qa_turn(&mut turns, &mut current, &qa, u.submitted_at_ms);
                    continue;
                }
                // Background task notifications are injected as user-role entries but
                // are not human questions: keep the current turn open so the follow-up
                // assistant answer attaches to the original question, and record the
                // notification body as a tool result.
                if u.is_task_notification {
                    if let Some(text) = u.text {
                        push_entry(&mut current, ContextEntryKind::ToolResult, text);
                    }
                    continue;
                }
                match u.text_kind {
                    // Turn acceptance mirrors the list parser exactly (shared gates in
                    // the decoder). The context view keeps the raw text (IDE-block
                    // compression + redaction), not the list's NFC-trimmed payload.
                    UserTextKind::Turn { .. } => {
                        if let Some(done) = current.take() {
                            turns.push(done);
                        }
                        current = Some(ContextTurn {
                            user: cleanup_user_text(&u.text.unwrap_or_default()),
                            submitted_at_ms: u.submitted_at_ms,
                            last_assistant_text: None,
                            entries: Vec::new(),
                        });
                    }
                    // Any extractable user text closes the current turn, even when it
                    // opens none (noise input, blank text) — except CLI-injected meta
                    // records (isMeta: a skill's SKILL.md body loaded mid-turn). Those
                    // are not user boundaries: closing here would orphan the rest of the
                    // turn (subsequent tool work and the final answer) as the current
                    // turn goes empty. Turn count is unaffected (a boundary never opens
                    // a turn), so list/detail parity is preserved.
                    UserTextKind::Boundary | UserTextKind::Blank => {
                        if !u.is_meta {
                            if let Some(done) = current.take() {
                                turns.push(done);
                            }
                        }
                    }
                    UserTextKind::NoText => {
                        if let Some(entry) = claude_tool_result(v) {
                            push_entry(&mut current, ContextEntryKind::ToolResult, entry);
                        }
                    }
                }
            }
            RecordKind::Assistant { items, .. } => {
                for item in items {
                    match item {
                        AssistantItem::Text(text) => {
                            set_last_assistant(&mut current, text);
                            push_entry(
                                &mut current,
                                ContextEntryKind::AssistantText,
                                text.to_string(),
                            );
                        }
                        AssistantItem::ToolUse(item) => {
                            push_entry(
                                &mut current,
                                ContextEntryKind::ToolCall,
                                compact_json(item),
                            );
                        }
                    }
                }
            }
            RecordKind::Title(_) | RecordKind::TurnCompleted { .. } | RecordKind::Other => {}
        }
    }

    if let Some(done) = current {
        turns.push(done);
    }
    Ok(turns)
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

    #[test]
    fn ismeta_skill_injection_does_not_truncate_the_turn() {
        // Regression: a skill's SKILL.md body is injected mid-turn as an
        // `isMeta: true` user record. `is_noise_turn` classifies it as a
        // boundary, which used to close the in-progress turn and orphan every
        // later entry — including the final answer. It must be transparent.
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"origin":{"kind":"human"},"message":{"role":"user","content":"문서 정리 후 commit push"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"content":[{"type":"tool_use","id":"t1","name":"Skill","input":{"skill":"tde"}}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}
{"type":"user","uuid":"d","parentUuid":"c","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: /home/x/skills/tde\n\n# TDE\n스킬 문서 본문"}]}}
{"type":"assistant","uuid":"e","parentUuid":"d","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"git commit"}}]}}
{"type":"user","uuid":"f","parentUuid":"e","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"[main c1cc872] done"}]}}
{"type":"assistant","uuid":"g","parentUuid":"f","message":{"content":[{"type":"text","text":"완료했습니다. 커밋·푸시까지 끝냈습니다."}]}}
"#;
        let path = write_temp("ismeta-skill", content);
        let turns = parse_turns(&path).expect("parse");

        // One turn only: the meta injection must not open or close a turn.
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user, "문서 정리 후 commit push");
        // The final answer after the injection is recovered, not orphaned.
        assert_eq!(
            turns[0].last_assistant_text.as_deref(),
            Some("완료했습니다. 커밋·푸시까지 끝냈습니다.")
        );
        // The post-injection git work stays attached to the same turn.
        assert!(turns[0]
            .entries
            .iter()
            .any(|e| e.kind == ContextEntryKind::ToolCall && e.text.contains("git commit")));
        assert!(turns[0]
            .entries
            .iter()
            .any(|e| e.kind == ContextEntryKind::ToolResult && e.text.contains("c1cc872")));
        // The skill-doc body itself is not surfaced as work.
        assert!(turns[0]
            .entries
            .iter()
            .all(|e| !e.text.contains("Base directory for this skill")));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn keeps_user_and_promoted_qa_submit_times() {
        let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","origin":{"kind":"human"},"message":{"role":"user","content":"first question"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","toolUseResult":{"questions":[{"question":"Continue?"}],"answers":{"Continue?":"Yes"}}}
"#;
        let path = write_temp("timestamps", content);
        let turns = parse_turns(&path).expect("parse");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].submitted_at_ms, Some(1_784_768_523_456));
        assert_eq!(turns[1].submitted_at_ms, Some(1_784_772_184_567));
        let _ = std::fs::remove_file(path);
    }
}
