//! Logic for saving session title changes.

use crate::model::{one_line, Agent, Session};
use crate::profile::Profile;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Persists the display title of a session to the storage location of the corresponding agent.
///
/// `profile` is the profile the session belongs to: its `path` is the config
/// root all title metadata paths derive from, so writes land in the same
/// account store the session was scanned from — not the default-path store.
/// The Claude CLI rename attempt also inherits the profile's env injection.
pub fn rename_session(profile: &Profile, session: &Session, title: &str) -> Result<()> {
    let title = normalize_title(title)?;
    match session.agent {
        Agent::Claude => rename_claude(profile, session, &title),
        Agent::Codex => rename_codex(&profile.path, session, &title),
        Agent::Antigravity => rename_antigravity(&profile.path, session, &title),
    }
}

fn normalize_title(title: &str) -> Result<String> {
    let title = one_line(title).trim().to_string();
    if title.is_empty() {
        return Err(anyhow!("Title cannot be empty"));
    }
    Ok(title)
}

fn rename_claude(profile: &Profile, session: &Session, title: &str) -> Result<()> {
    let cli_renamed = try_rename_claude_via_cli(profile, session, title).unwrap_or(false);
    let sessions_dir = profile.path.join("sessions");
    fs::create_dir_all(&sessions_dir)
        .with_context(|| format!("create {}", sessions_dir.display()))?;

    let mut matched = false;
    for entry in fs::read_dir(&sessions_dir)
        .with_context(|| format!("read {}", sessions_dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let data = match fs::read_to_string(&path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        let mut json: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if json
            .get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(|id| id == session.id)
        {
            ensure_object(&mut json);
            if let Some(obj) = json.as_object_mut() {
                obj.insert("sessionId".to_string(), Value::String(session.id.clone()));
                obj.insert("name".to_string(), Value::String(title.to_string()));
                obj.insert(
                    "nameSource".to_string(),
                    Value::String("custom".to_string()),
                );
            }
            fs::write(&path, serde_json::to_vec_pretty(&json)?)
                .with_context(|| format!("write {}", path.display()))?;
            matched = true;
        }
    }

    if !matched {
        let path = sessions_dir.join(format!("{}.json", session.id));
        let json = serde_json::json!({
            "sessionId": session.id.clone(),
            "name": title,
            "nameSource": "custom",
        });
        fs::write(&path, serde_json::to_vec_pretty(&json)?)
            .with_context(|| format!("write {}", path.display()))?;
    }

    if !cli_renamed {
        append_claude_title_event(session, title)?;
    }

    Ok(())
}

fn rename_codex(profile_root: &Path, session: &Session, title: &str) -> Result<()> {
    let path = profile_root.join("session_index.jsonl");
    let mut lines: Vec<String> = Vec::new();
    let mut matched = false;

    if let Ok(data) = fs::read_to_string(&path) {
        for line in data.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let mut json: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => {
                    lines.push(line.to_string());
                    continue;
                }
            };
            if json
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == session.id)
            {
                ensure_object(&mut json);
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(session.id.clone()));
                    obj.insert("thread_name".to_string(), Value::String(title.to_string()));
                }
                matched = true;
            }
            lines.push(serde_json::to_string(&json)?);
        }
    }

    if !matched {
        lines.push(serde_json::to_string(&serde_json::json!({
            "id": session.id.clone(),
            "thread_name": title,
        }))?);
    }

    write_lines(&path, &lines).with_context(|| format!("write {}", path.display()))?;
    update_codex_thread_title(profile_root, &session.id, title)
        .context("update Codex thread title")?;
    Ok(())
}

fn rename_antigravity(profile_root: &Path, session: &Session, title: &str) -> Result<()> {
    let anno_dir = profile_root.join("annotations");
    fs::create_dir_all(&anno_dir).with_context(|| format!("create {}", anno_dir.display()))?;

    let pbtxt_path = anno_dir.join(format!("{}.pbtxt", session.id));
    let mut lines: Vec<String> = Vec::new();
    let mut inserted = false;
    if let Ok(data) = fs::read_to_string(&pbtxt_path) {
        for line in data.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("title:\"") {
                lines.push(format!("title:\"{}\"", escape_pbtxt(title)));
                inserted = true;
            } else {
                lines.push(line.to_string());
            }
        }
    }
    if !inserted {
        if lines.is_empty() {
            lines.push(format!("title:\"{}\"", escape_pbtxt(title)));
        } else {
            lines.insert(0, format!("title:\"{}\"", escape_pbtxt(title)));
        }
    }
    write_lines(&pbtxt_path, &lines).with_context(|| format!("write {}", pbtxt_path.display()))?;

    let metadata_path = profile_root.join("cache/conversation_metadata.json");
    if let Ok(data) = fs::read_to_string(&metadata_path) {
        if let Ok(mut root) = serde_json::from_str::<Value>(&data) {
            if let Some(conversations) =
                root.get_mut("conversations").and_then(Value::as_object_mut)
            {
                let entry = conversations.entry(session.id.clone()).or_insert_with(|| {
                    serde_json::json!({
                        "summary": {}
                    })
                });
                if let Some(entry_obj) = entry.as_object_mut() {
                    let summary = entry_obj
                        .entry("summary".to_string())
                        .or_insert_with(|| serde_json::json!({}));
                    if let Some(summary_obj) = summary.as_object_mut() {
                        summary_obj.insert("Title".to_string(), Value::String(title.to_string()));
                    } else {
                        *summary = serde_json::json!({
                            "Title": title
                        });
                    }
                }
                fs::write(&metadata_path, serde_json::to_vec_pretty(&root)?)
                    .with_context(|| format!("write {}", metadata_path.display()))?;
            }
        }
    }

    Ok(())
}

fn update_codex_thread_title(profile_root: &Path, id: &str, title: &str) -> Result<()> {
    for db_path in codex_state_db_paths(profile_root) {
        let conn = match rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        ) {
            Ok(conn) => conn,
            Err(_) => continue,
        };

        let updated = conn.execute(
            "UPDATE threads SET title = ?1, updated_at = COALESCE(updated_at, strftime('%s','now')) WHERE id = ?2",
            rusqlite::params![title, id],
        );
        if let Ok(count) = updated {
            if count > 0 {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn codex_state_db_paths(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();

    let candidates = [
        root.join("state_1.sqlite"),
        root.join("state_2.sqlite"),
        root.join("state_3.sqlite"),
        root.join("state_4.sqlite"),
        root.join("state_5.sqlite"),
        root.join("sqlite").join("codex-dev.db"),
    ];
    for path in candidates {
        if path.exists() {
            out.push(path);
        }
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if !name.starts_with("state_") || !name.ends_with(".sqlite") {
                continue;
            }
            if !out.iter().any(|p| p == &path) {
                out.push(path);
            }
        }
    }

    out
}

fn write_lines(path: &Path, lines: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut out = lines.join("\n");
    out.push('\n');
    fs::write(path, out).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn escape_pbtxt(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', r#"\""#)
}

fn ensure_object(v: &mut Value) {
    if !v.is_object() {
        *v = Value::Object(serde_json::Map::new());
    }
}

fn append_claude_title_event(session: &Session, title: &str) -> Result<()> {
    let Some(path) = session.source_path.as_ref() else {
        return Ok(());
    };
    if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return Ok(());
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;

    if file.metadata().map(|m| m.len()).unwrap_or(0) > 0 {
        file.write_all(b"\n")
            .with_context(|| format!("write {}", path.display()))?;
    }

    let custom_title = serde_json::to_string(&serde_json::json!({
        "type": "custom-title",
        "customTitle": title,
        "sessionId": session.id,
    }))?;
    file.write_all(custom_title.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("write {}", path.display()))?;

    let agent_name = serde_json::to_string(&serde_json::json!({
        "type": "agent-name",
        "agentName": title,
        "sessionId": session.id,
    }))?;
    file.write_all(agent_name.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("write {}", path.display()))?;

    Ok(())
}

fn has_claude_title_events(data: &str, session_id: &str, title: &str) -> bool {
    let mut custom = false;
    let mut agent = false;

    for line in data.lines() {
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(kind) = json.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(id) = json.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        if id != session_id {
            continue;
        }
        match kind {
            "custom-title" => {
                custom = json
                    .get("customTitle")
                    .and_then(Value::as_str)
                    .is_some_and(|v| v == title);
            }
            "agent-name" => {
                agent = json
                    .get("agentName")
                    .and_then(Value::as_str)
                    .is_some_and(|v| v == title);
            }
            _ => {}
        }
        if custom && agent {
            return true;
        }
    }

    false
}

fn try_rename_claude_via_cli(profile: &Profile, session: &Session, title: &str) -> Result<bool> {
    #[cfg(test)]
    if std::env::var_os("ULAR_RESUME_TEST_ENABLE_CLAUDE_CLI").is_none() {
        return Ok(false);
    }

    let Some(path) = session.source_path.as_ref() else {
        return Ok(false);
    };
    if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return Ok(false);
    }

    let before = fs::read_to_string(path).unwrap_or_default();
    let cwd = if session.cwd.as_os_str().is_empty() {
        path.parent().unwrap_or_else(|| Path::new("."))
    } else {
        session.cwd.as_path()
    };

    let bin = std::env::var_os("ULAR_RESUME_CLAUDE_BIN").unwrap_or_else(|| "claude".into());
    let mut cmd = Command::new(bin);
    cmd.arg("--resume")
        .arg(&session.id)
        .arg("--name")
        .arg(title)
        .arg("-p")
        .arg("--output-format")
        .arg("json")
        .current_dir(cwd);
    // Same env rules as resume: strip contaminated Claude-session vars, and point
    // the CLI at the session's own account store (extra profiles only) — without
    // this the attempt runs against the default store and always misses.
    crate::resume::sanitize_agent_env(&mut cmd);
    if let Some((key, value)) = profile.env_var() {
        cmd.env(key, value);
    }
    let output = match cmd.output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };

    let after = fs::read_to_string(path).unwrap_or_default();
    if after == before {
        return Ok(false);
    }

    let renamed = has_claude_title_events(&after, &session.id, title);

    if renamed || output.status.success() {
        return Ok(renamed);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Mutex for serializing Claude rename path tests.
    ///
    /// Prevents flaky test runs: if other tests execute Claude rename while `prefers_claude_cli_rename...`
    /// sets ULAR_RESUME_* process-wide environment variables, the fake Claude script could be called
    /// and record duplicate entries in files.
    static CLAUDE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{}-{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn test_profile(agent: Agent, root: &Path) -> Profile {
        Profile {
            id: "profile-test".to_string(),
            agent,
            name: "Test".to_string(),
            path: root.to_path_buf(),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        }
    }

    #[test]
    fn writes_claude_title_meta() {
        let _guard = CLAUDE_ENV_LOCK.lock().expect("claude env lock");
        let root = temp_root("ular-s7s-rename-claude");
        let sessions_dir = root.join("sessions");
        let projects_dir = root.join("projects");
        let project_dir = projects_dir.join("-Users-username-DevSpace-my-project");
        fs::create_dir_all(&sessions_dir).expect("create dir");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::write(
            sessions_dir.join("session-a.json"),
            r#"{"sessionId":"abc-123","name":"old","nameSource":"derived"}"#,
        )
        .expect("write meta");
        fs::write(
            project_dir.join("abc-123.jsonl"),
            "{\"type\":\"ai-title\",\"aiTitle\":\"old\",\"sessionId\":\"abc-123\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"첫 질문\"}}",
        )
        .expect("write jsonl");
        let session = Session {
            agent: Agent::Claude,
            profile_id: String::new(),
            id: "abc-123".to_string(),
            source_path: Some(project_dir.join("abc-123.jsonl")),
            cwd: PathBuf::new(),
            folder: String::new(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["첫 질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        };

        rename_session(&test_profile(Agent::Claude, &root), &session, "새 제목").expect("rename");
        let data = fs::read_to_string(sessions_dir.join("session-a.json")).expect("read meta");
        let json: Value = serde_json::from_str(&data).expect("parse meta");
        assert_eq!(json.get("name").and_then(Value::as_str), Some("새 제목"));
        assert_eq!(
            json.get("nameSource").and_then(Value::as_str),
            Some("custom")
        );
        let project = fs::read_to_string(project_dir.join("abc-123.jsonl")).expect("read jsonl");
        assert!(project.contains(r#""type":"custom-title""#));
        assert!(project.contains(r#""customTitle":"새 제목""#));
        assert!(project.contains(r#""type":"agent-name""#));
        assert!(project.contains(r#""agentName":"새 제목""#));

        let parsed = crate::parser::claude::parse_file(&project_dir.join("abc-123.jsonl"), 0, None)
            .expect("parse claude");
        assert_eq!(parsed.title(), "새 제목");
        assert!(parsed.title_fixed);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prefers_claude_cli_rename_when_cli_writes_title_events() {
        let _guard = CLAUDE_ENV_LOCK.lock().expect("claude env lock");
        let root = temp_root("ular-s7s-rename-claude-cli");
        let sessions_dir = root.join("sessions");
        let projects_dir = root.join("projects");
        let project_dir = projects_dir.join("-Users-username-DevSpace-my-project");
        let bin_dir = root.join("bin");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        fs::create_dir_all(&project_dir).expect("create project dir");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        fs::write(
            sessions_dir.join("session-a.json"),
            r#"{"sessionId":"abc-123","name":"old","nameSource":"derived"}"#,
        )
        .expect("write meta");
        let project_path = project_dir.join("abc-123.jsonl");
        fs::write(
            &project_path,
            "{\"type\":\"ai-title\",\"aiTitle\":\"old\",\"sessionId\":\"abc-123\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"첫 질문\"}}\n",
        )
        .expect("write jsonl");

        let fake_claude = bin_dir.join("claude");
        fs::write(
            &fake_claude,
            format!(
                "#!/bin/sh\ncat <<'EOF' >> \"{}\"\n{{\"type\":\"custom-title\",\"customTitle\":\"새 제목\",\"sessionId\":\"abc-123\"}}\n{{\"type\":\"agent-name\",\"agentName\":\"새 제목\",\"sessionId\":\"abc-123\"}}\nEOF\nprintf '{{\"type\":\"result\",\"subtype\":\"success\"}}\\n'\n",
                project_path.display()
            ),
        )
        .expect("write fake claude");
        let mut perms = fs::metadata(&fake_claude)
            .expect("stat fake claude")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_claude, perms).expect("chmod fake claude");

        let old_bin = std::env::var_os("ULAR_RESUME_CLAUDE_BIN");
        let old_enable = std::env::var_os("ULAR_RESUME_TEST_ENABLE_CLAUDE_CLI");
        std::env::set_var("ULAR_RESUME_CLAUDE_BIN", &fake_claude);
        std::env::set_var("ULAR_RESUME_TEST_ENABLE_CLAUDE_CLI", "1");

        let session = Session {
            agent: Agent::Claude,
            profile_id: String::new(),
            id: "abc-123".to_string(),
            source_path: Some(project_path.clone()),
            cwd: PathBuf::new(),
            folder: String::new(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["첫 질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        };

        rename_session(&test_profile(Agent::Claude, &root), &session, "새 제목")
            .expect("rename via cli");

        match old_bin {
            Some(v) => std::env::set_var("ULAR_RESUME_CLAUDE_BIN", v),
            None => std::env::remove_var("ULAR_RESUME_CLAUDE_BIN"),
        }
        match old_enable {
            Some(v) => std::env::set_var("ULAR_RESUME_TEST_ENABLE_CLAUDE_CLI", v),
            None => std::env::remove_var("ULAR_RESUME_TEST_ENABLE_CLAUDE_CLI"),
        }

        let project = fs::read_to_string(&project_path).expect("read jsonl");
        assert_eq!(project.matches(r#""type":"custom-title""#).count(), 1);
        assert_eq!(project.matches(r#""type":"agent-name""#).count(), 1);
        let parsed = crate::parser::claude::parse_file(&project_path, 0, None).expect("parse");
        assert_eq!(parsed.title(), "새 제목");
        assert!(parsed.title_fixed);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn writes_codex_index_line() {
        let root = temp_root("ular-s7s-rename-codex");
        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create dir");
        fs::write(
            root.join("session_index.jsonl"),
            r#"{"id":"abc-123","thread_name":"old","other":1}"#,
        )
        .expect("write index");
        let db_path = root.join("state_5.sqlite");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
        conn.execute(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0, cwd TEXT NOT NULL DEFAULT '')",
            [],
        )
        .expect("create table");
        conn.execute(
            "INSERT INTO threads (id, title, updated_at, cwd) VALUES (?1, ?2, 1, ?3)",
            rusqlite::params!["abc-123", "old", "/tmp/demo"],
        )
        .expect("insert thread");
        let session = Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: "abc-123".to_string(),
            source_path: None,
            cwd: PathBuf::new(),
            folder: String::new(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec![],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        };

        rename_session(&test_profile(Agent::Codex, &root), &session, "새 제목").expect("rename");
        let data = fs::read_to_string(root.join("session_index.jsonl")).expect("read index");
        assert!(data.contains(r#""thread_name":"새 제목""#));
        assert!(data.contains(r#""other":1"#));
        let title: String = rusqlite::Connection::open(&db_path)
            .expect("reopen sqlite")
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                rusqlite::params!["abc-123"],
                |row| row.get(0),
            )
            .expect("query thread title");
        assert_eq!(title, "새 제목");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn writes_antigravity_annotation_title() {
        let root = temp_root("ular-s7s-rename-antigravity");
        let agy_root = root.join(".gemini/antigravity-cli");
        let session = Session {
            agent: Agent::Antigravity,
            profile_id: String::new(),
            id: "abc-123".to_string(),
            source_path: None,
            cwd: PathBuf::new(),
            folder: String::new(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec![],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        };

        rename_session(
            &test_profile(Agent::Antigravity, &agy_root),
            &session,
            "새 제목",
        )
        .expect("rename");
        let pbtxt =
            fs::read_to_string(root.join(".gemini/antigravity-cli/annotations/abc-123.pbtxt"))
                .expect("read annotation");
        assert!(pbtxt.contains(r#"title:"새 제목""#));

        let _ = fs::remove_dir_all(&root);
    }
}
