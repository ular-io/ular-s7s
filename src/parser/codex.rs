//! OpenAI Codex session parser.
//!
//! File path: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`
//! Session ID/cwd are extracted from `session_meta` lines. User turns are identified in `event_msg` where
//! `payload.type=="user_message"` or user messages within `response_item`.
//!
//! Backtrack (esc-esc "edit previous message") handling: the rollout is append-only; editing
//! a past message appends `event_msg` `payload.type=="thread_rolled_back"` with `num_turns` =
//! the number of most recent turns discarded, then the replacement turn follows. Turns dropped
//! by the rollback are removed from the preview. Verified against codex CLI 0.144.4 — the
//! rollback happens in the same rollout file (no fork file is created).

use super::{build_assistant_blob, clean_turn, finalize, is_noise_turn, turn};
use crate::model::{Agent, Session};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// OpenAI Codex session title metadata.
pub struct TitleMeta {
    pub title: Option<String>,
}

/// Loads thread names (session_id -> thread_name) from `~/.codex/session_index.jsonl`.
///
/// Tries both the provided directory and its parent directory, as the caller might
/// pass `~/.codex/sessions` instead of `~/.codex`.
pub fn load_title_meta(cli_dir: &Path) -> HashMap<String, TitleMeta> {
    let mut out = HashMap::new();
    let candidates = [
        cli_dir.join("session_index.jsonl"),
        cli_dir
            .parent()
            .map(|p| p.join("session_index.jsonl"))
            .unwrap_or_default(),
    ];

    for path in candidates.iter().filter(|p| !p.as_os_str().is_empty()) {
        let data = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => continue,
        };

        for line in data.lines() {
            if line.is_empty() {
                continue;
            }
            let v: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let id = match v.get("id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => continue,
            };
            let title = v
                .get("thread_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if title.is_some() || !out.contains_key(&id) {
                out.insert(id, TitleMeta { title });
            }
        }
    }

    out
}

/// Parses a single Codex rollout JSONL file. Returns None if there are no valid user turns.
pub fn parse_file(
    path: &Path,
    mtime_ms: i64,
    meta_map: Option<&HashMap<String, TitleMeta>>,
) -> Option<Session> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut turns: Vec<String> = Vec::new();
    // Index into `turns` where each user turn starts; boundaries are recorded even for
    // noise-filtered user messages so `thread_rolled_back.num_turns` counts real turns.
    let mut turn_starts: Vec<usize> = Vec::new();
    // Parallel to `turn_starts` (one slot per user-message turn): `(indexable, last
    // assistant answer)`. `indexable` is false for noise turns (slash commands, the
    // bootstrap prompt) so their answers are excluded from the search index. A rollback
    // truncates this exactly like `turn_starts`.
    let mut turn_asst: Vec<(bool, Option<String>)> = Vec::new();
    let mut title_hint: Option<String> = None;

    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if let Some(p) = v.get("payload") {
                    if id.is_none() {
                        id = p.get("id").and_then(Value::as_str).map(str::to_string);
                    }
                    if cwd.is_none() {
                        cwd = p.get("cwd").and_then(Value::as_str).map(str::to_string);
                    }
                }
            }
            Some("ai-title") => {
                if let Some(t) = v.get("aiTitle").and_then(Value::as_str) {
                    let t = t.trim();
                    if !t.is_empty() {
                        title_hint = Some(t.to_string());
                    }
                }
            }
            Some("event_msg") | Some("response_item") => {
                if let Some(n) = rolled_back_turns(&v) {
                    // Drop the last `n` user turns (each with its attached QA entries).
                    let keep = turn_starts.len().saturating_sub(n);
                    let cut = turn_starts.get(keep).copied().unwrap_or(turns.len());
                    turns.truncate(cut);
                    turn_starts.truncate(keep);
                    turn_asst.truncate(keep);
                    continue;
                }
                if let Some(text) = turn::extract_user_text(&v) {
                    turn_starts.push(turns.len());
                    let indexable = !is_noise_turn(&text) && clean_turn(&text).is_some();
                    turn_asst.push((indexable, None));
                    if !is_noise_turn(&text) {
                        if let Some(cleaned) = clean_turn(&text) {
                            turns.push(cleaned);
                        }
                    }
                }
                if let Some(qa) = turn::extract_question_answers(&v) {
                    if !is_noise_turn(&qa) {
                        if let Some(cleaned) = clean_turn(&qa) {
                            turns.push(cleaned);
                        }
                    }
                }
                // Record each turn's last assistant answer for the search index.
                if let Some(text) = assistant_text(&v) {
                    if let Some(slot) = turn_asst.last_mut() {
                        slot.1 = Some(text);
                    }
                }
            }
            _ => {}
        }
    }

    if turns.is_empty() {
        return None;
    }

    let id = id.unwrap_or_else(|| extract_uuid_from_name(path));
    let meta = meta_map.and_then(|m| m.get(&id));
    let cwd = cwd.unwrap_or_default();
    let folder = Path::new(&cwd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.clone());

    let mut session = Session {
        agent: Agent::Codex,
        profile_id: String::new(),
        id,
        source_path: Some(path.to_path_buf()),
        cwd: cwd.into(),
        folder,
        mtime_ms,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: turns,
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: meta.and_then(|m| m.title.clone()).or(title_hint),
        title_fixed: meta.and_then(|m| m.title.as_ref()).is_some(),
    };
    finalize(&mut session);
    let assistant_per_turn: Vec<String> = turn_asst
        .into_iter()
        .filter(|(indexable, _)| *indexable)
        .filter_map(|(_, answer)| answer)
        .collect();
    session.assistant_blob = build_assistant_blob(&assistant_per_turn);
    Some(session)
}

/// Extracts assistant answer text from an `event_msg`/`response_item` line, or None
/// when the line is not an assistant message (user turns are handled separately).
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

/// Joins the text parts of a `response_item` assistant message content.
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

/// Extracts UUID from the filename if session_meta is missing.
/// `rollout-2026-06-07T21-18-12-019ea204-eb92-7663-957f-16fcad90e789` -> last 5 dash-separated groups.
fn extract_uuid_from_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        parts[parts.len() - 5..].join("-")
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_title_meta_from_parent_directory() {
        let root = std::env::temp_dir().join(format!(
            "ular-s7s-codex-meta-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let sessions_dir = root.join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("create temp dir");
        std::fs::write(sessions_dir.join("session_index.jsonl"), "{}\n")
            .expect("write empty child index");
        std::fs::write(
            root.join("session_index.jsonl"),
            r#"{"id":"019f36e8-9157-7c63-bee8-8937a6314982","thread_name":"26-07 세션 타이틀 개선"}"#,
        )
        .expect("write session index");

        let meta = load_title_meta(&sessions_dir);
        let title = meta
            .get("019f36e8-9157-7c63-bee8-8937a6314982")
            .and_then(|m| m.title.as_deref());
        assert_eq!(title, Some("26-07 세션 타이틀 개선"));

        let _ = std::fs::remove_file(root.join("session_index.jsonl"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn write_rollout(name: &str, content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ular-s7s-codex-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path =
            root.join("rollout-2026-07-16T00-00-00-019f36e8-9157-7c63-bee8-000000000000.jsonl");
        std::fs::write(&path, content).expect("write rollout");
        (root, path)
    }

    #[test]
    fn thread_rollback_drops_recent_turns() {
        let content = r#"
{"type":"session_meta","payload":{"id":"x1","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"event_msg","payload":{"type":"user_message","message":"버려질 질문"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
        let (root, path) = write_rollout("rollback-one", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["첫 질문", "수정된 질문"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn thread_rollback_can_drop_all_turns() {
        let content = r#"
{"type":"session_meta","payload":{"id":"x2","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"event_msg","payload":{"type":"user_message","message":"둘째 질문"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":2}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
        let (root, path) = write_rollout("rollback-all", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["수정된 질문"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn thread_rollback_drops_question_answers_with_their_turn() {
        // The QA entry belongs to the turn being rolled back and must disappear with it.
        let content = r#"
{"type":"session_meta","payload":{"id":"x3","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"response_item","payload":{"toolUseResult":{"questions":[{"question":"진행할까요?"}],"answers":{"진행할까요?":"네"}}}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
        let (root, path) = write_rollout("rollback-qa", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["수정된 질문"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_file_uses_session_id_from_payload_for_title_lookup() {
        let root = std::env::temp_dir().join(format!(
            "ular-s7s-codex-parse-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path =
            root.join("rollout-2026-07-06T19-10-39-019f36e8-9157-7c63-bee8-8937a6314982.jsonl");
        std::fs::write(
            &path,
            r#"
{"type":"session_meta","payload":{"id":"019f36e8-9157-7c63-bee8-8937a6314982","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
"#,
        )
        .expect("write rollout");
        let mut meta_map = HashMap::new();
        meta_map.insert(
            "019f36e8-9157-7c63-bee8-8937a6314982".to_string(),
            TitleMeta {
                title: Some("26-07 세션 타이틀 개선".to_string()),
            },
        );

        let session = parse_file(&path, 0, Some(&meta_map)).expect("expected session");
        assert_eq!(session.title(), "26-07 세션 타이틀 개선");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn assistant_blob_indexes_answers_and_excludes_rollback() {
        let content = r#"
{"type":"session_meta","payload":{"id":"x1","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"질문1"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"답1 keepkw"}}
{"type":"event_msg","payload":{"type":"user_message","message":"버려질 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"버려질답 dropkw"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"수정답 fixkw"}}
"#;
        let (root, path) = write_rollout("asst-rollback", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert!(session.assistant_blob.contains("keepkw"));
        assert!(session.assistant_blob.contains("fixkw"));
        assert!(!session.assistant_blob.contains("dropkw"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn assistant_blob_keeps_single_copy_of_duplicated_answer_event() {
        // The same answer arrives twice (agent_message then response_item); only the
        // turn's last assistant text is indexed, so the keyword appears once.
        let content = r#"
{"type":"session_meta","payload":{"id":"x2","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"동일답변 dupkw"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"동일답변 dupkw"}]}}
"#;
        let (root, path) = write_rollout("asst-dup", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.assistant_blob.matches("dupkw").count(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn assistant_blob_excludes_bootstrap_ready_response() {
        let content = r#"
{"type":"session_meta","payload":{"id":"x3","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"<s7s-context-bootstrap>\nRun\n</s7s-context-bootstrap>"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"readykw 확인"}}
{"type":"event_msg","payload":{"type":"user_message","message":"진짜 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"realkw 답"}}
"#;
        let (root, path) = write_rollout("asst-bootstrap", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["진짜 질문"]);
        assert!(session.assistant_blob.contains("realkw"));
        assert!(!session.assistant_blob.contains("readykw"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
