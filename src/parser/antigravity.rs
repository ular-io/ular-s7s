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

use super::{build_assistant_blob, clean_turn, finalize, is_noise_turn};
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
    pub updated_ms: Option<i64>,
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
        let updated_ms = entry
            .get("last_modified_time")
            .and_then(Value::as_str)
            .and_then(rfc3339_to_ms)
            .or_else(|| {
                summary
                    .and_then(|s| s.get("UpdatedAt"))
                    .and_then(Value::as_str)
                    .and_then(rfc3339_to_ms)
            });
        out.insert(
            id.clone(),
            Meta {
                title,
                preview,
                workspace,
                updated_ms,
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
                    updated_ms: None,
                });
        }
    }
    out
}

/// Parses a single conversation `.db`. Returns None if there are no valid user turns and fallback via Preview/Title is not possible.
pub fn parse_db(path: &Path, mtime_ms: i64, meta: &HashMap<String, Meta>) -> Option<Session> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let m = meta.get(&id);

    let mut turns: Vec<String> = Vec::new();
    let mut cwd: Option<String> = None;
    let mut max_ts_ms: i64 = 0;

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
                        if ts > max_ts_ms {
                            max_ts_ms = ts;
                        }
                        if !is_noise_turn(&msg) {
                            if let Some(cleaned) = clean_turn(&msg) {
                                turns.push(cleaned);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback to workspace/mtime from metadata.
    if cwd.is_none() {
        cwd = m.and_then(|m| m.workspace.clone());
    }
    let mtime = [
        max_ts_ms,
        m.and_then(|m| m.updated_ms).unwrap_or(0),
        mtime_ms,
    ]
    .into_iter()
    .max()
    .unwrap_or(mtime_ms);

    // Fall back to Preview if no user turns were extracted (still displays the session).
    if turns.is_empty() {
        if let Some(preview) = m.and_then(|m| m.preview.clone()) {
            if let Some(cleaned) = clean_turn(&preview) {
                turns.push(cleaned);
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
    let assistant_per_turn: Vec<String> =
        crate::session_context::antigravity::transcript_path(path, &id)
            .and_then(|tp| crate::session_context::antigravity::parse_turns(&tp).ok())
            .map(|turns| {
                turns
                    .into_iter()
                    .filter_map(|t| t.last_assistant_text)
                    .collect()
            })
            .unwrap_or_default();

    let mut s = Session {
        agent: Agent::Antigravity,
        profile_id: String::new(),
        id,
        source_path: Some(path.to_path_buf()),
        cwd: PathBuf::from(cwd),
        folder,
        mtime_ms: mtime,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: turns,
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: m
            .and_then(|m| m.title.clone())
            .or_else(|| m.and_then(|m| m.preview.clone())),
        title_fixed: m.and_then(|m| m.title.as_ref()).is_some(),
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

    // Timestamp: 5.1.1 (seconds) + 5.1.2 (nanos) -> ms
    let ts_ms = pb_get_bytes(payload, 5)
        .and_then(|f5| pb_get_bytes(f5, 1))
        .map(|f5_1| {
            let secs = pb_get_varint(f5_1, 1).unwrap_or(0) as i64;
            let nanos = pb_get_varint(f5_1, 2).unwrap_or(0) as i64;
            secs * 1000 + nanos / 1_000_000
        })
        .unwrap_or(0);

    Some((msg, workspace, ts_ms))
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

// ---- Time/Path Helpers ----

fn strip_file_uri(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

/// RFC3339 -> epoch milliseconds (handles offset, Z, and fractional seconds).
fn rfc3339_to_ms(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;

    let mut idx = 19usize;
    let mut millis = 0i64;
    if bytes.get(idx) == Some(&b'.') {
        idx += 1;
        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        let frac = &s[start..idx];
        let ms_str: String = frac.chars().take(3).collect();
        millis = format!("{:0<3}", ms_str).parse().unwrap_or(0);
    }

    let mut offset_secs = 0i64;
    if idx < bytes.len() {
        match bytes[idx] {
            b'Z' => {}
            b'+' | b'-' => {
                let sign = if bytes[idx] == b'-' { -1 } else { 1 };
                let digits: String = s[idx + 1..]
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect();
                if digits.len() >= 4 {
                    let oh: i64 = digits[0..2].parse().ok()?;
                    let om: i64 = digits[2..4].parse().ok()?;
                    offset_secs = sign * (oh * 3600 + om * 60);
                }
            }
            _ => {}
        }
    }

    let days = days_from_civil(year, month as u32, day as u32);
    let epoch_secs = days * 86400 + hour * 3600 + min * 60 + sec - offset_secs;
    Some(epoch_secs * 1000 + millis)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
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
mod tests {
    use super::*;

    /// Protobuf encoder helper for tests: length-delimited (wire type 2) field.
    fn pb_len_field(field: u64, bytes: &[u8]) -> Vec<u8> {
        fn varint(mut v: u64, out: &mut Vec<u8>) {
            loop {
                let byte = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(byte);
                    break;
                }
                out.push(byte | 0x80);
            }
        }
        let mut out = Vec::new();
        varint(field << 3 | 2, &mut out);
        varint(bytes.len() as u64, &mut out);
        out.extend_from_slice(bytes);
        out
    }

    /// Encodes a 154.1 question block: question + 2 options + (optional) selected code.
    fn ask_payload(question: &str, options: [&str; 2], selected: Option<&str>) -> Vec<u8> {
        let mut block = pb_len_field(1, question.as_bytes());
        for (i, opt) in options.iter().enumerate() {
            let mut o = pb_len_field(1, (i + 1).to_string().as_bytes());
            o.extend(pb_len_field(2, opt.as_bytes()));
            block.extend(pb_len_field(2, &o));
        }
        if let Some(sel) = selected {
            block.extend(pb_len_field(4, sel.as_bytes()));
        }
        pb_len_field(154, &pb_len_field(1, &block))
    }

    #[test]
    fn extracts_answered_ask_question() {
        let payload = ask_payload(
            "원하시는 시안을 선택해 주세요.",
            ["V1. 왼쪽 시안", "V2. 오른쪽 시안"],
            Some("2"),
        );

        assert_eq!(
            extract_ask_answers(&payload).as_deref(),
            Some("· 원하시는 시안을 선택해 주세요. → V2. 오른쪽 시안")
        );
    }

    #[test]
    fn skipped_ask_question_returns_none() {
        let payload = ask_payload(
            "원하시는 시안을 선택해 주세요.",
            ["V1. 왼쪽 시안", "V2. 오른쪽 시안"],
            None,
        );

        assert_eq!(extract_ask_answers(&payload), None);
    }

    #[test]
    fn loads_title_from_conversation_metadata() {
        let root = std::env::temp_dir().join(format!(
            "s7s-antigravity-meta-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let cache_dir = root.join("cache");
        std::fs::create_dir_all(&cache_dir).expect("create temp dir");
        std::fs::write(
            cache_dir.join("conversation_metadata.json"),
            r#"{
  "conversations": {
    "8c456b4c-e7ba-46da-8c8a-9d37732e8e25": {
      "summary": {
        "ID": "8c456b4c-e7ba-46da-8c8a-9d37732e8e25",
        "Title": "26-07 컨테이너 레지스트리 이전",
        "Preview": "List GitLab Repository Commands",
        "WorkspaceURIs": ["file:///Users/username/DevSpace/v2s/1.common/gitops"]
      },
      "last_modified_time": "2026-07-03T16:34:08Z"
    }
  }
}"#,
        )
        .expect("write metadata");

        let meta = load_metadata(&root);
        let entry = meta
            .get("8c456b4c-e7ba-46da-8c8a-9d37732e8e25")
            .expect("meta");
        assert_eq!(
            entry.title.as_deref(),
            Some("26-07 컨테이너 레지스트리 이전")
        );
        assert_eq!(
            entry.preview.as_deref(),
            Some("List GitLab Repository Commands")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn loads_title_from_annotation_pbtxt() {
        let root = std::env::temp_dir().join(format!(
            "ular-s7s-antigravity-anno-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let cache_dir = root.join("cache");
        let anno_dir = root.join("annotations");
        std::fs::create_dir_all(&cache_dir).expect("create temp dir");
        std::fs::create_dir_all(&anno_dir).expect("create anno dir");
        std::fs::write(
            cache_dir.join("conversation_metadata.json"),
            r#"{
  "conversations": {
    "ade7c4a5-2fe7-4c2e-a99f-941ab2c8bf38": {
      "summary": {
        "ID": "ade7c4a5-2fe7-4c2e-a99f-941ab2c8bf38",
        "Title": "",
        "Preview": "글로벌 지침 파일 경로"
      }
    }
  }
}"#,
        )
        .expect("write metadata");
        std::fs::write(
            anno_dir.join("ade7c4a5-2fe7-4c2e-a99f-941ab2c8bf38.pbtxt"),
            "title:\"26-07 agy 전용 지침 추가\"",
        )
        .expect("write annotation");

        let meta = load_metadata(&root);
        let entry = meta
            .get("ade7c4a5-2fe7-4c2e-a99f-941ab2c8bf38")
            .expect("meta");
        assert_eq!(entry.title.as_deref(), Some("26-07 agy 전용 지침 추가"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
