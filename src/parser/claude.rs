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
use super::{clean_turn, finalize, is_noise_turn, turn};
use crate::model::{Agent, Session};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

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
        let fixed = !matches!(
            v.get("nameSource").and_then(Value::as_str),
            Some("derived") | Some("auto")
        );
        out.insert(id, TitleMeta { title, fixed });
    }

    out
}

/// Parses a single Claude JSONL file. Returns None if there are no valid user turns.
pub fn parse_file(path: &Path, mtime_ms: i64, meta: Option<&TitleMeta>) -> Option<Session> {
    let content = std::fs::read_to_string(path).ok()?;
    let id = path.file_stem()?.to_string_lossy().to_string();

    let mut cwd: Option<String> = None;
    // Turn texts paired with the uuid of the line they came from; filtered to the
    // active parentUuid chain after the scan (see module docs on `/rewind`).
    let mut candidates: Vec<(Option<String>, String)> = Vec::new();
    let mut parents: HashMap<String, Option<String>> = HashMap::new();
    let mut leaf: Option<String> = None;
    let mut explicit_title: Option<String> = None;
    let mut auto_title: Option<String> = None;

    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Use the first occurrence of cwd from any line.
        if cwd.is_none() {
            if let Some(c) = v.get("cwd").and_then(Value::as_str) {
                if !c.is_empty() {
                    cwd = Some(c.to_string());
                }
            }
        }

        match v.get("type").and_then(Value::as_str) {
            Some("custom-title") => {
                if let Some(t) = v.get("customTitle").and_then(Value::as_str) {
                    let t = t.trim();
                    if !t.is_empty() {
                        explicit_title = Some(t.to_string());
                        // Explicit titles set via `/rename` always take precedence over auto-generated titles.
                    }
                }
                continue;
            }
            Some("agent-name") => {
                if explicit_title.is_none() {
                    if let Some(t) = v.get("agentName").and_then(Value::as_str) {
                        let t = t.trim();
                        if !t.is_empty() {
                            explicit_title = Some(t.to_string());
                        }
                    }
                }
                continue;
            }
            Some("ai-title") => {
                if auto_title.is_none() && explicit_title.is_none() {
                    if let Some(t) = v.get("aiTitle").and_then(Value::as_str) {
                        let t = t.trim();
                        if !t.is_empty() {
                            auto_title = Some(t.to_string());
                        }
                    }
                }
                continue;
            }
            _ => {}
        }

        // Exclude subagent (sidechain) conversations.
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        // Record the parent link of every non-sidechain entry that carries a uuid;
        // the last such entry is the head (leaf) of the active conversation branch.
        let uuid = v.get("uuid").and_then(Value::as_str).map(str::to_string);
        if let Some(u) = uuid.clone() {
            let parent = v
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(str::to_string);
            parents.insert(u.clone(), parent);
            leaf = Some(u);
        }

        if v.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        if let Some(text) = turn::extract_user_text(&v) {
            if !is_noise_turn(&text) {
                if let Some(cleaned) = clean_turn(&text) {
                    candidates.push((uuid.clone(), cleaned));
                }
            }
        }
        if let Some(qa) = turn::extract_question_answers(&v) {
            if !is_noise_turn(&qa) {
                if let Some(cleaned) = clean_turn(&qa) {
                    candidates.push((uuid, cleaned));
                }
            }
        }
    }

    let turns = active_path_turns(candidates, &parents, leaf.as_deref());
    if turns.is_empty() {
        return None;
    }

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
        mtime_ms,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: turns,
        search_blob: String::new(),
        title_hint,
        title_fixed,
    };
    finalize(&mut session);
    Some(session)
}

/// Keeps only turns on the active branch: walks `parentUuid` links from the leaf to the
/// root and drops turns abandoned by `/rewind`. Falls back to all turns when the chain is
/// broken (an entry references a uuid that is not in the file) so an unexpected format
/// change can never blank out a session preview.
fn active_path_turns(
    candidates: Vec<(Option<String>, String)>,
    parents: &HashMap<String, Option<String>>,
    leaf: Option<&str>,
) -> Vec<String> {
    let Some(active) = active_uuid_set(parents, leaf) else {
        return candidates.into_iter().map(|(_, text)| text).collect();
    };

    candidates
        .into_iter()
        .filter(|(uuid, _)| match uuid {
            Some(u) => active.contains(u.as_str()),
            None => true,
        })
        .map(|(_, text)| text)
        .collect()
}

/// Set of uuids on the active branch (leaf -> root walk over `parentUuid` links).
///
/// Returns None when the chain cannot be trusted — no leaf, or a dangling parent
/// reference — meaning the caller must keep every entry. Shared between the list
/// parser and the detailed session-context parser so both views agree on which
/// turns were abandoned by `/rewind`.
pub(crate) fn active_uuid_set(
    parents: &HashMap<String, Option<String>>,
    leaf: Option<&str>,
) -> Option<HashSet<String>> {
    let leaf = leaf?;
    let mut active: HashSet<String> = HashSet::new();
    let mut cursor = Some(leaf.to_string());
    while let Some(u) = cursor {
        if !active.insert(u.clone()) {
            break; // Cycle guard (corrupt file).
        }
        match parents.get(&u) {
            Some(parent) => cursor = parent.clone(),
            // Dangling reference: the chain cannot be trusted, keep everything.
            None => return None,
        }
    }
    Some(active)
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
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"질문2 버려진 분기"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"답2"}]}}
{"type":"user","uuid":"e","parentUuid":"b","message":{"role":"user","content":"질문3 리와인드 후"}}
{"type":"assistant","uuid":"f","parentUuid":"e","message":{"role":"assistant","content":[{"type":"text","text":"답3"}]}}
"#;
        let path = write_temp("rewind-branch", content);
        let session = parse_file(&path, 0, None).expect("expected session");
        assert_eq!(session.user_turns, vec!["질문1", "질문3 리와인드 후"]);
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
}
