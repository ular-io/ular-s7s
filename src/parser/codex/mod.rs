//! OpenAI Codex session parser.
//!
//! File path: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`
//! Session ID/cwd are extracted from `session_meta` lines. User turns are identified in `event_msg`
//! where `payload.type=="item_completed"` carries an `item.type=="UserMessage"` (codex 0.147+), where
//! `payload.type=="user_message"` (pre-0.147), or in user messages within `response_item`.
//!
//! Title stores (codex 0.153): `threads.name` in `<root>/state_*.sqlite` is what the
//! codex CLI itself reads and writes; `session_index.jsonl` is an append-only log
//! codex also writes on rename (last record for an id wins). `threads.title` holds
//! codex's own auto-derived text, not a user title, so it is never read here — s7s
//! derives its own fallback title from the first user turn.
//!
//! Backtrack (esc-esc "edit previous message") handling: the rollout is append-only; editing
//! a past message appends `event_msg` `payload.type=="thread_rolled_back"` with `num_turns` =
//! the number of most recent turns discarded, then the replacement turn follows. Turns dropped
//! by the rollback are removed from the preview. Verified against codex CLI 0.144.4 — the
//! rollback happens in the same rollout file (no fork file is created).
//!
//! Codex 0.159 records a rewind as a new rollout segment instead; see [`segments`]. Every
//! rollout is read through [`segments::read_rollout`], so a segment parses as its whole thread.

pub(crate) mod events;
pub(crate) mod segments;

use super::{build_assistant_blob, clean_turn, finalize, is_noise_turn, session_updated_at_ms};
use crate::model::{Agent, Session};
use events::{CodexRecord, UserTextKind};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct TurnIndex {
    indexable: bool,
    last_assistant: Option<String>,
    last_response_at_ms: Option<i64>,
}

/// OpenAI Codex session title metadata.
pub struct TitleMeta {
    pub title: Option<String>,
}

/// Config roots to look for codex storage in. The caller may pass `~/.codex` or
/// `~/.codex/sessions`, so both the directory and its parent are tried.
fn config_roots(cli_dir: &Path) -> Vec<PathBuf> {
    let mut roots = vec![cli_dir.to_path_buf()];
    if let Some(parent) = cli_dir.parent() {
        roots.push(parent.to_path_buf());
    }
    roots
}

/// `state_*.sqlite` files under a codex config root, newest suffix last so a
/// later file's row wins. `sqlite/codex-*.db` is deliberately excluded: it holds
/// `local_thread_catalog`, an observation cache with no `threads` table, which
/// codex rebuilds by scanning and s7s never writes.
pub(crate) fn state_db_paths(root: &Path) -> Vec<PathBuf> {
    numbered_db_paths(root, "state_")
}

/// `thread_history_*.sqlite` files under a codex config root. Codex 0.153
/// started projecting each rollout into this database (`history_mode =
/// paginated`), so a session's items survive here after its rollout file is
/// gone; deletion has to reach it.
pub(crate) fn history_db_paths(root: &Path) -> Vec<PathBuf> {
    numbered_db_paths(root, "thread_history_")
}

/// Sqlite files whose name starts with `prefix` and ends with `.sqlite`, sorted
/// so a later suffix comes last.
fn numbered_db_paths(root: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with(prefix) && name.ends_with(".sqlite") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Loads explicit thread names (session_id -> name).
///
/// `threads.name` in `state_*.sqlite` wins, because that is the column the codex
/// CLI reads for its own display. `session_index.jsonl` is read first as the
/// broader/legacy source; within it the last record for an id wins, since codex
/// appends rather than rewriting.
pub fn load_title_meta(cli_dir: &Path) -> HashMap<String, TitleMeta> {
    let mut out = HashMap::new();
    let roots = config_roots(cli_dir);

    for root in &roots {
        load_index_titles(&root.join("session_index.jsonl"), &mut out);
    }
    for root in &roots {
        for db in state_db_paths(root) {
            load_state_names(&db, &mut out);
        }
    }

    out
}

/// Merges `session_index.jsonl` records. A record with no usable name still
/// registers the id so a later empty record cannot erase an earlier name.
fn load_index_titles(path: &Path, out: &mut HashMap<String, TitleMeta>) {
    let Ok(data) = std::fs::read_to_string(path) else {
        return;
    };
    for line in data.lines() {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            continue;
        };
        let title = v
            .get("thread_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if title.is_some() || !out.contains_key(id) {
            out.insert(id.to_string(), TitleMeta { title });
        }
    }
}

/// Overlays `threads.name` from one state database. A missing table or column
/// (older codex) leaves the map untouched.
fn load_state_names(db_path: &Path, out: &mut HashMap<String, TitleMeta>) {
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return;
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT id, name FROM threads WHERE name IS NOT NULL AND name <> ''")
    else {
        return;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return;
    };
    for (id, name) in rows.flatten() {
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.insert(id, TitleMeta { title: Some(name) });
    }
}

/// Parses a single Codex rollout JSONL file. Returns None if there are no valid user turns.
pub fn parse_file(
    path: &Path,
    source_mtime_ms: i64,
    meta_map: Option<&HashMap<String, TitleMeta>>,
) -> Option<Session> {
    let content = segments::read_rollout(path).ok()?;

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
    let mut context_source: Option<crate::model::ContextSource> = None;

    let decoder = events::Decoder::new(&content);
    for (line_no, line) in content.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match decoder.decode(line_no, &v) {
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
                // The genuine launch envelope is always the leading user turn;
                // only capture before any real turn so a later message that merely
                // quotes a bootstrap (e.g. a meta-discussion) is not misread.
                if context_source.is_none() && turns.is_empty() {
                    context_source = crate::parser::parse_context_bootstrap(&u.text);
                }
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
        context_source,
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
/// A segment name's `_<segment id>` suffix is not part of the thread id.
fn extract_uuid_from_name(path: &Path) -> String {
    if let Some((id, _)) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(segments::thread_id_from_name)
    {
        return id.to_string();
    }
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
mod tests;
