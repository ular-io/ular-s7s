use super::*;

#[test]
fn loads_title_meta_from_parent_directory() {
    let root = std::env::temp_dir().join(format!(
        "ular-s7s-codex-meta-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let sessions_dir = root.join("sessions");
    std::fs::create_dir_all(&sessions_dir).expect("create temp dir");
    std::fs::write(sessions_dir.join("session_index.jsonl"), "{}\n")
        .expect("write empty child index");
    std::fs::write(
        root.join("session_index.jsonl"),
        r#"{"id":"019f36e8-9157-7c63-bee8-8937a6314982","thread_name":"26-07 세션 타이틀 개선"}"#,
    )
    .expect("write session index");

    let meta = load_title_meta(&sessions_dir);
    let title = meta
        .get("019f36e8-9157-7c63-bee8-8937a6314982")
        .and_then(|m| m.title.as_deref());
    assert_eq!(title, Some("26-07 세션 타이틀 개선"));

    let _ = std::fs::remove_file(root.join("session_index.jsonl"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn thread_name_column_wins_over_the_session_index() {
    let root = std::env::temp_dir().join(format!(
        "ular-s7s-codex-name-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp dir");
    // Two records for one id: codex appends on rename, so the last one wins.
    std::fs::write(
        root.join("session_index.jsonl"),
        "{\"id\":\"abc-123\",\"thread_name\":\"첫 기록\"}\n\
             {\"id\":\"abc-123\",\"thread_name\":\"index 기록\"}\n\
             {\"id\":\"only-index\",\"thread_name\":\"index 전용\"}\n",
    )
    .expect("write session index");

    let conn = rusqlite::Connection::open(root.join("state_5.sqlite")).expect("open sqlite");
    conn.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT NOT NULL, name TEXT)",
        [],
    )
    .expect("create table");
    conn.execute(
        "INSERT INTO threads (id, title, name) VALUES \
             ('abc-123', 'auto text', 'state 기록'), \
             ('no-name', 'auto text', NULL), \
             ('blank-name', 'auto text', '  ')",
        [],
    )
    .expect("insert threads");
    drop(conn);

    let meta = load_title_meta(&root);
    let title = |id: &str| meta.get(id).and_then(|m| m.title.as_deref());

    // The column codex itself displays overrides the index record.
    assert_eq!(title("abc-123"), Some("state 기록"));
    // A session only the index knows about keeps its index name.
    assert_eq!(title("only-index"), Some("index 전용"));
    // A NULL or blank name is not a title, and must not mask anything.
    assert_eq!(title("no-name"), None);
    assert_eq!(title("blank-name"), None);
    // `threads.title` is codex's auto text, never a title source here.
    assert!(!meta
        .values()
        .any(|m| m.title.as_deref() == Some("auto text")));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn state_db_paths_skips_the_observation_catalog() {
    let root = std::env::temp_dir().join(format!(
        "ular-s7s-codex-dbs-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("sqlite")).expect("create temp dir");
    for name in ["state_5.sqlite", "state_1.sqlite", "queue_1.sqlite"] {
        std::fs::write(root.join(name), "").expect("write file");
    }
    // `local_thread_catalog` lives here and holds no `threads` table.
    std::fs::write(root.join("sqlite/codex-dev.db"), "").expect("write catalog");

    let names: Vec<String> = state_db_paths(&root)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["state_1.sqlite", "state_5.sqlite"]);

    let _ = std::fs::remove_dir_all(&root);
}

fn write_rollout(name: &str, content: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "ular-s7s-codex-{}-{}",
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp dir");
    let path = root.join("rollout-2026-07-16T00-00-00-019f36e8-9157-7c63-bee8-000000000000.jsonl");
    std::fs::write(&path, content).expect("write rollout");
    (root, path)
}

#[test]
fn thread_rollback_drops_recent_turns() {
    let content = r#"
{"type":"session_meta","payload":{"id":"x1","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"event_msg","payload":{"type":"user_message","message":"버려질 질문"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"timestamp":"2026-07-23T03:04:05.678Z","type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
    let (root, path) = write_rollout("rollback-one", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["첫 질문", "수정된 질문"]);
    assert_eq!(
        session.user_turn_timestamps_ms,
        vec![Some(1_784_768_523_456), Some(1_784_775_845_678)]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parses_item_completed_rollout_without_counting_instruction_preamble() {
    // codex 0.147 line shape: the turn arrives as `item_completed`, and the
    // `response_item` role=user lines that mirror it (one of them the
    // prepended AGENTS.md preamble) must not add turns of their own.
    let content = r##"
{"type":"session_meta","payload":{"id":"items","cwd":"/tmp/demo"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /tmp/demo\n<INSTRUCTIONS>\n</INSTRUCTIONS>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"첫 질문"}]}}
{"timestamp":"2026-08-08T09:46:04.037Z","type":"event_msg","payload":{"type":"item_completed","item":{"type":"UserMessage","content":[{"type":"text","text":"첫 질문","text_elements":[]}]}}}
{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","content":[{"type":"Text","text":"답변 answerkw"}],"phase":"final_answer"}}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"답변 answerkw"}]}}
{"timestamp":"2026-08-08T09:50:00.000Z","type":"event_msg","payload":{"type":"task_complete"}}
"##;
    let (root, path) = write_rollout("item-completed", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["첫 질문"]);
    assert_eq!(
        session.user_turn_timestamps_ms,
        vec![Some(1_786_182_364_037)]
    );
    assert_eq!(session.updated_at_ms, 1_786_182_600_000);
    assert_eq!(session.assistant_blob.matches("answerkw").count(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn thread_rollback_can_drop_all_turns() {
    let content = r#"
{"type":"session_meta","payload":{"id":"x2","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"event_msg","payload":{"type":"user_message","message":"둘째 질문"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":2}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
    let (root, path) = write_rollout("rollback-all", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["수정된 질문"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn activity_time_uses_task_complete_and_ignores_later_storage_mtime() {
    let content = r#"
{"type":"session_meta","payload":{"id":"activity","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"question"}}
{"timestamp":"2026-07-23T01:03:03.456Z","type":"event_msg","payload":{"type":"agent_message","message":"answer"}}
{"timestamp":"2026-07-23T01:03:04.567Z","type":"event_msg","payload":{"type":"task_complete","completed_at":1784768584}}
{"timestamp":"2026-07-24T09:00:00Z","type":"event_msg","payload":{"type":"thread_settings_applied"}}
"#;
    let (root, path) = write_rollout("activity-complete", content);
    let later_mtime = 1_784_899_999_999;
    let session = parse_file(&path, later_mtime, None).expect("expected session");
    assert_eq!(session.updated_at_ms, 1_784_768_584_567);
    assert_ne!(session.updated_at_ms, later_mtime);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn activity_time_excludes_rolled_back_response_completion() {
    let content = r#"
{"type":"session_meta","payload":{"id":"activity-rollback","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"kept question"}}
{"timestamp":"2026-07-23T01:01:00Z","type":"event_msg","payload":{"type":"task_complete"}}
{"timestamp":"2026-07-23T02:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"abandoned question"}}
{"timestamp":"2026-07-23T05:00:00Z","type":"event_msg","payload":{"type":"task_complete"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"timestamp":"2026-07-23T03:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"replacement question"}}
"#;
    let (root, path) = write_rollout("activity-rollback", content);
    let session = parse_file(&path, 1_784_899_999_999, None).expect("expected session");
    assert_eq!(session.updated_at_ms, 1_784_775_600_000);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn thread_rollback_drops_question_answers_with_their_turn() {
    // The QA entry belongs to the turn being rolled back and must disappear with it.
    let content = r#"
{"type":"session_meta","payload":{"id":"x3","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
{"type":"response_item","payload":{"toolUseResult":{"questions":[{"question":"진행할까요?"}],"answers":{"진행할까요?":"네"}}}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
"#;
    let (root, path) = write_rollout("rollback-qa", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["수정된 질문"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn list_session_keeps_user_and_promoted_qa_submit_times() {
    let content = r#"
{"type":"session_meta","payload":{"id":"times","cwd":"/tmp/demo"}}
{"timestamp":"2026-07-23T01:02:03.456Z","type":"event_msg","payload":{"type":"user_message","message":"first question"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"response_item","payload":{"toolUseResult":{"questions":[{"question":"Continue?"}],"answers":{"Continue?":"Yes"}}}}
"#;
    let (root, path) = write_rollout("turn-times", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns.len(), 2);
    assert_eq!(
        session.user_turn_timestamps_ms,
        vec![Some(1_784_768_523_456), Some(1_784_772_184_567)]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parse_file_uses_session_id_from_payload_for_title_lookup() {
    let root = std::env::temp_dir().join(format!(
        "ular-s7s-codex-parse-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create temp dir");
    let path = root.join("rollout-2026-07-06T19-10-39-019f36e8-9157-7c63-bee8-8937a6314982.jsonl");
    std::fs::write(
        &path,
        r#"
{"type":"session_meta","payload":{"id":"019f36e8-9157-7c63-bee8-8937a6314982","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"첫 질문"}}
"#,
    )
    .expect("write rollout");
    let mut meta_map = HashMap::new();
    meta_map.insert(
        "019f36e8-9157-7c63-bee8-8937a6314982".to_string(),
        TitleMeta {
            title: Some("26-07 세션 타이틀 개선".to_string()),
        },
    );

    let session = parse_file(&path, 0, Some(&meta_map)).expect("expected session");
    assert_eq!(session.title(), "26-07 세션 타이틀 개선");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn assistant_blob_indexes_answers_and_excludes_rollback() {
    let content = r#"
{"type":"session_meta","payload":{"id":"x1","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"질문1"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"답1 keepkw"}}
{"type":"event_msg","payload":{"type":"user_message","message":"버려질 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"버려질답 dropkw"}}
{"type":"event_msg","payload":{"type":"thread_rolled_back","num_turns":1}}
{"type":"event_msg","payload":{"type":"user_message","message":"수정된 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"수정답 fixkw"}}
"#;
    let (root, path) = write_rollout("asst-rollback", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert!(session.assistant_blob.contains("keepkw"));
    assert!(session.assistant_blob.contains("fixkw"));
    assert!(!session.assistant_blob.contains("dropkw"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn assistant_blob_keeps_single_copy_of_duplicated_answer_event() {
    // The same answer arrives twice (agent_message then response_item); only the
    // turn's last assistant text is indexed, so the keyword appears once.
    let content = r#"
{"type":"session_meta","payload":{"id":"x2","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"동일답변 dupkw"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"동일답변 dupkw"}]}}
"#;
    let (root, path) = write_rollout("asst-dup", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.assistant_blob.matches("dupkw").count(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn assistant_blob_excludes_bootstrap_ready_response() {
    let content = r#"
{"type":"session_meta","payload":{"id":"x3","cwd":"/tmp/demo"}}
{"type":"event_msg","payload":{"type":"user_message","message":"<s7s-context-bootstrap>\nRun\n</s7s-context-bootstrap>"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"readykw 확인"}}
{"type":"event_msg","payload":{"type":"user_message","message":"진짜 질문"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"realkw 답"}}
"#;
    let (root, path) = write_rollout("asst-bootstrap", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["진짜 질문"]);
    assert!(session.assistant_blob.contains("realkw"));
    assert!(!session.assistant_blob.contains("readykw"));
    let _ = std::fs::remove_dir_all(&root);
}
