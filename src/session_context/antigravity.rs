//! Antigravity detailed turn parser (session context).
//!
//! Conversation bodies live in protobuf payloads inside SQLite, but a readable
//! JSONL transcript is also written under `brain/<id>/.system_generated/logs/`.
//! The detailed parser reads that transcript. `/rewind` rewrites the DB (and
//! transcript source of truth) destructively, so no dead-branch filtering is
//! needed here — the store only ever holds the active path.

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
    let content = std::fs::read_to_string(path)?;
    let mut turns: Vec<ContextTurn> = Vec::new();
    let mut current: Option<ContextTurn> = None;
    // Question list from the preceding ask_question tool_call (for pairing with ASK_QUESTION answers).
    let mut pending_questions: Vec<String> = Vec::new();

    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
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
                        last_assistant_text: None,
                        entries: Vec::new(),
                    });
                }
            }
            ("MODEL", "PLANNER_RESPONSE") => {
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
                    Some(qa) => promote_qa_turn(&mut turns, &mut current, &qa),
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
    Ok(turns)
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
