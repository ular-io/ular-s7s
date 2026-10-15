use super::*;

#[test]
fn apply_codex_title_meta_updates_cached_sessions_by_session_id() {
    let mut sessions = vec![Session {
        agent: Agent::Codex,
        profile_id: String::new(),
        id: "019f36e8-9157-7c63-bee8-8937a6314982".to_string(),
        source_path: None,
        cwd: "/tmp/demo".into(),
        folder: "demo".to_string(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["첫 질문".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: Some("첫 질문".to_string()),
        title_fixed: false,
        context_source: None,
    }];
    let mut meta = HashMap::new();
    meta.insert(
        "019f36e8-9157-7c63-bee8-8937a6314982".to_string(),
        parser::codex::TitleMeta {
            title: Some("26-07 세션 타이틀 개선".to_string()),
        },
    );

    apply_codex_title_meta(&mut sessions, &meta, &WorkspaceStore::default(), "p");

    assert_eq!(sessions[0].title(), "26-07 세션 타이틀 개선");
    assert!(sessions[0].title_fixed);
    // Enforce search matching by including renamed titles in the search blob, even if they are not in the body.
    assert!(sessions[0].search_blob.contains("타이틀"));
    assert!(sessions[0].search_blob.contains("첫 질문"));
}

#[test]
fn apply_claude_title_meta_fills_gap_but_never_overrides_body_title() {
    let mut meta = HashMap::new();
    meta.insert(
        "abc-123".to_string(),
        parser::claude::TitleMeta {
            title: Some("메타 제목".to_string()),
            fixed: true,
        },
    );
    let path = std::path::PathBuf::from("/tmp/x/abc-123.jsonl");

    // Cached body-derived title wins over the meta name.
    let mut sessions = vec![claude_session("abc-123", Some("본문 제목"))];
    apply_claude_title_meta(&path, &mut sessions, &meta, &WorkspaceStore::default(), "p");
    assert_eq!(sessions[0].title_hint.as_deref(), Some("본문 제목"));
    assert!(sessions[0].title_fixed);

    // Without a body title the meta name fills the gap (and enters the search blob).
    let mut sessions = vec![claude_session("abc-123", None)];
    apply_claude_title_meta(&path, &mut sessions, &meta, &WorkspaceStore::default(), "p");
    assert_eq!(sessions[0].title_hint.as_deref(), Some("메타 제목"));
    assert!(sessions[0].search_blob.contains("메타 제목"));
}

#[test]
fn claude_body_title_reaches_the_search_blob_without_a_registry_entry() {
    // claude 2.1.263 keys `sessions/` by process id, so a stored session
    // normally has no entry here. The blob must still be recomputed, or
    // `session search` cannot find a renamed session by its title.
    let meta: HashMap<String, parser::claude::TitleMeta> = HashMap::new();
    let path = std::path::PathBuf::from("/tmp/x/abc-123.jsonl");

    let mut sessions = vec![claude_session("abc-123", Some("HAND-OVER: 인계 항목"))];
    apply_claude_title_meta(&path, &mut sessions, &meta, &WorkspaceStore::default(), "p");

    assert_eq!(
        sessions[0].title_hint.as_deref(),
        Some("HAND-OVER: 인계 항목")
    );
    assert!(sessions[0].search_blob.contains("hand-over"));
    // The turn text is still indexed alongside the title.
    assert!(sessions[0].search_blob.contains("첫 질문"));
}

#[test]
fn scan_loads_claude_title_meta_from_profile_root() {
    // Meta lives at `<root>/sessions/*.json` while bodies live under
    // `<root>/projects/...` — regression for passing the projects dir to
    // load_title_meta (which made the meta fallback unreachable).
    let root = std::env::temp_dir().join(format!("s7s-scan-title-meta-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project_dir = root.join("projects/-tmp-app");
    std::fs::create_dir_all(&project_dir).expect("create projects");
    std::fs::create_dir_all(root.join("sessions")).expect("create sessions");
    std::fs::write(
        project_dir.join("abc-123.jsonl"),
        "{\"uuid\":\"u1\",\"parentUuid\":null,\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"첫 질문\"},\"cwd\":\"/tmp/app\"}\n",
    )
    .expect("write body");
    std::fs::write(
        root.join("sessions/abc-123.json"),
        r#"{"sessionId":"abc-123","name":"메타 제목","nameSource":"custom"}"#,
    )
    .expect("write meta");

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
    assert_eq!(result.sessions.len(), 1);
    assert_eq!(result.sessions[0].title(), "메타 제목");
    assert!(result.sessions[0].title_fixed);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn apply_antigravity_scan_meta_prefers_fixed_title_over_preview() {
    let mut sessions = vec![Session {
        agent: Agent::Antigravity,
        profile_id: String::new(),
        id: "8c456b4c-e7ba-46da-8c8a-9d37732e8e25".to_string(),
        source_path: None,
        cwd: "/tmp/demo".into(),
        folder: "demo".to_string(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["첫 질문".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: Some("List GitLab Repository Commands".to_string()),
        title_fixed: false,
        context_source: None,
    }];
    let mut meta = HashMap::new();
    meta.insert(
        "8c456b4c-e7ba-46da-8c8a-9d37732e8e25".to_string(),
        parser::antigravity::Meta {
            title: Some("26-07 컨테이너 레지스트리 이전".to_string()),
            preview: Some("List GitLab Repository Commands".to_string()),
            workspace: None,
        },
    );

    apply_antigravity_scan_meta(&mut sessions, &meta, &WorkspaceStore::default(), "agy-a");

    assert_eq!(sessions[0].title(), "26-07 컨테이너 레지스트리 이전");
    assert!(sessions[0].title_fixed);
}
