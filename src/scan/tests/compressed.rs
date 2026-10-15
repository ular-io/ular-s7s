use super::*;
use crate::parser::codex::{rollout, segments};
use crate::session_context::{ContextCompleteness, ContextEntryKind};
use crate::ui::test_support::TempBookmarkStore;
use serde_json::json;

const THREAD: &str = "00000000-0000-4000-8000-000000000001";
const NAME: &str = "rollout-2026-10-01T09-00-00-00000000-0000-4000-8000-000000000001.jsonl";
const SEGMENT: &str = "rollout-2026-10-02T09-00-00-00000000-0000-4000-8000-000000000001_00000000-0000-4000-8000-000000000002.jsonl";

fn profile(root: &Path) -> Profile {
    Profile {
        id: "compressed-test".into(),
        agent: Agent::Codex,
        name: "compressed-test".into(),
        path: root.to_path_buf(),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    }
}

fn write(path: &Path, records: &[serde_json::Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = records.iter().map(|v| format!("{v}\n")).collect::<String>();
    std::fs::write(path, text).unwrap();
}

fn compress(path: &Path, remove_plain: bool) -> PathBuf {
    let compressed = rollout::compressed_path(path);
    let input = std::fs::read(path).unwrap();
    std::fs::write(
        &compressed,
        zstd::stream::encode_all(input.as_slice(), 0).unwrap(),
    )
    .unwrap();
    if remove_plain {
        std::fs::remove_file(path).unwrap();
    }
    compressed
}

fn meta() -> serde_json::Value {
    json!({"ordinal":0,"type":"session_meta","payload":{"id":THREAD,"cwd":"/tmp/demo"}})
}

fn user(ordinal: u64, text: &str) -> serde_json::Value {
    json!({"ordinal":ordinal,"type":"event_msg","timestamp":"2026-10-01T09:00:00Z",
        "payload":{"type":"user_message","message":text}})
}

fn answer(ordinal: u64, text: &str) -> serde_json::Value {
    json!({"ordinal":ordinal,"type":"event_msg","timestamp":"2026-10-01T09:01:00Z",
        "payload":{"type":"agent_message","message":text}})
}

fn scan_fixture(root: &Path, rebuild: bool) -> ScanResult {
    scan_at(
        &[profile(root)],
        rebuild,
        &root.join("index.bin"),
        &root.join("workspaces.json"),
    )
}

#[test]
fn compressed_scan_cache_search_detail_and_rename_preserve_content() {
    let temp = TempBookmarkStore::new();
    let root = temp.path.parent().unwrap();
    let path = root.join("sessions/2026/10/01").join(NAME);
    write(
        &path,
        &[
            meta(),
            user(1, "Find the storage regression"),
            json!({"ordinal":2,"type":"response_item","payload":{"type":"function_call",
            "name":"inspect_storage","arguments":"{}","call_id":"call-1"}}),
            answer(3, "Compressed answer searchable"),
        ],
    );
    let compressed = compress(&path, true);
    let original = std::fs::read(&compressed).unwrap();
    let first = scan_fixture(root, true);
    assert_eq!(first.sessions.len(), 1);
    let session = &first.sessions[0];
    assert_eq!(session.id, THREAD);
    assert_eq!(session.source_path.as_deref(), Some(compressed.as_path()));
    assert_eq!(session.size_bytes, original.len() as u64);
    assert!(session
        .assistant_blob
        .contains("compressed answer searchable"));
    let filter = crate::filter::Filter {
        keyword: "searchable".into(),
        ..Default::default()
    };
    assert_eq!(crate::filter::apply(&first.sessions, &filter), [0]);
    let ctx = crate::session_context::load(session);
    assert_eq!(ctx.completeness, ContextCompleteness::Full);
    assert_eq!(ctx.turns.len(), session.user_turns.len());
    assert_eq!(
        ctx.turns[0].last_assistant_text.as_deref(),
        Some("Compressed answer searchable")
    );
    assert!(ctx.turns[0]
        .entries
        .iter()
        .any(|e| e.kind == ContextEntryKind::ToolCall));
    assert!(!path.exists());
    assert_eq!(std::fs::read(&compressed).unwrap(), original);

    crate::rename::rename_session(&profile(root), session, "Renamed compressed session").unwrap();
    let cached = scan_fixture(root, false);
    assert_eq!(cached.reparsed_files, 0);
    assert_eq!(cached.sessions[0].title(), "Renamed compressed session");
    assert!(cached.sessions[0]
        .search_blob
        .contains("renamed compressed session"));
    assert_eq!(std::fs::read(&compressed).unwrap(), original);
    assert_eq!(
        scan_fixture(root, true).sessions[0].title(),
        "Renamed compressed session"
    );
}

#[test]
fn transitions_between_variants_do_not_duplicate_or_hide_sessions() {
    let temp = TempBookmarkStore::new();
    let root = temp.path.parent().unwrap();
    let path = root.join("sessions/2026/10/01").join(NAME);
    // No session_meta: identity must still come from the compressed file name.
    write(
        &path,
        &[user(1, "Original question"), answer(2, "Original answer")],
    );
    let before = scan_fixture(root, true).sessions.remove(0);
    let compressed = compress(&path, true);
    let stale = crate::session_context::load(&before);
    assert_eq!(stale.completeness, ContextCompleteness::Full);
    assert_eq!(
        stale.turns[0].last_assistant_text.as_deref(),
        Some("Original answer")
    );
    let after = scan_fixture(root, false);
    assert_eq!(after.sessions.len(), 1);
    assert_eq!(after.sessions[0].id, THREAD);
    assert_eq!(after.sessions[0].updated_at_ms, before.updated_at_ms);

    write(
        &path,
        &[
            user(1, "Materialized question"),
            answer(2, "Materialized answer"),
        ],
    );
    let both = scan_fixture(root, false);
    assert_eq!(both.sessions.len(), 1);
    assert_eq!(
        both.sessions[0].source_path.as_deref(),
        Some(path.as_path())
    );
    assert_eq!(both.sessions[0].user_turns, ["Materialized question"]);
    std::fs::remove_file(&compressed).unwrap();
    let stale = crate::session_context::load(&after.sessions[0]);
    assert_eq!(stale.completeness, ContextCompleteness::Full);
    assert_eq!(
        stale.turns[0].last_assistant_text.as_deref(),
        Some("Materialized answer")
    );
    let plain = scan_fixture(root, false);
    assert_eq!(plain.sessions.len(), 1);
    assert_eq!(plain.reparsed_files, 0);
}

#[test]
fn compressed_rewind_history_is_folded_once_and_invalidates_cache() {
    for compress_segment in [false, true] {
        let temp = TempBookmarkStore::new();
        let root = temp.path.parent().unwrap();
        let first = root.join("sessions/2026/10/01").join(NAME);
        let segment = root.join("sessions/2026/10/02").join(SEGMENT);
        write(
            &first,
            &[
                meta(),
                user(1, "Kept question"),
                answer(2, "Kept answer"),
                user(3, "Abandoned question"),
                answer(4, "Abandoned answer"),
            ],
        );
        let mut base = meta();
        base["ordinal"] = json!(3);
        base["payload"]["history_base"] = json!({"thread_id":THREAD,"end_ordinal_exclusive":3});
        write(
            &segment,
            &[
                base,
                user(4, "Replacement question"),
                answer(5, "Replacement answer"),
            ],
        );
        let compressed_first = compress(&first, false);
        let segment = if compress_segment {
            compress(&segment, false)
        } else {
            segment
        };

        let scan = scan_fixture(root, true);
        assert_eq!(scan.sessions.len(), 1);
        assert_eq!(
            scan.sessions[0].user_turns,
            ["Kept question", "Replacement question"]
        );
        assert!(!scan.sessions[0].assistant_blob.contains("abandoned"));
        assert_eq!(
            scan.sessions[0].size_bytes,
            file_size_bytes(&first) + file_size_bytes(&rollout::plain_path(&segment))
        );
        let ctx = crate::session_context::load(&scan.sessions[0]);
        assert_eq!(ctx.completeness, ContextCompleteness::Full);
        assert_eq!(ctx.turns.len(), 2);

        std::fs::remove_file(&first).unwrap();
        let result = scan_fixture(root, false);
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].user_turns, scan.sessions[0].user_turns);
        assert!(segments::thread_files(&segment, THREAD).contains(&compressed_first));

        // A changed compressed predecessor must invalidate an unchanged segment.
        let old_mtime = file_mtime_ms(&compressed_first).unwrap();
        let old = rollout::read(&compressed_first).unwrap();
        let changed = old.replace("Kept question", "Updated predecessor");
        std::fs::write(
            &compressed_first,
            zstd::stream::encode_all(changed.as_bytes(), 0).unwrap(),
        )
        .unwrap();
        std::fs::File::open(&compressed_first)
            .unwrap()
            .set_modified(
                std::time::UNIX_EPOCH + std::time::Duration::from_millis(old_mtime as u64 + 5000),
            )
            .unwrap();
        let result = scan_fixture(root, false);
        assert_eq!(result.reparsed_files, 1);
        assert_eq!(
            result.sessions[0].user_turns,
            ["Updated predecessor", "Replacement question"]
        );
    }
}

#[test]
fn delete_reaches_compressed_segments_and_both_variants_after_a_transition() {
    let temp = TempBookmarkStore::new();
    let root = temp.path.parent().unwrap();
    let path = root.join("sessions/2026/10/01").join(NAME);
    write(&path, &[meta(), user(1, "Delete only this session")]);
    let stale = scan_fixture(root, true).sessions.remove(0);
    let compressed = compress(&path, true);
    let segment = root.join("sessions/2026/10/02").join(SEGMENT);
    write(&segment, &[meta(), user(1, "Delete segment too")]);
    let compressed_segment = compress(&segment, false);
    let other = root.join("sessions/2026/10/01/rollout-2026-10-01T10-00-00-00000000-0000-4000-8000-000000000003.jsonl.zst");
    std::fs::write(&other, "unrelated sentinel").unwrap();
    let files = crate::session_delete::transcript_files(&stale);
    for target in [&compressed, &segment, &compressed_segment] {
        assert!(files.contains(target));
    }
    let profiles = crate::profile::ProfileStore {
        profiles: vec![profile(root)],
    };
    crate::session_delete::delete_session_artifacts(&profiles, &stale).unwrap();
    for target in [&path, &compressed, &segment, &compressed_segment] {
        assert!(!target.exists());
    }
    assert_eq!(
        std::fs::read_to_string(other).unwrap(),
        "unrelated sentinel"
    );
}

#[test]
fn fork_reads_a_compressed_segment_by_storage_id_without_owning_its_files() {
    let temp = TempBookmarkStore::new();
    let root = temp.path.parent().unwrap();
    let first = root.join("sessions/2026/10/01").join(NAME);
    let segment = root.join("sessions/2026/10/02").join(SEGMENT);
    write(
        &first,
        &[
            meta(),
            user(1, "Shared original question"),
            answer(2, "Shared answer"),
        ],
    );
    let first = compress(&first, true);
    let mut segment_meta = meta();
    segment_meta["ordinal"] = json!(3);
    segment_meta["payload"]["history_base"] = json!({"thread_id":THREAD,"end_ordinal_exclusive":3});
    write(&segment, &[segment_meta]);
    let segment = compress(&segment, true);

    let fork_id = "00000000-0000-4000-8000-000000000004";
    let fork = root
        .join("sessions/2026/10/03")
        .join(format!("rollout-2026-10-03T09-00-00-{fork_id}.jsonl"));
    let mut fork_meta = meta();
    fork_meta["ordinal"] = json!(4);
    fork_meta["payload"]["id"] = json!(fork_id);
    fork_meta["payload"]["history_base"] =
        json!({"thread_id":"00000000-0000-4000-8000-000000000002","end_ordinal_exclusive":4});
    write(&fork, &[fork_meta]);
    let fork = compress(&fork, true);
    let scan = scan_fixture(root, true);
    assert_eq!(scan.sessions.len(), 2);
    let fork_session = scan.sessions.iter().find(|s| s.id == fork_id).unwrap();
    assert_eq!(fork_session.user_turns, ["Shared original question"]);
    let ctx = crate::session_context::load(fork_session);
    assert_eq!(ctx.completeness, ContextCompleteness::Full);
    assert_eq!(
        ctx.turns[0].last_assistant_text.as_deref(),
        Some("Shared answer")
    );
    assert_eq!(
        crate::session_delete::transcript_files(fork_session),
        std::slice::from_ref(&fork)
    );

    let profiles = crate::profile::ProfileStore {
        profiles: vec![profile(root)],
    };
    crate::session_delete::delete_session_artifacts(&profiles, fork_session).unwrap();
    assert!(!fork.exists());
    assert!(first.exists() && segment.exists());
    let surviving = scan_fixture(root, false);
    assert_eq!(surviving.sessions.len(), 1);
    assert_eq!(surviving.sessions[0].id, THREAD);
    assert_eq!(
        surviving.sessions[0].user_turns,
        ["Shared original question"]
    );
}
