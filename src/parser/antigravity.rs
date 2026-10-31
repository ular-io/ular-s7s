//! Antigravity CLI session parser.
//!
//! Actual conversation contents are stored in `conversations/<id>.db` (SQLite). The `history.jsonl` file
//! is not used because it only logs raw prompt inputs typed in the input box (including slash commands),
//! making the initial question unreliable or missing.
//!
//! `.db` schema:
//! - Table `steps`, rows where `step_type=14` represent user turns.
//! - In `step_payload` (protobuf), user message is field `19.2`,
//!   timestamp (seconds) is field `5.1.1`, and workspace URI is field `19.12.12`.
//! - Rows where `step_type=138` represent ask_question (agent questions). For each field `154.1` block,
//!   question is `1`, options are repeated `2` (`.1` code, `.2` text), and the selected
//!   option code is `4` (only present when answered, skipped questions contain a `6` flag instead).
//!
//! Since one file equals one session, file-level mtime cache is applied similar to claude/codex.

use super::{build_assistant_blob, clean_turn, finalize, is_noise_turn, session_updated_at_ms};
use crate::model::{Agent, Session};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Path to the conversations directory.
pub fn conversations_dir(cli_dir: &Path) -> PathBuf {
    cli_dir.join("conversations")
}

/// Supplementary metadata per conversation (Title/Preview/Workspace). Used as fallback if values are missing in the DB.
pub struct Meta {
    pub title: Option<String>,
    pub preview: Option<String>,
    pub workspace: Option<String>,
}

/// Loads conversation_metadata.json (conversation ID -> Meta).
pub fn load_metadata(cli_dir: &Path) -> HashMap<String, Meta> {
    let mut out = HashMap::new();
    let path = cli_dir.join("cache/conversation_metadata.json");
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return out,
    };
    let root: Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let convs = match root.get("conversations").and_then(Value::as_object) {
        Some(o) => o,
        None => return out,
    };
    for (id, entry) in convs {
        let summary = entry.get("summary");
        let title = summary
            .and_then(|s| s.get("Title"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let preview = summary
            .and_then(|s| s.get("Preview"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let workspace = summary
            .and_then(|s| s.get("WorkspaceURIs"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(strip_file_uri);
        out.insert(
            id.clone(),
            Meta {
                title,
                preview,
                workspace,
            },
        );
    }

    // `/rename` outputs are sometimes only written to annotations/*.pbtxt.
    let anno_dir = cli_dir.join("annotations");
    if let Ok(entries) = std::fs::read_dir(&anno_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("pbtxt") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let title = parse_pbtxt_title(&data);
            if title.is_none() {
                continue;
            }
            out.entry(id)
                .and_modify(|meta| meta.title = title.clone())
                .or_insert(Meta {
                    title,
                    preview: None,
                    workspace: None,
                });
        }
    }
    out
}

/// Parses a single conversation `.db`. Returns None if there are no valid user turns and fallback via Preview/Title is not possible.
pub fn parse_db(
    path: &Path,
    source_mtime_ms: i64,
    meta: &HashMap<String, Meta>,
) -> Option<Session> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let m = meta.get(&id);

    let mut turns: Vec<String> = Vec::new();
    let mut turn_timestamps: Vec<Option<i64>> = Vec::new();
    let mut cwd: Option<String> = None;
    let mut context_source: Option<crate::model::ContextSource> = None;

    // Open as read-only and immutable to avoid locks or mutations during execution.
    if let Ok(conn) = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    ) {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT step_type, step_payload FROM steps WHERE step_type IN (14, 138) ORDER BY idx",
        ) {
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            });
            if let Ok(rows) = rows {
                for (step_type, payload) in rows.flatten() {
                    if step_type == 138 {
                        // Agent questions (ask_question): promote answered ones
                        // to virtual user turns in the format `· question → answer`.
                        if let Some(qa) = extract_ask_answers(&payload) {
                            if let Some(cleaned) = clean_turn(&qa) {
                                turns.push(cleaned);
                                turn_timestamps
                                    .push(step_timestamp_ms(&payload).filter(|value| *value > 0));
                            }
                        }
                        continue;
                    }
                    if let Some((msg, ws, ts)) = extract_user_step(&payload) {
                        if cwd.is_none() {
                            if let Some(w) = ws {
                                if !w.is_empty() {
                                    cwd = Some(w);
                                }
                            }
                        }
                        // Only the leading user turn is a genuine launch envelope;
                        // a later message quoting a bootstrap must not be misread.
                        if context_source.is_none() && turns.is_empty() {
                            context_source = crate::parser::parse_context_bootstrap(&msg);
                        }
                        if !is_noise_turn(&msg) {
                            if let Some(cleaned) = clean_turn(&msg) {
                                turns.push(cleaned);
                                turn_timestamps.push((ts > 0).then_some(ts));
                            }
                        }
                    }
                }
            }
        }
    }

    // Fall back to the workspace from metadata.
    if cwd.is_none() {
        cwd = m.and_then(|m| m.workspace.clone());
    }
    // Fall back to Preview if no user turns were extracted (still displays the session).
    if turns.is_empty() {
        if let Some(preview) = m.and_then(|m| m.preview.clone()) {
            if let Some(cleaned) = clean_turn(&preview) {
                turns.push(cleaned);
                turn_timestamps.push(None);
            }
        }
    }
    if turns.is_empty() {
        return None;
    }

    let cwd = cwd.unwrap_or_default();
    let folder = Path::new(&cwd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.clone());

    // Assistant answers live in the JSONL transcript, not the DB. Reuse the detailed
    // transcript parser to pull each turn's last assistant text for the search index;
    // a missing/rotated transcript simply yields fewer (or no) indexed answers, and
    // search falls back to user turns for that session.
    let (assistant_per_turn, last_response_completed_at_ms): (Vec<String>, Option<i64>) =
        crate::session_context::antigravity::transcript_path(path, &id)
            .and_then(|tp| {
                crate::session_context::antigravity::parse_turns_with_activity_for_db(&tp, path)
                    .ok()
            })
            .map(|(turns, completed_at_ms)| {
                let answers = turns
                    .into_iter()
                    .filter_map(|t| t.last_assistant_text)
                    .collect();
                (answers, completed_at_ms)
            })
            .unwrap_or_default();
    let updated_at_ms = session_updated_at_ms(
        &turn_timestamps,
        last_response_completed_at_ms,
        source_mtime_ms,
    );

    let mut s = Session {
        agent: Agent::Antigravity,
        profile_id: String::new(),
        id,
        source_path: Some(path.to_path_buf()),
        cwd: PathBuf::from(cwd),
        folder,
        updated_at_ms,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: turns,
        user_turn_timestamps_ms: turn_timestamps,
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: m
            .and_then(|m| m.title.clone())
            .or_else(|| m.and_then(|m| m.preview.clone())),
        title_fixed: m.and_then(|m| m.title.as_ref()).is_some(),
        context_source,
    };
    finalize(&mut s);
    s.assistant_blob = build_assistant_blob(&assistant_per_turn);
    Some(s)
}

/// Extracts (user message, workspace, timestamp in ms) from step_type=14 payload.
fn extract_user_step(payload: &[u8]) -> Option<(String, Option<String>, i64)> {
    // User message: 19.2 (string)
    let f19 = pb_get_bytes(payload, 19)?;
    let msg_bytes = pb_get_bytes(f19, 2)?;
    let msg = String::from_utf8_lossy(msg_bytes).to_string();

    // Workspace: 19.12.12 (file:// URI)
    let workspace = pb_get_bytes(f19, 12)
        .and_then(|f19_12| pb_get_bytes(f19_12, 12))
        .map(|b| strip_file_uri(&String::from_utf8_lossy(b)));

    let ts_ms = step_timestamp_ms(payload).unwrap_or(0);

    Some((msg, workspace, ts_ms))
}

/// Timestamp: 5.1.1 (seconds) + 5.1.2 (nanos) -> ms.
fn step_timestamp_ms(payload: &[u8]) -> Option<i64> {
    pb_get_bytes(payload, 5)
        .and_then(|f5| pb_get_bytes(f5, 1))
        .map(|f5_1| {
            let secs = pb_get_varint(f5_1, 1).unwrap_or(0) as i64;
            let nanos = pb_get_varint(f5_1, 2).unwrap_or(0) as i64;
            secs * 1000 + nanos / 1_000_000
        })
}

/// Extracts answered `· question → answer` list from step_type=138 (ask_question) payload.
///
/// Field `154.1` block: `1` = question, repeated `2` = option (`.1` code, `.2` text),
/// repeated `4` = selected option code (missing if skipped). Returns None if no questions were answered.
/// Uses the same notation as `turn::extract_question_answers` (claude/codex) to align UI displays across agents.
fn extract_ask_answers(payload: &[u8]) -> Option<String> {
    let f154 = pb_get_bytes(payload, 154)?;
    let mut lines = Vec::new();

    for block in pb_get_bytes_all(f154, 1) {
        let question = match pb_get_bytes(block, 1) {
            Some(q) => String::from_utf8_lossy(q).to_string(),
            None => continue,
        };
        let selected: Vec<String> = pb_get_bytes_all(block, 4)
            .into_iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();
        if selected.is_empty() {
            continue; // Skipped question.
        }

        // Map option code to text. Fall back to the raw code if matching fails.
        let answers: Vec<String> = selected
            .iter()
            .map(|num| {
                pb_get_bytes_all(block, 2)
                    .into_iter()
                    .find(|opt| {
                        pb_get_bytes(opt, 1)
                            .map(|n| String::from_utf8_lossy(n) == num.as_str())
                            .unwrap_or(false)
                    })
                    .and_then(|opt| pb_get_bytes(opt, 2))
                    .map(|t| String::from_utf8_lossy(t).to_string())
                    .unwrap_or_else(|| num.clone())
            })
            .collect();
        lines.push(format!("· {} → {}", question, answers.join(", ")));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

// ---- Minimal Protobuf Reader ----

/// Decodes a varint. Returns (value, next offset).
fn read_varint(buf: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = *buf.get(i)?;
        i += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Returns the first length-delimited (wire type 2) field value as a slice from the message.
fn pb_get_bytes(buf: &[u8], field: u64) -> Option<&[u8]> {
    let mut i = 0;
    while i < buf.len() {
        let (tag, ni) = read_varint(buf, i)?;
        i = ni;
        let fno = tag >> 3;
        let wt = tag & 7;
        match wt {
            0 => {
                let (_, ni) = read_varint(buf, i)?;
                i = ni;
            }
            2 => {
                let (len, ni) = read_varint(buf, i)?;
                i = ni;
                let end = i + len as usize;
                let slice = buf.get(i..end)?;
                if fno == field {
                    return Some(slice);
                }
                i = end;
            }
            5 => i += 4,
            1 => i += 8,
            _ => return None,
        }
    }
    None
}

/// Returns all length-delimited (wire type 2) field values as slices from the message (for repeated fields).
fn pb_get_bytes_all(buf: &[u8], field: u64) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let Some((tag, ni)) = read_varint(buf, i) else {
            return out;
        };
        i = ni;
        let fno = tag >> 3;
        let wt = tag & 7;
        match wt {
            0 => {
                let Some((_, ni)) = read_varint(buf, i) else {
                    return out;
                };
                i = ni;
            }
            2 => {
                let Some((len, ni)) = read_varint(buf, i) else {
                    return out;
                };
                i = ni;
                let end = i + len as usize;
                let Some(slice) = buf.get(i..end) else {
                    return out;
                };
                if fno == field {
                    out.push(slice);
                }
                i = end;
            }
            5 => i += 4,
            1 => i += 8,
            _ => return out,
        }
    }
    out
}

/// Returns the first varint (wire type 0) field value from the message.
fn pb_get_varint(buf: &[u8], field: u64) -> Option<u64> {
    let mut i = 0;
    while i < buf.len() {
        let (tag, ni) = read_varint(buf, i)?;
        i = ni;
        let fno = tag >> 3;
        let wt = tag & 7;
        match wt {
            0 => {
                let (v, ni) = read_varint(buf, i)?;
                i = ni;
                if fno == field {
                    return Some(v);
                }
            }
            2 => {
                let (len, ni) = read_varint(buf, i)?;
                i = ni + len as usize;
            }
            5 => i += 4,
            1 => i += 8,
            _ => return None,
        }
    }
    None
}

// ---- Path Helpers ----

fn strip_file_uri(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

fn parse_pbtxt_title(data: &str) -> Option<String> {
    for line in data.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("title:\"") {
            return rest.strip_suffix('"').map(str::to_string);
        }
    }
    None
}

#[cfg(test)]
mod tests;
