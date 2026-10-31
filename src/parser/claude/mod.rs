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
    let mut context_source: Option<crate::model::ContextSource> = None;
    let mut seen_real_turn = false;

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
                // The genuine launch envelope is always the leading user turn;
                // only capture before any real turn so a later message that merely
                // quotes a bootstrap (e.g. a meta-discussion) is not misread.
                if context_source.is_none() && !seen_real_turn {
                    if let Some(src) = u
                        .text
                        .as_deref()
                        .and_then(crate::parser::parse_context_bootstrap)
                    {
                        context_source = Some(src);
                    }
                }
                if !u.is_task_notification {
                    match u.text_kind {
                        UserTextKind::Turn { cleaned } => {
                            events.push(Event::User(cleaned, u.submitted_at_ms));
                            has_open_user_turn = true;
                            seen_real_turn = true;
                        }
                        // Slash commands, the bootstrap prompt, etc. close the current
                        // turn but do not open one.
                        UserTextKind::Boundary => {
                            events.push(Event::Boundary);
                            has_open_user_turn = false;
                        }
                        UserTextKind::Blank | UserTextKind::NoText => {}
                        // Neither a question nor a boundary; counted only
                        // when it is the session's sole user text.
                        UserTextKind::CompactSummary { cleaned } => {
                            if !seen_real_turn {
                                events.push(Event::User(cleaned, u.submitted_at_ms));
                                has_open_user_turn = true;
                                seen_real_turn = true;
                            }
                        }
                    }
                }
                // Task notifications produce no event (the real final answer
                // follows, so they keep the turn open); Q&A answers still count.
                if let Some(qa) = u.qa {
                    if !is_noise_turn(&qa) {
                        if let Some(cleaned) = clean_turn(&qa) {
                            events.push(Event::User(cleaned, u.submitted_at_ms));
                            has_open_user_turn = true;
                            seen_real_turn = true;
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
        context_source,
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
mod tests;
