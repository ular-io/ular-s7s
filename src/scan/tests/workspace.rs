use super::*;

#[test]
fn workspace_override_replaces_a_stored_folder_for_claude_and_codex() {
    let root = std::env::temp_dir().join(format!(
        "s7s-scan-folder-override-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let workspace_path = root.join("session_workspaces.json");
    crate::session_workspace::record(
        &workspace_path,
        "p",
        "changed",
        Path::new("/tmp/chosen-project"),
    )
    .expect("record override");
    let workspaces = WorkspaceStore::load(&workspace_path);

    // Claude and Codex both resume where s7s launches them, so the recorded
    // folder is the real one and wins over the transcript's own value.
    let mut claude = claude_session("changed", Some("claude"));
    claude.cwd = PathBuf::from("/tmp/wrong-project");
    claude.folder = "wrong-project".to_string();
    let mut untouched = claude_session("untouched", Some("claude"));
    untouched.cwd = PathBuf::from("/tmp/original-project");
    untouched.folder = "original-project".to_string();
    let mut sessions = vec![claude, untouched];
    apply_claude_title_meta(
        Path::new("/tmp/changed.jsonl"),
        &mut sessions,
        &HashMap::new(),
        &workspaces,
        "p",
    );
    assert_eq!(sessions[0].cwd, PathBuf::from("/tmp/chosen-project"));
    assert_eq!(sessions[0].folder, "chosen-project");
    assert!(sessions[0].search_blob.contains("chosen-project"));
    assert_eq!(sessions[1].folder, "original-project");

    let mut codex = claude_session("changed", Some("codex"));
    codex.agent = Agent::Codex;
    codex.cwd = PathBuf::from("/tmp/wrong-project");
    codex.folder = "wrong-project".to_string();
    let mut sessions = vec![codex];
    apply_codex_title_meta(&mut sessions, &HashMap::new(), &workspaces, "p");
    assert_eq!(sessions[0].folder, "chosen-project");

    // Antigravity keeps its own value: a resumed agy conversation works in
    // the folder it was created in, whatever folder it is launched from.
    let mut agy = claude_session("changed", Some("agy"));
    agy.agent = Agent::Antigravity;
    agy.cwd = PathBuf::from("/tmp/agy-project");
    agy.folder = "agy-project".to_string();
    let mut sessions = vec![agy];
    apply_antigravity_scan_meta(&mut sessions, &HashMap::new(), &workspaces, "p");
    assert_eq!(sessions[0].folder, "agy-project");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn antigravity_handoff_workspace_fills_only_a_missing_cwd_and_reindexes() {
    let root =
        std::env::temp_dir().join(format!("s7s-scan-handoff-workspace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let workspace_path = root.join("session_workspaces.json");
    crate::session_workspace::record(
        &workspace_path,
        "agy-a",
        "missing-cwd",
        Path::new("/tmp/target-project"),
    )
    .expect("record workspace");
    crate::session_workspace::record(
        &workspace_path,
        "agy-a",
        "existing-cwd",
        Path::new("/tmp/wrong-project"),
    )
    .expect("record fallback that must not win");
    let workspaces = WorkspaceStore::load(&workspace_path);
    let meta = HashMap::new();
    let mut missing = claude_session("missing-cwd", Some("handoff"));
    missing.agent = Agent::Antigravity;
    missing.cwd = PathBuf::new();
    missing.folder.clear();
    let mut existing = claude_session("existing-cwd", Some("interactive"));
    existing.agent = Agent::Antigravity;
    existing.cwd = PathBuf::from("/tmp/original-project");
    existing.folder = "original-project".to_string();
    let mut sessions = vec![missing, existing];

    apply_antigravity_scan_meta(&mut sessions, &meta, &workspaces, "agy-a");

    assert_eq!(sessions[0].cwd, PathBuf::from("/tmp/target-project"));
    assert_eq!(sessions[0].folder, "target-project");
    assert!(sessions[0].search_blob.contains("target-project"));
    assert_eq!(sessions[1].cwd, PathBuf::from("/tmp/original-project"));
    assert_eq!(sessions[1].folder, "original-project");
    assert!(!sessions[1].search_blob.contains("wrong-project"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn antigravity_handoff_workspace_survives_reparse_and_cache_hit() {
    let root = std::env::temp_dir().join(format!("s7s-scan-handoff-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let profile_root = root.join("agy");
    let conversations = profile_root.join("conversations");
    std::fs::create_dir_all(&conversations).expect("create conversations");
    let db_path = conversations.join("agy-handoff.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open fixture db");
    conn.execute(
        "CREATE TABLE steps (idx INTEGER, step_type INTEGER, step_payload BLOB)",
        [],
    )
    .expect("create steps");
    // Minimal protobuf: top-level field 19 contains field 2 user text, with
    // no field 12 workspace payload (the shape emitted by `agy --print`).
    let message = b"park this handoff";
    let mut field_19 = vec![0x12, message.len() as u8];
    field_19.extend_from_slice(message);
    let mut payload = vec![0x9a, 0x01, field_19.len() as u8];
    payload.extend_from_slice(&field_19);
    conn.execute(
        "INSERT INTO steps (idx, step_type, step_payload) VALUES (1, 14, ?1)",
        [&payload],
    )
    .expect("insert user step");
    drop(conn);

    let profile = crate::profile::Profile {
        id: "agy-a".to_string(),
        agent: Agent::Antigravity,
        name: "agy-a".to_string(),
        path: profile_root,
        oauth_token: None,
        active: true,
        shortcut: None,
        builtin: false,
    };
    let cache_path = root.join("index.bin");
    let workspace_path = root.join("session_workspaces.json");
    crate::session_workspace::record(
        &workspace_path,
        &profile.id,
        "agy-handoff",
        Path::new("/tmp/target-project"),
    )
    .expect("record workspace");

    let reparsed = scan_at(
        std::slice::from_ref(&profile),
        true,
        &cache_path,
        &workspace_path,
    );
    assert_eq!(reparsed.reparsed_files, 1);
    assert_eq!(reparsed.sessions.len(), 1);
    assert_eq!(reparsed.sessions[0].folder, "target-project");
    assert!(reparsed.sessions[0].search_blob.contains("target-project"));

    let cached = scan_at(&[profile], false, &cache_path, &workspace_path);
    assert_eq!(cached.reparsed_files, 0);
    assert_eq!(cached.sessions.len(), 1);
    assert_eq!(cached.sessions[0].cwd, PathBuf::from("/tmp/target-project"));
    assert_eq!(cached.sessions[0].folder, "target-project");

    let _ = std::fs::remove_dir_all(root);
}
