//! Claude Code session parser.
//!
//! File path: `~/.claude/projects/<encoded-cwd>/<sessionId>.jsonl`
//! One file represents one session. User turns are identified where `type=="user"` && `message.role=="user"`,
//! extracting text that is not tool results, sidechain messages, or command injections.
//!
//! `/rewind` handling: the file is append-only and every entry links to its predecessor via
//! `parentUuid`. Rewinding then continuing appends the next user message with a `parentUuid`
//! pointing at a node before the rewind point, leaving the abandoned turns in the file as a
//! dead branch. Only turns on the active path (walked from the last non-sidechain entry back
//! to the root) are shown. Verified against claude CLI 2.1.211 — a rewind alone writes
//! nothing, so it is only detectable once the user sends a message afterwards.
pub(crate) mod events;

use super::{build_assistant_blob, clean_turn, finalize, is_noise_turn, session_updated_at_ms};
use crate::model::{Agent, Session};
use events::{AssistantItem, RecordKind, TitleEvent, UserTextKind};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Ordered conversation event used to reconstruct turn boundaries for both the user
/// turn list and the per-turn last assistant answer (search index). Built from
/// active-branch records only (the `/rewind` filter is applied while decoding),
/// so no per-event uuid is needed.
enum Event {
    /// A real user turn: opens a new turn and contributes to `user_turns`.
    User(String, Option<i64>),
    /// A countable-but-noise user line (slash command, bootstrap prompt): closes the
    /// current turn without opening one, so a following answer is not indexed under it.
    Boundary,
    /// One assistant text block; the last one within a turn becomes that turn's answer.
    Assistant(String),
}

/// Claude session title metadata.
pub struct TitleMeta {
    pub title: Option<String>,
    pub fixed: bool,
}

/// Loads title metadata (sessionId -> TitleMeta) from `~/.claude/sessions/*.json`.
pub fn load_title_meta(cli_dir: &Path) -> HashMap<String, TitleMeta> {
    let mut out = HashMap::new();
    let dir = cli_dir.join("sessions");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => return out,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let data = match std::fs::read_to_string(&path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = match v.get("sessionId").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let title = v
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // Only explicit sources mark the title as fixed: every supported rename
        // path writes "custom" (real meta files carry "custom"/"derived"/absent).
        // Treating a missing nameSource as fixed would let degenerate CLI auto
        // names (e.g. the session id used as the name) bypass the bad-auto-title
        // fallback in title::resolve.
        let fixed = matches!(
            v.get("nameSource").and_then(Value::as_str),
            Some("custom") | Some("user")
        );
        out.insert(id, TitleMeta { title, fixed });
    }

    out
}

/// Parses a single Claude JSONL file. Returns None if there are no valid user turns.
pub fn parse_file(path: &Path, source_mtime_ms: i64, meta: Option<&TitleMeta>) -> Option<Session> {
    let content = std::fs::read_to_string(path).ok()?;
    let id = path.file_stem()?.to_string_lossy().to_string();

    let values = events::parse_lines(&content);

    // Use the first occurrence of cwd from any line (sidechain and title records
    // included — the field is per-line metadata, not conversation content).
    let cwd = values
        .iter()
        .find_map(|v| {
            v.get("cwd")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
        })
        .map(str::to_string);

    // Pass 1: active-branch membership (see module docs on `/rewind`); a broken
    // chain keeps everything — a format change must never blank a session.
    let filter = events::chain_filter(&values);

    // Pass 2: decode active-branch records into ordered events (user/boundary/
    // assistant) and resolve the body title events.
    let mut events: Vec<Event> = Vec::new();
    let mut explicit_title: Option<String> = None;
    let mut auto_title: Option<String> = None;
    let mut last_response_completed_at_ms: Option<i64> = None;
    let mut has_open_user_turn = false;

    for v in &values {
        let Some(record) = events::decode(v) else {
            continue;
        };
        if !filter.is_active(record.uuid) {
            continue; // Abandoned by /rewind.
        }
        match record.kind {
            RecordKind::Title(TitleEvent::Custom(t)) => {
                // Explicit titles set via `/rename` always take precedence over auto-generated titles.
                explicit_title = Some(t.to_string());
            }
            RecordKind::Title(TitleEvent::AgentName(t)) => {
                if explicit_title.is_none() {
                    explicit_title = Some(t.to_string());
                }
            }
            RecordKind::Title(TitleEvent::Ai(t)) => {
                if auto_title.is_none() && explicit_title.is_none() {
                    auto_title = Some(t.to_string());
                }
            }
            RecordKind::User(u) => {
                if !u.is_task_notification {
                    match u.text_kind {
                        UserTextKind::Turn { cleaned } => {
                            events.push(Event::User(cleaned, u.submitted_at_ms));
                            has_open_user_turn = true;
                        }
                        // Slash commands, the bootstrap prompt, etc. close the current
                        // turn but do not open one.
                        UserTextKind::Boundary => {
                            events.push(Event::Boundary);
                            has_open_user_turn = false;
                        }
                        UserTextKind::Blank | UserTextKind::NoText => {}
                    }
                }
                // Task notifications produce no event (the real final answer
                // follows, so they keep the turn open); Q&A answers still count.
                if let Some(qa) = u.qa {
                    if !is_noise_turn(&qa) {
                        if let Some(cleaned) = clean_turn(&qa) {
                            events.push(Event::User(cleaned, u.submitted_at_ms));
                            has_open_user_turn = true;
                        }
                    }
                }
            }
            RecordKind::Assistant {
                items,
                emitted_at_ms,
            } => {
                for item in items {
                    if let AssistantItem::Text(t) = item {
                        if !t.trim().is_empty() {
                            events.push(Event::Assistant(t.to_string()));
                            if has_open_user_turn {
                                last_response_completed_at_ms =
                                    last_response_completed_at_ms.max(emitted_at_ms);
                            }
                        }
                    }
                }
            }
            RecordKind::TurnCompleted { completed_at_ms } => {
                if has_open_user_turn {
                    last_response_completed_at_ms =
                        last_response_completed_at_ms.max(completed_at_ms);
                    has_open_user_turn = false;
                }
            }
            RecordKind::Other => {}
        }
    }

    let turns: Vec<String> = events
        .iter()
        .filter_map(|ev| match ev {
            Event::User(text, _) => Some(text.clone()),
            _ => None,
        })
        .collect();
    let turn_timestamps: Vec<Option<i64>> = events
        .iter()
        .filter_map(|event| match event {
            Event::User(_, timestamp) => Some(*timestamp),
            _ => None,
        })
        .collect();
    if turns.is_empty() {
        return None;
    }
    let assistant_per_turn = last_assistant_per_turn(&events);
    let updated_at_ms = session_updated_at_ms(
        &turn_timestamps,
        last_response_completed_at_ms,
        source_mtime_ms,
    );

    let cwd = cwd.unwrap_or_else(|| decode_project_dir(path));
    let folder = folder_name(&cwd);
    let title_fixed = explicit_title.is_some() || meta.map(|m| m.fixed).unwrap_or(false);
    let title_hint = explicit_title
        .clone()
        .or(auto_title)
        .or_else(|| meta.and_then(|m| m.title.clone()));

    let mut session = Session {
        agent: Agent::Claude,
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
        title_hint,
        title_fixed,
    };
    finalize(&mut session);
    session.assistant_blob = build_assistant_blob(&assistant_per_turn);
    Some(session)
}

/// Walks the active events in order and returns each turn's last assistant text.
/// A turn opens on a `User` event and closes on the next `User`/`Boundary`; only the
/// last assistant text seen while a turn is open is kept (intermediate progress
/// messages are dropped). Assistant text with no open turn (e.g. the bootstrap ready
/// response before the first real question) is ignored.
fn last_assistant_per_turn(events: &[Event]) -> Vec<String> {
    let mut per_turn = Vec::new();
    let mut current: Option<String> = None;
    let mut in_turn = false;
    for ev in events {
        match ev {
            Event::User(_, _) => {
                if in_turn {
                    if let Some(a) = current.take() {
                        per_turn.push(a);
                    }
                }
                in_turn = true;
                current = None;
            }
            Event::Boundary => {
                if in_turn {
                    if let Some(a) = current.take() {
                        per_turn.push(a);
                    }
                }
                in_turn = false;
                current = None;
            }
            Event::Assistant(text) => {
                if in_turn {
                    current = Some(text.clone());
                }
            }
        }
    }
    if in_turn {
        if let Some(a) = current {
            per_turn.push(a);
        }
    }
    per_turn
}

/// Reconstructs (approximates) the path from the dash-encoded directory name when the cwd field is missing.
/// Example: `-Users-username-DevSpace-my-project` -> `/Users/username/DevSpace/my-project`
/// (Approximated since dashes cannot distinguish between slashes and hyphens, used solely for folder display).
fn decode_project_dir(path: &Path) -> String {
    let dir = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if dir.starts_with('-') {
        dir.replacen('-', "/", 1).replace('-', "/")
    } else {
        dir
    }
}

/// Extracts the last path component as the folder name.
fn folder_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::turn;

    #[test]
    fn title_meta_fixed_only_for_explicit_name_sources() {
        let root =
            std::env::temp_dir().join(format!("s7s-claude-meta-fixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("sessions");
        std::fs::create_dir_all(&dir).expect("create sessions dir");
        for (file, body) in [
            (
                "a.json",
                r#"{"sessionId":"id-custom","name":"제목","nameSource":"custom"}"#,
            ),
            (
                "b.json",
                r#"{"sessionId":"id-user","name":"제목","nameSource":"user"}"#,
            ),
            (
                "c.json",
                r#"{"sessionId":"id-derived","name":"ular-s7s-2b","nameSource":"derived"}"#,
            ),
            ("d.json", r#"{"sessionId":"id-none","name":"4a72d37c"}"#),
        ] {
            std::fs::write(dir.join(file), body).expect("write meta");
        }

        let meta = load_title_meta(&root);
        assert!(meta.get("id-custom").expect("custom").fixed);
        assert!(meta.get("id-user").expect("user").fixed);
        assert!(!meta.get("id-derived").expect("derived").fixed);
        // Missing nameSource must not be fixed: degenerate CLI auto names
        // (session id as name) would otherwise bypass the bad-auto-title fallback.
        assert!(!meta.get("id-none").expect("none").fixed);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extracts_question_answers_from_tool_use_result() {
        let v: Value = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_123",
                    "content": "Your questions have been answered..."
                }]
            },
            "toolUseResult": {
                "questions": [{
                    "question": "헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요?",
                    "header": "질문",
                    "multiSelect": false
                }],
                "answers": {
                    "헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요?": "적용"
                }
            }
        });

        let out = turn::extract_question_answers(&v).expect("expected question/answer turn");
        assert_eq!(
            out,
            "· 헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요? → 적용"
        );
    }

    #[test]
    fn extracts_plain_user_text_without_tool_result_noise() {
        let v: Value = serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": "안녕하세요"
            }
        });

        let out = turn::extract_user_text(&v).expect("expected plain user text");
        assert_eq!(out, "안녕하세요");
    }

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ular-s7s-{}-{}.jsonl",
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
    fn rewind_hides_turns_on_abandoned_branch() {
        // Q2 was abandoned by /rewind: Q3 branches from the same parent (b) as Q2.
        let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"question 1"}}
{"timestamp":"2026-07-23T01:03:03.456Z","type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"answer 1"}]}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"abandoned question 2"}}
{"timestamp":"2026-07-23T05:00:00Z","type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"abandoned answer 2"}]}}
{"timestamp":"2026-07-23T05:01:00Z","type":"system","subtype":"turn_duration","uuid":"x","parentUuid":"d"}
{"timestamp":"2026-07-23T03:04:05.678Z","type":"user","uuid":"e","parentUuid":"b","message":{"role":"user","content":"replacement question 3"}}
{"timestamp":"2026-07-23T03:05:05.678Z","type":"assistant","uuid":"f","parentUuid":"e","message":{"role":"assistant","content":[{"type":"text","text":"answer 3"}]}}
"#;
        let path = write_temp("rewind-branch", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(
            session.user_turns,
            vec!["question 1", "replacement question 3"]
        );
        assert_eq!(
            session.user_turn_timestamps_ms,
            vec![Some(1_784_768_523_456), Some(1_784_775_845_678)]
        );
        assert_eq!(session.updated_at_ms, 1_784_775_905_678);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn linear_session_keeps_all_turns() {
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"질문2"}}
"#;
        let path = write_temp("linear", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["질문1", "질문2"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn activity_time_ignores_resume_only_last_prompt_write() {
        let content = r#"
{"timestamp":"2026-07-22T08:58:46.672Z","type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"question"}}
{"timestamp":"2026-07-22T08:59:43.293Z","type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"answer"}]}}
{"timestamp":"2026-07-22T08:59:43.525Z","type":"system","subtype":"turn_duration","uuid":"c","parentUuid":"b","durationMs":56847}
{"type":"last-prompt","lastPrompt":"question","leafUuid":"c","sessionId":"test"}
"#;
        let path = write_temp("activity-last-prompt", content);
        let later_mtime = 1_784_769_999_999;
        let session = parse_file(&path, later_mtime, None).expect("expected session");
        assert_eq!(session.updated_at_ms, 1_784_710_783_525);
        assert_ne!(session.updated_at_ms, later_mtime);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn activity_time_uses_new_query_when_response_is_not_complete() {
        let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"first question"}}
{"timestamp":"2026-07-23T01:03:03.456Z","type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"first answer"}]}}
{"timestamp":"2026-07-23T01:03:04.456Z","type":"system","subtype":"turn_duration","uuid":"c","parentUuid":"b"}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","uuid":"d","parentUuid":"c","message":{"role":"user","content":"second question"}}
"#;
        let path = write_temp("activity-open-query", content);
        let session = parse_file(&path, 1_784_799_999_999, None).expect("expected session");
        assert_eq!(session.updated_at_ms, 1_784_772_184_567);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn list_session_keeps_user_and_promoted_qa_submit_times() {
        let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","message":{"role":"user","content":"first question"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","toolUseResult":{"questions":[{"question":"Continue?"}],"answers":{"Continue?":"Yes"}}}
"#;
        let path = write_temp("turn-times", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns.len(), 2);
        assert_eq!(
            session.user_turn_timestamps_ms,
            vec![Some(1_784_768_523_456), Some(1_784_772_184_567)]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn broken_chain_falls_back_to_all_turns() {
        // The leaf references a uuid missing from the file: the chain cannot be
        // trusted, so every turn must survive.
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"user","uuid":"c","parentUuid":"ghost","message":{"role":"user","content":"질문2"}}
"#;
        let path = write_temp("broken-chain", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["질문1", "질문2"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sidechain_entries_do_not_become_leaf() {
        // A trailing subagent (sidechain) entry must not divert the active-path walk.
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"s1","parentUuid":null,"isSidechain":true,"message":{"role":"user","content":"사이드체인 프롬프트"}}
"#;
        let path = write_temp("sidechain-leaf", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["질문1"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn prefers_custom_title_over_ai_title() {
        let content = r#"
{"type":"ai-title","aiTitle":"GPS 센서 데이터 결합하여 정차 판정 개선","sessionId":"923fd752-5ba7-4cdb-8b54-d1074b046b7c"}
{"type":"custom-title","customTitle":"26-07 주행정보 생성 성능 개선 검토","sessionId":"923fd752-5ba7-4cdb-8b54-d1074b046b7c"}
{"type":"user","message":{"role":"user","content":"첫 질문"}}
"#;

        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "ular-s7s-test-{}.jsonl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::write(&path, content).expect("write temp file");

        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.title(), "26-07 주행정보 생성 성능 개선 검토");
        assert!(session.title_fixed);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn assistant_blob_indexes_last_answer_and_drops_intermediate() {
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b1","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"중간설명 intermediatekw"}]}}
{"type":"assistant","uuid":"b2","parentUuid":"b1","message":{"role":"assistant","content":[{"type":"text","text":"최종답변 finalkw"}]}}
{"type":"user","uuid":"c","parentUuid":"b2","message":{"role":"user","content":"질문2"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"두번째답변 secondkw"}]}}
"#;
        let path = write_temp("asst-blob", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        // Last answer of each turn is indexed; intermediate answers within a turn are not.
        assert!(session.assistant_blob.contains("finalkw"));
        assert!(session.assistant_blob.contains("secondkw"));
        assert!(!session.assistant_blob.contains("intermediatekw"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn assistant_blob_excludes_rewound_answer() {
        // Q2/A2 abandoned by /rewind: Q3 branches from the same parent (b).
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1 keepkw"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"질문2 버려진 분기"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"버려진답 dropkw"}]}}
{"type":"user","uuid":"e","parentUuid":"b","message":{"role":"user","content":"질문3 리와인드 후"}}
{"type":"assistant","uuid":"f","parentUuid":"e","message":{"role":"assistant","content":[{"type":"text","text":"답3 keep3kw"}]}}
"#;
        let path = write_temp("asst-rewind", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert!(session.assistant_blob.contains("keepkw"));
        assert!(session.assistant_blob.contains("keep3kw"));
        assert!(!session.assistant_blob.contains("dropkw"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn assistant_blob_excludes_bootstrap_ready_response() {
        // The bootstrap ready response precedes the first real question and must not
        // be indexed; the bootstrap prompt itself is not a user turn.
        let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"<s7s-context-bootstrap>\nRun something\n</s7s-context-bootstrap>"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"readykw 확인했습니다"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"진짜 질문"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"realkw 답변"}]}}
"#;
        let path = write_temp("asst-bootstrap", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["진짜 질문"]);
        assert!(session.assistant_blob.contains("realkw"));
        assert!(!session.assistant_blob.contains("readykw"));
        let _ = std::fs::remove_file(path);
    }
}
