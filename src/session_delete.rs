//! Session artifact deletion: removes the on-disk stores a session is made of.
//!
//! Shared by the TUI delete action and `s7s session delete`, so both paths obey
//! the same profile-scoping rule: every auxiliary store written to is derived
//! from the config root of the profile the session belongs to (`Profile.path`).
//! A session whose profile is gone means a stale list, so the auxiliary cleanup
//! is skipped rather than falling back to a default root — that would delete
//! another account's metadata.
//!
//! Removing the transcript is no longer the whole job for Codex: since 0.153 a
//! session's turns are also projected into `thread_history_*.sqlite`, and its
//! title and first user message live in `threads` in `state_*.sqlite`. Deleting
//! only the rollout file left the text of a deleted session readable in both.
//!
//! Deletion is irreversible: the session transcript is removed, not archived.

use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Removes the transcript and every agent-specific store that holds the session.
///
/// The transcript is authoritative: its removal failing is an error, while the
/// auxiliary cleanup (Codex thread rows, Antigravity metadata, sqlite sidecars)
/// is best effort so a partially-written store cannot block the delete.
pub fn delete_session_artifacts(profiles: &ProfileStore, session: &Session) -> Result<()> {
    let Some(source_path) = session.source_path.as_ref() else {
        return Err(anyhow!("source path is missing"));
    };

    // Ask codex to drop the thread first: the app server reaches every store
    // codex keeps for it, including ones no s7s version knows about. The reply
    // is not trusted — the direct cleanup below runs either way and is a no-op
    // on rows the app server already removed.
    if session.agent == Agent::Codex {
        if let Some(profile) = profiles.find(&session.profile_id) {
            let params = serde_json::json!({ "threadId": session.id });
            let _ = crate::codex_app_server::call(profile, "thread/delete", params);
        }
    }

    remove_file_best_effort(source_path)
        .with_context(|| format!("remove {}", source_path.display()))?;

    // Best-effort auxiliary cleanup; skipped when the owning profile is gone
    // (never touch another profile's store).
    match session.agent {
        Agent::Codex => {
            if let Some(root) = session_profile_root(profiles, session) {
                remove_codex_thread_records(&root, session.id.as_str());
            }
        }
        Agent::Antigravity => {
            if let Some(root) = session_profile_root(profiles, session) {
                let _ = remove_antigravity_metadata(&root, session.id.as_str());
                let _ = remove_file_best_effort(
                    &root
                        .join("annotations")
                        .join(format!("{}.pbtxt", session.id)),
                );
            }
            remove_sqlite_sidecars(source_path);
        }
        Agent::Claude => {}
    }

    // The s7s-owned folder record (handoff launch folder, or a `Change Folder`
    // override) is independent of the external profile root, so remove it for
    // every agent and even if that profile disappeared after the last scan.
    let _ = crate::session_workspace::remove(
        &crate::config::session_workspaces_path(),
        &session.profile_id,
        &session.id,
    );

    Ok(())
}

/// Removes every stored trace of one codex thread outside its rollout file.
///
/// Each statement is independent and failure is ignored: a table absent on an
/// older codex build, a database held by a running CLI, or a row the app server
/// already removed must not turn a completed delete into an error.
fn remove_codex_thread_records(profile_root: &Path, id: &str) {
    for db_path in crate::parser::codex::state_db_paths(profile_root) {
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        ) else {
            continue;
        };
        let _ = conn.execute("DELETE FROM threads WHERE id = ?1", rusqlite::params![id]);
        for table in ["thread_attachments", "thread_dynamic_tools"] {
            let _ = conn.execute(
                &format!("DELETE FROM {table} WHERE thread_id = ?1"),
                rusqlite::params![id],
            );
        }
    }

    for db_path in crate::parser::codex::history_db_paths(profile_root) {
        let Ok(conn) = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        ) else {
            continue;
        };
        for table in [
            "thread_items",
            "thread_turns",
            "thread_realtime_items",
            "thread_history_projection_state",
        ] {
            let _ = conn.execute(
                &format!("DELETE FROM {table} WHERE thread_id = ?1"),
                rusqlite::params![id],
            );
        }
    }

    let _ = remove_codex_index_records(&profile_root.join("session_index.jsonl"), id);
}

/// Drops the `session_index.jsonl` records of one thread. The file is codex's
/// append-only title index, and s7s writes to it during a rename, so a deleted
/// session must not keep a name there.
fn remove_codex_index_records(path: &Path, id: &str) -> Result<()> {
    let Ok(data) = fs::read_to_string(path) else {
        return Ok(());
    };
    let mut kept: Vec<&str> = Vec::new();
    let mut dropped = false;
    for line in data.lines() {
        let matches_id = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|v| {
                v.get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(|found| found == id)
            })
            .unwrap_or(false);
        if matches_id {
            dropped = true;
            continue;
        }
        kept.push(line);
    }
    if !dropped {
        return Ok(());
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    fs::write(path, out).with_context(|| format!("write {}", path.display()))
}

/// Config root (`Profile.path`) of the profile a session belongs to.
/// Sessions are re-stamped with live profile ids on every scan, so a miss
/// means a stale list — callers must not fall back to the default root.
pub fn session_profile_root(profiles: &ProfileStore, session: &Session) -> Option<PathBuf> {
    profiles.find(&session.profile_id).map(|p| p.path.clone())
}

fn remove_file_best_effort(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn remove_sqlite_sidecars(db_path: &Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut sidecar = db_path.to_path_buf();
        let name = match db_path.file_name().and_then(|s| s.to_str()) {
            Some(name) => format!("{name}{suffix}"),
            None => continue,
        };
        sidecar.set_file_name(name);
        let _ = fs::remove_file(sidecar);
    }
}

fn remove_antigravity_metadata(profile_root: &Path, id: &str) -> Result<()> {
    let path = profile_root.join("cache/conversation_metadata.json");
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let mut root: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    if let Some(conversations) = root
        .get_mut("conversations")
        .and_then(serde_json::Value::as_object_mut)
    {
        conversations.remove(id);
        let bytes = serde_json::to_vec_pretty(&root)?;
        fs::write(&path, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Agent;
    use crate::profile::Profile;

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

    fn store(agent: Agent, root: &Path) -> ProfileStore {
        ProfileStore {
            profiles: vec![Profile {
                id: "profile-test".to_string(),
                agent,
                name: "Test".to_string(),
                path: root.to_path_buf(),
                oauth_token: None,
                active: true,
                shortcut: None,
                builtin: false,
            }],
        }
    }

    fn session(agent: Agent, id: &str, source: &Path) -> Session {
        Session {
            agent,
            profile_id: "profile-test".to_string(),
            id: id.to_string(),
            source_path: Some(source.to_path_buf()),
            cwd: PathBuf::new(),
            folder: String::new(),
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
            context_source: None,
        }
    }

    /// Deleting a codex session must empty every store that holds its text.
    /// Removing only the rollout file left the title and the first user message
    /// in `threads`, and every turn in the paginated history, readable.
    #[test]
    fn codex_delete_clears_the_thread_from_every_store() {
        let root = temp_root("ular-s7s-delete-codex");
        let sessions_dir = root.join("sessions/2026/09/18");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let rollout = sessions_dir.join("rollout-2026-09-18T01-27-23-thread-1.jsonl");
        fs::write(&rollout, "{\"type\":\"session_meta\"}\n").expect("write rollout");

        let state = rusqlite::Connection::open(root.join("state_1.sqlite")).expect("open state");
        state
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, name TEXT, title TEXT);
                 CREATE TABLE thread_attachments (thread_id TEXT, path TEXT);
                 INSERT INTO threads VALUES ('thread-1', '제목', '첫 질문');
                 INSERT INTO threads VALUES ('thread-2', '남는 제목', '남는 질문');
                 INSERT INTO thread_attachments VALUES ('thread-1', '/tmp/a.png');",
            )
            .expect("seed state");
        drop(state);

        let history =
            rusqlite::Connection::open(root.join("thread_history_1.sqlite")).expect("open history");
        history
            .execute_batch(
                "CREATE TABLE thread_items (thread_id TEXT, item_json TEXT);
                 CREATE TABLE thread_turns (thread_id TEXT, turn_id TEXT);
                 CREATE TABLE thread_realtime_items (thread_id TEXT, item_id TEXT);
                 CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY);
                 INSERT INTO thread_items VALUES ('thread-1', '{\"text\":\"첫 질문\"}');
                 INSERT INTO thread_items VALUES ('thread-2', '{\"text\":\"남는 질문\"}');
                 INSERT INTO thread_turns VALUES ('thread-1', 'turn-1');
                 INSERT INTO thread_realtime_items VALUES ('thread-1', 'item-1');
                 INSERT INTO thread_history_projection_state VALUES ('thread-1');",
            )
            .expect("seed history");
        drop(history);

        fs::write(
            root.join("session_index.jsonl"),
            "{\"id\":\"thread-1\",\"thread_name\":\"제목\"}\n{\"id\":\"thread-2\",\"thread_name\":\"남는 제목\"}\n",
        )
        .expect("write index");

        delete_session_artifacts(
            &store(Agent::Codex, &root),
            &session(Agent::Codex, "thread-1", &rollout),
        )
        .expect("delete");

        assert!(!rollout.exists(), "rollout file must be gone");

        let state = rusqlite::Connection::open(root.join("state_1.sqlite")).expect("reopen state");
        let threads: i64 = state
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get(0),
            )
            .expect("count threads");
        assert_eq!(threads, 0);
        let attachments: i64 = state
            .query_row("SELECT COUNT(*) FROM thread_attachments", [], |row| {
                row.get(0)
            })
            .expect("count attachments");
        assert_eq!(attachments, 0);
        let kept: i64 = state
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 'thread-2'",
                [],
                |row| row.get(0),
            )
            .expect("count kept thread");
        assert_eq!(kept, 1, "another session's row must survive");

        let history = rusqlite::Connection::open(root.join("thread_history_1.sqlite"))
            .expect("reopen history");
        for table in [
            "thread_items",
            "thread_turns",
            "thread_realtime_items",
            "thread_history_projection_state",
        ] {
            let left: i64 = history
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE thread_id = 'thread-1'"),
                    [],
                    |row| row.get(0),
                )
                .expect("count history rows");
            assert_eq!(left, 0, "{table} still holds the deleted thread");
        }
        let kept_items: i64 = history
            .query_row(
                "SELECT COUNT(*) FROM thread_items WHERE thread_id = 'thread-2'",
                [],
                |row| row.get(0),
            )
            .expect("count kept items");
        assert_eq!(kept_items, 1);

        let index = fs::read_to_string(root.join("session_index.jsonl")).expect("read index");
        assert!(!index.contains("thread-1"));
        assert!(index.contains("thread-2"));

        let _ = fs::remove_dir_all(&root);
    }

    /// A codex profile with none of the newer stores must still delete cleanly:
    /// the sqlite cleanup is best effort and cannot turn into an error.
    #[test]
    fn codex_delete_succeeds_without_the_sqlite_stores() {
        let root = temp_root("ular-s7s-delete-codex-bare");
        let sessions_dir = root.join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let rollout = sessions_dir.join("rollout-thread-9.jsonl");
        fs::write(&rollout, "{}\n").expect("write rollout");

        delete_session_artifacts(
            &store(Agent::Codex, &root),
            &session(Agent::Codex, "thread-9", &rollout),
        )
        .expect("delete");
        assert!(!rollout.exists());

        let _ = fs::remove_dir_all(&root);
    }

    /// s7s writes the Antigravity rename into `annotations/<id>.pbtxt`, so the
    /// delete has to take it back: the title of a deleted session must not stay
    /// behind and reattach to a later conversation with the same id.
    #[test]
    fn antigravity_delete_removes_the_rename_annotation() {
        let root = temp_root("ular-s7s-delete-agy");
        let conversations = root.join("conversations");
        let annotations = root.join("annotations");
        fs::create_dir_all(&conversations).expect("create conversations");
        fs::create_dir_all(&annotations).expect("create annotations");
        let db = conversations.join("conv-1.db");
        fs::write(&db, "").expect("write db");
        let annotation = annotations.join("conv-1.pbtxt");
        fs::write(&annotation, "title:\"제목\"\n").expect("write annotation");
        let other = annotations.join("conv-2.pbtxt");
        fs::write(&other, "title:\"남는 제목\"\n").expect("write other annotation");

        delete_session_artifacts(
            &store(Agent::Antigravity, &root),
            &session(Agent::Antigravity, "conv-1", &db),
        )
        .expect("delete");

        assert!(!db.exists());
        assert!(!annotation.exists(), "rename annotation must be gone");
        assert!(other.exists(), "another session's annotation must survive");

        let _ = fs::remove_dir_all(&root);
    }
}
