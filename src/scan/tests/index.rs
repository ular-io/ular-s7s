use super::*;

/// A rewound codex thread spans two rollout files but is one session: the
/// newest segment is listed with the whole thread, the earlier file is not.
#[test]
fn scan_lists_a_rewound_codex_thread_once() {
    use crate::parser::codex::segments::tests::{temp_sessions, write_rewound_thread, THREAD};
    let sessions_dir = temp_sessions("scan-once");
    let root = sessions_dir.parent().unwrap().to_path_buf();
    let (first, segment) = write_rewound_thread(&sessions_dir);

    let profiles = vec![crate::profile::Profile {
        id: "t".to_string(),
        agent: Agent::Codex,
        name: "t".to_string(),
        path: root.clone(),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    }];
    let result = scan_at(
        &profiles,
        true,
        &root.join("index.bin"),
        &root.join("session_workspaces.json"),
    );
    assert_eq!(result.sessions.len(), 1);
    let session = &result.sessions[0];
    assert_eq!(session.id, THREAD);
    assert_eq!(session.source_path.as_deref(), Some(segment.as_path()));
    assert_eq!(session.user_turns, ["질문1", "질문2", "새 질문"]);
    assert_eq!(
        session.size_bytes,
        file_size_bytes(&first) + file_size_bytes(&segment)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn scan_sorts_by_session_activity_instead_of_storage_mtime() {
    let root = std::env::temp_dir().join(format!("s7s-scan-activity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project_dir = root.join("projects/-tmp-app");
    std::fs::create_dir_all(&project_dir).expect("create projects");
    let older_activity = project_dir.join("older-activity.jsonl");
    let newer_activity = project_dir.join("newer-activity.jsonl");
    std::fs::write(
        &older_activity,
        "{\"timestamp\":\"2026-07-22T01:00:00Z\",\"uuid\":\"a\",\"parentUuid\":null,\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"older\"},\"cwd\":\"/tmp/app\"}\n",
    )
    .expect("write older activity");
    std::fs::write(
        &newer_activity,
        "{\"timestamp\":\"2026-07-23T01:00:00Z\",\"uuid\":\"b\",\"parentUuid\":null,\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"newer\"},\"cwd\":\"/tmp/app\"}\n",
    )
    .expect("write newer activity");

    let base = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_784_800_000);
    std::fs::File::open(&older_activity)
        .expect("open older activity")
        .set_modified(base + std::time::Duration::from_secs(100))
        .expect("make older activity file physically newer");
    std::fs::File::open(&newer_activity)
        .expect("open newer activity")
        .set_modified(base)
        .expect("make newer activity file physically older");

    let profiles = vec![crate::profile::Profile {
        id: "t".to_string(),
        agent: Agent::Claude,
        name: "t".to_string(),
        path: root.clone(),
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    }];
    let result = scan_at(
        &profiles,
        true,
        &root.join("index.bin"),
        &root.join("session_workspaces.json"),
    );
    assert_eq!(result.sessions.len(), 2);
    assert_eq!(result.sessions[0].id, "newer-activity");
    let first_source_mtime = file_mtime_ms(
        result.sessions[0]
            .source_path
            .as_deref()
            .expect("first source path"),
    )
    .expect("first source mtime");
    let second_source_mtime = file_mtime_ms(
        result.sessions[1]
            .source_path
            .as_deref()
            .expect("second source path"),
    )
    .expect("second source mtime");
    assert!(first_source_mtime < second_source_mtime);
    assert!(result.sessions[0].updated_at_ms > result.sessions[1].updated_at_ms);

    let _ = std::fs::remove_dir_all(&root);
}

/// Manual before/after gate for parser refactoring (§11.4): dumps one line per
/// session of the machine's real index — order, identity, title, Q count, and
/// blob hashes — so two runs (baseline vs. refactored) can be diffed exactly.
/// Uses a throwaway cache path with a forced rebuild, so every session is fully
/// reparsed by the current code and the user's real cache is never touched.
/// Run explicitly with:
/// `cargo test real_data_index_snapshot -- --ignored --nocapture | grep '^S7S-IDX'`
#[test]
#[ignore]
fn real_data_index_snapshot() {
    use std::hash::{Hash, Hasher};
    fn h(s: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    let profiles = crate::profile::ProfileStore::load();
    let cache = std::env::temp_dir().join(format!("s7s-idx-snap-{}.bin", std::process::id()));
    let result = scan_at(
        &profiles.profiles,
        true,
        &cache,
        &crate::config::session_workspaces_path(),
    );
    let _ = std::fs::remove_file(&cache);

    // Result order is meaningful (semantic activity desc): dump in order, no sorting.
    for s in &result.sessions {
        println!(
            "S7S-IDX\t{}\t{}\t{}\t{}\t{}\t{}\tQ{}\tsb{:016x}\tab{:016x}\t{}",
            s.agent.key(),
            s.profile_id,
            s.id,
            s.updated_at_ms,
            s.title(),
            s.title_fixed,
            s.user_turns.len(),
            h(&s.search_blob),
            h(&s.assistant_blob),
            s.folder,
        );
    }
    println!("S7S-IDX-TOTAL\t{}", result.sessions.len());
}
