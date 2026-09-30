//! Antigravity detailed turn parser (session context).
//!
//! Conversation bodies live in protobuf payloads inside SQLite, but a readable
//! JSONL transcript is also written under `brain/<id>/.system_generated/logs/`.
//! `/rewind` truncates the DB, but the transcript can retain the old suffix and
//! append the replacement branch with reused step indices. Reduce that suffix
//! before extracting turns, work entries, and response-completion timestamps.

use super::model::{ContextEntryKind, ContextTurn};
use super::{cleanup_user_text, compact_json, promote_qa_turn, push_entry, set_last_assistant};
use crate::parser::is_noise_turn;
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Path to the JSONL transcript of an Antigravity conversation.
/// Stored under `brain/<id>/.system_generated/logs/` alongside `conversations/<id>.db`.
/// Prefers `transcript_full.jsonl` (full), falling back to `transcript.jsonl`
/// (recent rotating file) if missing.
pub fn transcript_path(db_path: &Path, id: &str) -> Option<PathBuf> {
    let cli_dir = db_path.parent()?.parent()?;
    let logs = cli_dir
        .join("brain")
        .join(id)
        .join(".system_generated/logs");
    let full = logs.join("transcript_full.jsonl");
    if full.is_file() {
        return Some(full);
    }
    let tail = logs.join("transcript.jsonl");
    tail.is_file().then_some(tail)
}

/// Parses the Antigravity transcript (JSONL).
/// Entry structure: `source` (USER_EXPLICIT/MODEL/SYSTEM) + `type` + `content`.
/// - USER_EXPLICIT/USER_INPUT: `<USER_REQUEST>` body = user turn starts.
/// - MODEL/PLANNER_RESPONSE: assistant text (the last one is the turn's last
///   assistant text) + `tool_calls`.
/// - MODEL/ASK_QUESTION: answered questions are promoted to virtual user turns
///   (matching the session list database parser). Skipped questions remain as tool records.
/// - MODEL/Others (RUN_COMMAND, VIEW_FILE, MCP_TOOL, etc.): tool execution results.
/// - SYSTEM/ERROR_MESSAGE: errors are kept as work records. Other SYSTEM sources are ignored.
pub fn parse_turns(path: &Path) -> Result<Vec<ContextTurn>> {
    parse_turns_with_activity(path).map(|(turns, _)| turns)
}

/// Parses turns together with the latest completed assistant-response timestamp.
/// The transcript has no dedicated turn-complete event, so the last DONE
/// `PLANNER_RESPONSE.created_at` attached to a real user turn is the best
/// available completion timestamp.
pub(crate) fn parse_turns_with_activity(path: &Path) -> Result<(Vec<ContextTurn>, Option<i64>)> {
    parse_transcript(path, None)
}

/// The DB is truncated immediately on rewind, before a replacement user record
/// reaches the transcript. Its last step also bounds the reduced transcript.
/// A missing/unreadable DB leaves standalone transcript parsing available.
pub(crate) fn parse_turns_with_activity_for_db(
    path: &Path,
    db_path: &Path,
) -> Result<(Vec<ContextTurn>, Option<i64>)> {
    let last_step =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()
            .and_then(|conn| {
                conn.query_row("SELECT MAX(idx) FROM steps", [], |row| {
                    row.get::<_, Option<i64>>(0)
                })
                .ok()
            })
            .map(|index| index.unwrap_or(-1));
    parse_transcript(path, last_step)
}

fn parse_transcript(
    path: &Path,
    last_step: Option<i64>,
) -> Result<(Vec<ContextTurn>, Option<i64>)> {
    let content = std::fs::read_to_string(path)?;
    let mut turns: Vec<ContextTurn> = Vec::new();
    let mut current: Option<ContextTurn> = None;
    let mut last_response_completed_at_ms: Option<i64> = None;
    // Question list from the preceding ask_question tool_call (for pairing with ASK_QUESTION answers).
    let mut pending_questions: Vec<String> = Vec::new();

    for line in active_transcript_lines(&content) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let (Some(limit), Some(index)) = (last_step, v.get("step_index").and_then(Value::as_i64))
        {
            if index > limit {
                continue;
            }
        }
        let source = v.get("source").and_then(Value::as_str).unwrap_or("");
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let text = v.get("content").and_then(Value::as_str).unwrap_or("");

        match (source, ty) {
            ("USER_EXPLICIT", "USER_INPUT") => {
                if let Some(done) = current.take() {
                    turns.push(done);
                }
                let user = extract_user_request(text);
                if !user.trim().is_empty() && !is_noise_turn(&user) {
                    current = Some(ContextTurn {
                        user: cleanup_user_text(&user),
                        submitted_at_ms: None,
                        last_assistant_text: None,
                        entries: Vec::new(),
                    });
                }
            }
            ("MODEL", "PLANNER_RESPONSE") => {
                let completed = matches!(
                    v.get("status").and_then(Value::as_str),
                    None | Some("") | Some("DONE")
                );
                if current.is_some() && completed {
                    last_response_completed_at_ms =
                        last_response_completed_at_ms.max(created_at_ms(&v));
                }
                if !text.trim().is_empty() {
                    set_last_assistant(&mut current, text);
                    push_entry(
                        &mut current,
                        ContextEntryKind::AssistantText,
                        text.to_string(),
                    );
                }
                match v.get("tool_calls") {
                    Some(Value::Array(calls)) => {
                        for call in calls {
                            if call.get("name").and_then(Value::as_str) == Some("ask_question") {
                                pending_questions = ask_question_texts(call);
                            }
                            push_entry(
                                &mut current,
                                ContextEntryKind::ToolCall,
                                compact_json(call),
                            );
                        }
                    }
                    Some(other) if !other.is_null() => {
                        push_entry(
                            &mut current,
                            ContextEntryKind::ToolCall,
                            compact_json(other),
                        );
                    }
                    _ => {}
                }
            }
            ("MODEL", "ASK_QUESTION") => {
                match antigravity_ask_answers(text, &pending_questions) {
                    Some(qa) => promote_qa_turn(&mut turns, &mut current, &qa, None),
                    // No valid answer (e.g. skipped) -> keep as work record only.
                    None if !text.trim().is_empty() => {
                        push_entry(
                            &mut current,
                            ContextEntryKind::ToolResult,
                            format!("[{ty}]\n{text}"),
                        );
                    }
                    None => {}
                }
                pending_questions.clear();
            }
            ("MODEL", _) if !text.trim().is_empty() => {
                push_entry(
                    &mut current,
                    ContextEntryKind::ToolResult,
                    format!("[{ty}]\n{text}"),
                );
            }
            ("SYSTEM", "ERROR_MESSAGE") if !text.trim().is_empty() => {
                push_entry(
                    &mut current,
                    ContextEntryKind::ToolResult,
                    format!("[ERROR]\n{text}"),
                );
            }
            _ => {}
        }
    }

    if let Some(done) = current {
        turns.push(done);
    }
    Ok((turns, last_response_completed_at_ms))
}

/// A replayed user step replaces the previous suffix. Do not treat arbitrary
/// index decreases as rewinds: normal planner responses and checkpoints can
/// arrive after records with higher indices. Unindexed transcripts retain their
/// original order, and truncating before parsing also discards pending QA state.
fn active_transcript_lines(content: &str) -> Vec<&str> {
    let mut active: Vec<(Option<u64>, &str)> = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let index = record.get("step_index").and_then(Value::as_u64);
        if record.get("source").and_then(Value::as_str) == Some("USER_EXPLICIT")
            && record.get("type").and_then(Value::as_str) == Some("USER_INPUT")
        {
            if let Some(index) = index {
                if let Some(start) = active
                    .iter()
                    .position(|(previous, _)| previous.is_some_and(|previous| previous >= index))
                {
                    active.truncate(start);
                }
            }
        }
        active.push((index, line));
    }
    active.into_iter().map(|(_, line)| line).collect()
}

fn created_at_ms(v: &Value) -> Option<i64> {
    v.get("created_at")
        .and_then(Value::as_str)
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
        .map(|timestamp| timestamp.timestamp_millis())
}

/// Extracts the list of question texts from the args of an ask_question tool_call.
/// Handles cases where args are recorded as a JSON string.
fn ask_question_texts(call: &Value) -> Vec<String> {
    let args = call.get("args").unwrap_or(call);
    // Prepare for cases where args are recorded as a JSON string.
    let parsed;
    let args = match args {
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v) => {
                parsed = v;
                &parsed
            }
            Err(_) => return Vec::new(),
        },
        other => other,
    };
    args.get("questions")
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|q| q.get("question").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Pairs ASK_QUESTION content (`A<n>: Answer` lines) with the question list
/// and normalizes them to `· Question -> Answer`. Excludes skips ("User Skipped"),
/// returning None if there are no valid answers.
fn antigravity_ask_answers(content: &str, questions: &[String]) -> Option<String> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('A') else {
            continue;
        };
        let Some(colon) = rest.find(':') else {
            continue;
        };
        let (num, answer) = rest.split_at(colon);
        if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let answer = answer[1..].trim();
        if answer.is_empty() || answer == "User Skipped" {
            continue;
        }
        let question = num
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|idx| questions.get(idx))
            .map(String::as_str)
            .unwrap_or("?");
        lines.push(format!("· {} → {}", question, answer));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Extracts only the `<USER_REQUEST>` body from USER_INPUT content.
/// (Removes system metadata like `<ADDITIONAL_METADATA>`. If no tags are found, keeps raw text.)
fn extract_user_request(content: &str) -> String {
    const OPEN: &str = "<USER_REQUEST>";
    const CLOSE: &str = "</USER_REQUEST>";
    if let Some(start) = content.find(OPEN) {
        let rest = &content[start + OPEN.len()..];
        let end = rest.find(CLOSE).unwrap_or(rest.len());
        return rest[..end].trim().to_string();
    }
    content.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fixture(content: &str) -> (Vec<ContextTurn>, Option<i64>) {
        let path = std::env::temp_dir().join(format!(
            "s7s-antigravity-context-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, content).expect("write transcript");
        let parsed = parse_turns_with_activity(&path).expect("parse transcript");
        std::fs::remove_file(path).expect("remove fixture");
        parsed
    }

    #[test]
    fn rewind_replaces_the_suffix_including_answers_tools_qa_and_activity() {
        let (turns, activity) = parse_fixture(
            r#"
{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"first"}
{"step_index":2,"source":"MODEL","type":"RUN_COMMAND","content":"kept tool"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T01:00:00Z","content":"kept answer"}
{"step_index":10,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"old second"}
{"step_index":11,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T05:00:00Z","content":"abandoned answer","tool_calls":[{"name":"ask_question","args":{"questions":[{"question":"abandoned question"}]}}]}
{"step_index":12,"source":"MODEL","type":"ASK_QUESTION","content":"A1: Yes"}
{"step_index":13,"source":"MODEL","type":"RUN_COMMAND","content":"abandoned tool"}
{"step_index":20,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"old third"}
{"step_index":21,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T06:00:00Z","content":"abandoned third answer"}
{"step_index":10,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"new second"}
{"step_index":11,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T02:00:00Z","content":"new answer"}
"#,
        );
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user, "first");
        assert_eq!(turns[0].last_assistant_text.as_deref(), Some("kept answer"));
        assert!(turns[0]
            .entries
            .iter()
            .any(|e| e.text.contains("kept tool")));
        assert_eq!(turns[1].user, "new second");
        assert_eq!(turns[1].last_assistant_text.as_deref(), Some("new answer"));
        assert!(turns
            .iter()
            .flat_map(|t| &t.entries)
            .all(|e| !e.text.contains("abandoned")));
        assert_eq!(activity, Some(1_784_772_000_000));
    }

    #[test]
    fn repeated_rewinds_can_replace_work_with_a_user_or_noise_boundary() {
        let (turns, activity) = parse_fixture(
            r#"
{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"first"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T01:00:00Z","content":"kept answer"}
{"step_index":2,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T03:00:00Z","content":"old continuation"}
{"step_index":4,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"old second"}
{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"replacement second"}
{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","tool_calls":[{"name":"ask_question","args":{"questions":[{"question":"discarded question"}]}}]}
{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"/usage"}
{"step_index":3,"source":"MODEL","type":"ASK_QUESTION","content":"A1: User Skipped"}
"#,
        );
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].last_assistant_text.as_deref(), Some("kept answer"));
        assert_eq!(activity, Some(1_784_768_400_000));
    }

    #[test]
    fn unindexed_transcripts_keep_repeated_prompts_and_skip_malformed_lines() {
        let (turns, _) = parse_fixture(
            r#"
{"source":"USER_EXPLICIT","type":"USER_INPUT","content":"same prompt"}
not json
{"source":"MODEL","type":"PLANNER_RESPONSE","content":"first answer"}
{"source":"USER_EXPLICIT","type":"USER_INPUT","content":"same prompt"}
{"source":"MODEL","type":"PLANNER_RESPONSE","content":"second answer"}
"#,
        );
        assert_eq!(turns.len(), 2);
        assert_eq!(
            turns[0].last_assistant_text.as_deref(),
            Some("first answer")
        );
        assert_eq!(
            turns[1].last_assistant_text.as_deref(),
            Some("second answer")
        );
    }

    #[test]
    fn activity_uses_latest_done_planner_response_attached_to_a_user_turn() {
        let (turns, completed_at_ms) = parse_fixture(
            r#"
{"step_index":0,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-07-23T00:00:00Z","content":"orphan"}
{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-07-23T01:00:00Z","content":"<USER_REQUEST>question</USER_REQUEST>"}
{"step_index":2,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-07-23T01:01:02Z","content":"answer"}
{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","status":"RUNNING","created_at":"2026-07-23T01:02:00Z","content":"partial"}
"#,
        );
        assert_eq!(turns.len(), 1);
        assert_eq!(completed_at_ms, Some(1_784_768_462_000));
    }
}
