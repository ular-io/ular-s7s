//! Rollout segments: one codex thread stored across several rollout files.
//!
//! Codex 0.159 stopped recording a rewind as a `thread_rolled_back` marker in
//! the same file (observed from the VS Code extension on 0.159.2 and 0.160.0).
//! It starts a new file for the same thread instead,
//! `rollout-<ts>-<thread id>_<segment id>.jsonl`, whose `session_meta` carries
//! `history_base { thread_id, end_ordinal_exclusive, end_byte_offset }`. The
//! thread's history is the earlier records with an `ordinal` below
//! `end_ordinal_exclusive`, followed by the new file; the new file's own
//! ordinals continue from that point. The earlier file keeps the abandoned
//! turns past the cut and is no longer written, and `threads.rollout_path` in
//! `state_*.sqlite` moves to the new file.
//!
//! [`read_rollout`] rebuilds that logical record stream so the list and the
//! context parsers see one thread as one session, and [`Segments`] tells the
//! scanner which files only hold history for a newer segment.

use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Guards the history walk against a cycle of malformed `history_base` links.
const MAX_DEPTH: usize = 64;

/// Thread id encoded in a rollout file name, and whether the name carries a
/// `_<segment id>` suffix. `rollout-<YYYY-MM-DDTHH-MM-SS>-<uuid>[_<uuid>].jsonl`;
/// None for any other shape.
pub(crate) fn thread_id_from_name(name: &str) -> Option<(&str, bool)> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let (head, segment) = match stem.split_once('_') {
        Some((head, _)) => (head, true),
        None => (stem, false),
    };
    // The uuid is the last five dash-separated groups.
    let start = head.rmatch_indices('-').nth(4).map(|(i, _)| i + 1)?;
    Some((&head[start..], segment))
}

/// Reads a rollout as its thread's logical record stream: the inherited
/// history (when the file is a segment) followed by the file itself. A missing
/// or unreadable earlier file leaves just the file's own records, which is what
/// s7s showed before segments were understood.
pub(crate) fn read_rollout(path: &Path) -> std::io::Result<String> {
    let own = std::fs::read_to_string(path)?;
    Ok(with_history(path, own, 0))
}

/// Every rollout file of `thread_id` in the store that holds `path`, oldest
/// first. Deletion uses this to reach the segments the listed file inherits.
pub(crate) fn thread_files(path: &Path, thread_id: &str) -> Vec<PathBuf> {
    store_root(path)
        .map(|root| thread_rollouts(root, thread_id))
        .unwrap_or_default()
}

fn with_history(path: &Path, own: String, depth: usize) -> String {
    let Some(base) = history_base(&own) else {
        return own;
    };
    if depth >= MAX_DEPTH {
        return own;
    }
    let Some(earlier) = predecessor(path, &base.thread_id) else {
        return own;
    };
    let Ok(earlier_own) = std::fs::read_to_string(&earlier) else {
        return own;
    };
    let earlier_full = with_history(&earlier, earlier_own, depth + 1);
    let mut out = inherited_prefix(&earlier_full, base.end_ordinal);
    out.push_str(&own);
    out
}

struct HistoryBase {
    thread_id: String,
    end_ordinal: u64,
}

/// `history_base` of the file's leading `session_meta`, if it has one.
fn history_base(content: &str) -> Option<HistoryBase> {
    let first = content.lines().find(|line| !line.trim().is_empty())?;
    let v: Value = serde_json::from_str(first).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    let base = v.get("payload")?.get("history_base")?;
    Some(HistoryBase {
        thread_id: base.get("thread_id")?.as_str()?.to_string(),
        end_ordinal: base.get("end_ordinal_exclusive")?.as_u64()?,
    })
}

/// The newest rollout of `thread_id` created before `path`. File names start
/// with the creation time, so name order is creation order.
fn predecessor(path: &Path, thread_id: &str) -> Option<PathBuf> {
    let name = path.file_name()?;
    thread_files(path, thread_id)
        .into_iter()
        .rfind(|p| p.file_name().is_some_and(|n| n < name))
}

/// The records of `content` below `end`, without `session_meta`: the newer
/// segment opens with its own, and the list parser takes the first one it
/// sees as the session's identity. A record without an `ordinal` takes its
/// position in the stream, which is what codex numbers from.
fn inherited_prefix(content: &str, end: u64) -> String {
    #[derive(Deserialize)]
    struct Head {
        #[serde(rename = "type")]
        kind: Option<String>,
        ordinal: Option<u64>,
    }
    let mut out = String::new();
    for (pos, line) in content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let Ok(head) = serde_json::from_str::<Head>(line) else {
            continue;
        };
        if head.ordinal.unwrap_or(pos as u64) >= end || head.kind.as_deref() == Some("session_meta")
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// The `sessions` (or `archived_sessions`) root above the date folders, else
/// the file's own folder.
fn store_root(path: &Path) -> Option<&Path> {
    path.ancestors()
        .skip(1)
        .find(|p| {
            matches!(
                p.file_name().and_then(|n| n.to_str()),
                Some("sessions" | "archived_sessions")
            )
        })
        .or_else(|| path.parent())
}

fn thread_rollouts(root: &Path, thread_id: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.file_name()
                .to_str()
                .and_then(thread_id_from_name)
                .is_some_and(|(id, _)| id == thread_id)
        })
        .map(walkdir::DirEntry::into_path)
        .collect();
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

/// Segment layout of one rollout store, computed once per scan.
#[derive(Default)]
pub(crate) struct Segments {
    /// Files that only hold history for a newer segment of the same thread.
    /// Listing them too would show the thread twice, the older entry with the
    /// turns the rewind abandoned.
    superseded: HashSet<PathBuf>,
    /// Newest segment -> the thread's earlier files, oldest first.
    earlier: HashMap<PathBuf, Vec<PathBuf>>,
}

impl Segments {
    pub(crate) fn scan(root: &Path) -> Self {
        let mut threads: HashMap<String, (Vec<PathBuf>, bool)> = HashMap::new();
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some((id, segment)) = entry.file_name().to_str().and_then(thread_id_from_name)
            else {
                continue;
            };
            let slot = threads.entry(id.to_string()).or_default();
            slot.0.push(entry.path().to_path_buf());
            slot.1 |= segment;
        }
        let mut out = Self::default();
        for (mut files, has_segment) in threads.into_values() {
            if !has_segment || files.len() < 2 {
                continue;
            }
            files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
            let newest = files.pop().expect("at least two files");
            out.superseded.extend(files.iter().cloned());
            out.earlier.insert(newest, files);
        }
        out
    }

    pub(crate) fn is_superseded(&self, path: &Path) -> bool {
        self.superseded.contains(path)
    }

    /// The thread's earlier files when `path` is its newest segment.
    pub(crate) fn earlier(&self, path: &Path) -> &[PathBuf] {
        self.earlier.get(path).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const THREAD: &str = "01a0fbba-11a3-7d03-87ce-b7b18f6de6cf";
    pub(crate) const ROOT_NAME: &str =
        "rollout-2026-10-02T17-27-50-01a0fbba-11a3-7d03-87ce-b7b18f6de6cf.jsonl";
    pub(crate) const SEGMENT_NAME: &str = "rollout-2026-10-02T18-00-42-01a0fbba-11a3-7d03-87ce-b7b18f6de6cf_01a0fbd8-2913-7282-ad3e-296c0b04ed03.jsonl";

    fn meta(ordinal: u64, base: Option<u64>) -> String {
        let base = base
            .map(|end| {
                format!(r#","history_base":{{"thread_id":"{THREAD}","end_ordinal_exclusive":{end},"end_byte_offset":0}}"#)
            })
            .unwrap_or_default();
        format!(
            r#"{{"ordinal":{ordinal},"type":"session_meta","payload":{{"id":"{THREAD}","cwd":"/tmp/demo"{base}}}}}"#
        )
    }

    fn user(ordinal: u64, text: &str) -> String {
        format!(
            r#"{{"ordinal":{ordinal},"type":"event_msg","payload":{{"type":"item_completed","item":{{"type":"UserMessage","content":[{{"type":"text","text":"{text}"}}]}}}}}}"#
        )
    }

    fn answer(ordinal: u64, text: &str) -> String {
        format!(
            r#"{{"ordinal":{ordinal},"type":"event_msg","payload":{{"type":"agent_message","message":"{text}"}}}}"#
        )
    }

    /// A rewound thread laid out the way codex 0.160 wrote it: the root file
    /// holds two kept turns (ordinals 0-4) and one abandoned turn (5-6); the
    /// segment inherits ordinals below 5 and continues with one new turn.
    pub(crate) fn write_rewound_thread(sessions: &Path) -> (PathBuf, PathBuf) {
        let day = sessions.join("2026/10/02");
        std::fs::create_dir_all(&day).expect("create day dir");
        let root = day.join(ROOT_NAME);
        let segment = day.join(SEGMENT_NAME);
        let root_lines = [
            meta(0, None),
            user(1, "질문1"),
            answer(2, "답1"),
            user(3, "질문2"),
            answer(4, "답2"),
            user(5, "버린 질문"),
            answer(6, "버린 답"),
        ];
        let segment_lines = [meta(5, Some(5)), user(6, "새 질문"), answer(7, "새 답")];
        std::fs::write(&root, root_lines.join("\n") + "\n").expect("write root");
        std::fs::write(&segment, segment_lines.join("\n") + "\n").expect("write segment");
        (root, segment)
    }

    pub(crate) fn temp_sessions(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "s7s-codex-segments-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        root.join("sessions")
    }

    #[test]
    fn thread_id_comes_from_the_name_with_or_without_a_segment() {
        assert_eq!(thread_id_from_name(ROOT_NAME), Some((THREAD, false)));
        assert_eq!(thread_id_from_name(SEGMENT_NAME), Some((THREAD, true)));
        assert_eq!(thread_id_from_name("rollout-thread-9.jsonl"), None);
        assert_eq!(thread_id_from_name("history.jsonl"), None);
    }

    #[test]
    fn segment_reads_inherited_history_then_its_own_records() {
        let sessions = temp_sessions("read");
        let (root, segment) = write_rewound_thread(&sessions);

        let logical = read_rollout(&segment).expect("read");
        assert!(logical.contains("질문1") && logical.contains("질문2"));
        assert!(logical.contains("새 질문"));
        // The abandoned turn past the cut stays in the root file only.
        assert!(!logical.contains("버린 질문"));
        // Only the segment's own session_meta remains.
        assert_eq!(logical.matches("session_meta").count(), 1);
        assert!(logical.find("질문2") < logical.find("새 질문"));

        // The root file alone still reads as itself.
        assert!(read_rollout(&root).expect("read").contains("버린 질문"));
        let _ = std::fs::remove_dir_all(sessions.parent().unwrap());
    }

    #[test]
    fn segment_without_its_earlier_file_reads_as_itself() {
        let sessions = temp_sessions("orphan");
        let (root, segment) = write_rewound_thread(&sessions);
        std::fs::remove_file(&root).expect("remove root");

        let logical = read_rollout(&segment).expect("read");
        assert!(logical.contains("새 질문"));
        assert!(!logical.contains("질문1"));
        let _ = std::fs::remove_dir_all(sessions.parent().unwrap());
    }

    #[test]
    fn scan_marks_every_file_but_the_newest_segment_superseded() {
        let sessions = temp_sessions("scan");
        let (root, segment) = write_rewound_thread(&sessions);
        let other = sessions.join(
            "2026/10/02/rollout-2026-10-02T09-00-00-01a0aaaa-0000-7000-8000-000000000000.jsonl",
        );
        std::fs::write(&other, meta(0, None) + "\n").expect("write other");

        let segments = Segments::scan(&sessions);
        assert!(segments.is_superseded(&root));
        assert!(!segments.is_superseded(&segment));
        assert!(!segments.is_superseded(&other));
        assert_eq!(segments.earlier(&segment), std::slice::from_ref(&root));
        assert!(segments.earlier(&other).is_empty());
        assert_eq!(thread_files(&segment, THREAD), [root, segment]);
        let _ = std::fs::remove_dir_all(sessions.parent().unwrap());
    }
}
