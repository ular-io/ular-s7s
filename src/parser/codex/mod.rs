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

pub(crate) mod events;

use super::{build_assistant_blob, clean_turn, finalize, is_noise_turn, session_updated_at_ms};
use crate::model::{Agent, Session};
use events::{CodexRecord, UserTextKind};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

struct TurnIndex {
    indexable: bool,
    last_assistant: Option<String>,
    last_response_at_ms: Option<i64>,
}

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
    source_mtime_ms: i64,
    meta_map: Option<&HashMap<String, TitleMeta>>,
) -> Option<Session> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut turns: Vec<String> = Vec::new();
    let mut turn_timestamps: Vec<Option<i64>> = Vec::new();
    // Index into `turns` where each user turn starts; boundaries are recorded even for
    // noise-filtered user messages so `thread_rolled_back.num_turns` counts real turns.
    let mut turn_starts: Vec<usize> = Vec::new();
    // Parallel to `turn_starts` (one slot per user-message turn). Noise turns
    // remain non-indexable and cannot move the semantic activity time. A rollback
    // truncates response activity exactly like the associated user turns.
    let mut turn_index: Vec<TurnIndex> = Vec::new();
    let mut title_hint: Option<String> = None;

    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match events::decode(&v) {
            CodexRecord::Meta { id: i, cwd: c } => {
                if id.is_none() {
                    id = i.map(str::to_string);
                }
                if cwd.is_none() {
                    cwd = c.map(str::to_string);
                }
            }
            CodexRecord::Title(t) => title_hint = Some(t.to_string()),
            CodexRecord::RolledBack(n) => {
                // Drop the last `n` user turns (each with its attached QA entries).
                let keep = turn_starts.len().saturating_sub(n);
                let cut = turn_starts.get(keep).copied().unwrap_or(turns.len());
                turns.truncate(cut);
                turn_timestamps.truncate(cut);
                turn_starts.truncate(keep);
                turn_index.truncate(keep);
            }
            CodexRecord::User(u) => {
                // A user line always records a boundary (so a rollback counts real
                // CLI turns) but opens an indexable turn only when it survives the
                // shared noise/clean gate.
                turn_starts.push(turns.len());
                match u.kind {
                    UserTextKind::Turn { cleaned } => {
                        turn_index.push(TurnIndex {
                            indexable: true,
                            last_assistant: None,
                            last_response_at_ms: None,
                        });
                        turns.push(cleaned);
                        turn_timestamps.push(u.submitted_at_ms);
                    }
                    UserTextKind::Boundary => turn_index.push(TurnIndex {
                        indexable: false,
                        last_assistant: None,
                        last_response_at_ms: None,
                    }),
                }
            }
            CodexRecord::Qa {
                text: qa,
                submitted_at_ms,
            } => {
                if !is_noise_turn(&qa) {
                    if let Some(cleaned) = clean_turn(&qa) {
                        turns.push(cleaned);
                        turn_timestamps.push(submitted_at_ms);
                    }
                }
            }
            // Record each turn's last assistant answer for the search index.
            CodexRecord::Assistant {
                text,
                emitted_at_ms,
            } => {
                if let Some(slot) = turn_index.last_mut() {
                    slot.last_assistant = Some(text);
                    if slot.indexable {
                        slot.last_response_at_ms = slot.last_response_at_ms.max(emitted_at_ms);
                    }
                }
            }
            CodexRecord::TurnCompleted { completed_at_ms } => {
                if let Some(slot) = turn_index.last_mut() {
                    if slot.indexable {
                        slot.last_response_at_ms = slot.last_response_at_ms.max(completed_at_ms);
                    }
                }
            }
            CodexRecord::ToolCall(_) | CodexRecord::ToolResult(_) | CodexRecord::Other => {}
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
    let last_response_completed_at_ms = turn_index
        .iter()
        .filter(|slot| slot.indexable)
        .filter_map(|slot| slot.last_response_at_ms)
        .max();
    let updated_at_ms = session_updated_at_ms(
        &turn_timestamps,
        last_response_completed_at_ms,
        source_mtime_ms,
    );

    let mut session = Session {
        agent: Agent::Codex,
        profile_id: String::new(),
        id,
        source_path: Some(path.to_path_buf()),
        cwd: cwd.into(),
        folder,
        updated_at_ms,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: turns,
        user_turn_timestamps_ms: turn_timestamps,
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: meta.and_then(|m| m.title.clone()).or(title_hint),
        title_fixed: meta.and_then(|m| m.title.as_ref()).is_some(),
    };
    finalize(&mut session);
    let assistant_per_turn: Vec<String> = turn_index
        .into_iter()
        .filter(|slot| slot.indexable)
        .filter_map(|slot| slot.last_assistant)
        .collect();
    session.assistant_blob = build_assistant_blob(&assistant_per_turn);
    Some(session)
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
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"event_msg","payload":{"type":"user_message","message":"버려질 질문"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"timestamp":"2026-07-23T03:04:05.678Z","type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
        let (root, path) = write_rollout("rollback-one", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["첫 질문", "수정된 질문"]);
        assert_eq!(
            session.user_turn_timestamps_ms,
            vec![Some(1_784_768_523_456), Some(1_784_775_845_678)]
        );
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
    fn activity_time_uses_task_complete_and_ignores_later_storage_mtime() {
        let content = r#"
{"type":"session_meta","payload":{"id":"activity","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"question"}}
{"timestamp":"2026-07-23T01:03:03.456Z","type":"event_msg","payload":{"type":"agent_message","message":"answer"}}
{"timestamp":"2026-07-23T01:03:04.567Z","type":"event_msg","payload":{"type":"task_complete","completed_at":1784768584}}
{"timestamp":"2026-07-24T09:00:00Z","type":"event_msg","payload":{"type":"thread_settings_applied"}}
"#;
        let (root, path) = write_rollout("activity-complete", content);
        let later_mtime = 1_784_899_999_999;
        let session = parse_file(&path, later_mtime, None).expect("expected session");
        assert_eq!(session.updated_at_ms, 1_784_768_584_567);
        assert_ne!(session.updated_at_ms, later_mtime);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn activity_time_excludes_rolled_back_response_completion() {
        let content = r#"
{"type":"session_meta","payload":{"id":"activity-rollback","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"kept question"}}
{"timestamp":"2026-07-23T01:01:00Z","type":"event_msg","payload":{"type":"task_complete"}}
{"timestamp":"2026-07-23T02:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"abandoned question"}}
{"timestamp":"2026-07-23T05:00:00Z","type":"event_msg","payload":{"type":"task_complete"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"timestamp":"2026-07-23T03:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"replacement question"}}
"#;
        let (root, path) = write_rollout("activity-rollback", content);
        let session = parse_file(&path, 1_784_899_999_999, None).expect("expected session");
        assert_eq!(session.updated_at_ms, 1_784_775_600_000);
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
    fn list_session_keeps_user_and_promoted_qa_submit_times() {
        let content = r#"
{"type":"session_meta","payload":{"id":"times","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"first question"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"response_item","payload":{"toolUseResult":{"questions":[{"question":"Continue?"}],"answers":{"Continue?":"Yes"}}}}
"#;
        let (root, path) = write_rollout("turn-times", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns.len(), 2);
        assert_eq!(
            session.user_turn_timestamps_ms,
            vec![Some(1_784_768_523_456), Some(1_784_772_184_567)]
        );
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
