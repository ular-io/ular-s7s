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

fn pb_varint_field(field: u64, value: u64) -> Vec<u8> {
    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
    }
    let mut out = Vec::new();
    varint(field << 3, &mut out);
    varint(value, &mut out);
    out
}

fn timestamp_payload(timestamp_ms: i64) -> Vec<u8> {
    let mut timestamp = pb_varint_field(1, (timestamp_ms / 1000) as u64);
    timestamp.extend(pb_varint_field(
        2,
        ((timestamp_ms % 1000) * 1_000_000) as u64,
    ));
    pb_len_field(5, &pb_len_field(1, &timestamp))
}

fn user_payload(message: &str, timestamp_ms: i64) -> Vec<u8> {
    let mut payload = pb_len_field(19, &pb_len_field(2, message.as_bytes()));
    payload.extend(timestamp_payload(timestamp_ms));
    payload
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
fn parse_db_keeps_indexable_turn_timestamps() {
    let path = std::env::temp_dir().join(format!(
        "s7s-antigravity-turn-times-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let conn = rusqlite::Connection::open(&path).expect("open temp DB");
    conn.execute(
        "CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, step_payload BLOB)",
        [],
    )
    .expect("create steps");
    conn.execute(
        "INSERT INTO steps VALUES (0, 14, ?1)",
        [user_payload("first question", 1_784_768_523_456)],
    )
    .expect("insert user");
    conn.execute(
        "INSERT INTO steps VALUES (1, 14, ?1)",
        [user_payload("/usage", 1_784_769_000_000)],
    )
    .expect("insert noise");
    let mut qa = ask_payload("Continue?", ["Yes", "No"], Some("1"));
    qa.extend(timestamp_payload(1_784_772_184_567));
    conn.execute("INSERT INTO steps VALUES (2, 138, ?1)", [qa])
        .expect("insert QA");
    drop(conn);

    let session = parse_db(&path, 0, &HashMap::new()).expect("parse session");
    assert_eq!(session.user_turns.len(), 2);
    assert_eq!(
        session.user_turn_timestamps_ms,
        vec![Some(1_784_768_523_456), Some(1_784_772_184_567)]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn parse_db_uses_transcript_response_time_instead_of_later_storage_mtime() {
    let root = std::env::temp_dir().join(format!(
        "s7s-antigravity-activity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let id = "activity-session";
    let conversations = root.join("conversations");
    let logs = root.join(format!("brain/{id}/.system_generated/logs"));
    std::fs::create_dir_all(&conversations).expect("create conversations");
    std::fs::create_dir_all(&logs).expect("create logs");
    let path = conversations.join(format!("{id}.db"));
    let conn = rusqlite::Connection::open(&path).expect("open temp DB");
    conn.execute(
        "CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, step_payload BLOB)",
        [],
    )
    .expect("create steps");
    conn.execute(
        "INSERT INTO steps VALUES (0, 14, ?1)",
        [user_payload("question", 1_784_768_523_456)],
    )
    .expect("insert user");
    conn.execute("INSERT INTO steps VALUES (1, 15, X'')", [])
        .expect("insert response step");
    drop(conn);
    std::fs::write(
        logs.join("transcript_full.jsonl"),
        r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-07-23T01:02:03Z","content":"<USER_REQUEST>question</USER_REQUEST>"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-07-23T01:03:04Z","content":"answer"}
"#,
    )
    .expect("write transcript");

    let later_mtime = 1_784_899_999_999;
    let session = parse_db(&path, later_mtime, &HashMap::new()).expect("parse session");
    assert_eq!(session.updated_at_ms, 1_784_768_584_000);
    assert_ne!(session.updated_at_ms, later_mtime);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rewind_keeps_db_detail_and_assistant_search_on_the_active_branch() {
    let root = std::env::temp_dir().join(format!(
        "s7s-antigravity-rewind-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let id = "rewind-session";
    let conversations = root.join("conversations");
    let logs = root.join(format!("brain/{id}/.system_generated/logs"));
    std::fs::create_dir_all(&conversations).expect("create conversations");
    std::fs::create_dir_all(&logs).expect("create logs");
    let path = conversations.join(format!("{id}.db"));
    let conn = rusqlite::Connection::open(&path).expect("open temp DB");
    conn.execute(
        "CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, step_payload BLOB)",
        [],
    )
    .expect("create steps");
    for (idx, prompt) in [(0, "first"), (2, "replacement")] {
        conn.execute(
            "INSERT INTO steps VALUES (?1, 14, ?2)",
            rusqlite::params![idx, user_payload(prompt, 1_784_768_523_456)],
        )
        .expect("insert user");
    }
    conn.execute("INSERT INTO steps VALUES (3, 15, X'')", [])
        .expect("insert response step");
    drop(conn);
    std::fs::write(
        logs.join("transcript_full.jsonl"),
        r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"first"}
{"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T01:03:04Z","content":"retained answer"}
{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"abandoned prompt"}
{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T03:00:00Z","content":"abandoned answer"}
{"step_index":4,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"abandoned later prompt"}
{"step_index":5,"source":"MODEL","type":"RUN_COMMAND","content":"abandoned work"}
{"step_index":2,"source":"USER_EXPLICIT","type":"USER_INPUT","content":"replacement"}
{"step_index":3,"source":"MODEL","type":"PLANNER_RESPONSE","created_at":"2026-07-23T02:00:00Z","content":"replacement answer"}
"#,
    )
    .expect("write transcript");

    let session = parse_db(&path, 0, &HashMap::new()).expect("parse session");
    assert_eq!(session.user_turns, ["first", "replacement"]);
    assert!(session.assistant_blob.contains("retained answer"));
    assert!(session.assistant_blob.contains("replacement answer"));
    assert!(!session.assistant_blob.contains("abandoned"));
    assert_eq!(session.updated_at_ms, 1_784_772_000_000);
    let context = crate::session_context::load(&session);
    assert_eq!(
        context.completeness,
        crate::session_context::ContextCompleteness::Full
    );
    assert_eq!(context.turns.len(), session.user_turns.len());
    assert_eq!(context.turns[1].user, "replacement");
    assert_eq!(
        context.turns[1].last_assistant_text.as_deref(),
        Some("replacement answer")
    );
    let detail_turns = crate::handoff::load_turns(&session);
    assert_eq!(detail_turns.len(), 2);
    assert!(detail_turns
        .iter()
        .flat_map(|t| &t.work_entries)
        .all(|e| !e.text.contains("abandoned")));

    // Rewind again without submitting a replacement: the transcript has no
    // new boundary, so the DB's remaining step range must exclude its tail.
    let conn = rusqlite::Connection::open(&path).expect("reopen temp DB");
    conn.execute("DELETE FROM steps WHERE idx >= 2", [])
        .expect("rewind DB");
    conn.execute("INSERT INTO steps VALUES (1, 15, X'')", [])
        .expect("keep first response step");
    let session = parse_db(&path, 0, &HashMap::new()).expect("parse rewound session");
    assert_eq!(session.user_turns, ["first"]);
    assert_eq!(session.assistant_blob, "retained answer");
    assert_eq!(session.updated_at_ms, 1_784_768_584_000);
    let context = crate::session_context::load(&session);
    assert_eq!(context.turns.len(), 1);
    assert_eq!(
        context.turns[0].last_assistant_text.as_deref(),
        Some("retained answer")
    );
    assert_eq!(
        context.completeness,
        crate::session_context::ContextCompleteness::Full
    );

    // An empty steps table is an empty active path, not an unavailable bound.
    conn.execute("DELETE FROM steps", []).expect("clear DB");
    let (turns, activity) = crate::session_context::antigravity::parse_turns_with_activity_for_db(
        &logs.join("transcript_full.jsonl"),
        &path,
    )
    .expect("parse empty active path");
    assert!(turns.is_empty());
    assert_eq!(activity, None);
    drop(conn);
    std::fs::remove_dir_all(root).expect("remove fixture");
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

/// A user step whose input invoked `skill` (`19.3[].2.27.1.{1,2}`, agy 1.4.x).
fn skill_call_payload(message: &str, skill: &str, timestamp_ms: i64) -> Vec<u8> {
    let mut entry = pb_len_field(1, skill.as_bytes());
    entry.extend(pb_len_field(2, b"<SKILL>The user has explicitly invoked"));
    let mention = pb_len_field(2, &pb_len_field(27, &pb_len_field(1, &entry)));
    let mut step = pb_len_field(2, message.as_bytes());
    step.extend(pb_len_field(3, &mention));
    let mut payload = pb_len_field(19, &step);
    payload.extend(timestamp_payload(timestamp_ms));
    payload
}

#[test]
fn parse_db_keeps_a_bare_skill_call_and_drops_a_bare_builtin_command() {
    let path = std::env::temp_dir().join(format!(
        "s7s-antigravity-skill-call-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let conn = rusqlite::Connection::open(&path).expect("open temp DB");
    conn.execute(
        "CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, step_payload BLOB)",
        [],
    )
    .expect("create steps");
    for (idx, payload) in [
        user_payload("first question", 1_784_768_523_000),
        user_payload("/model", 1_784_768_600_000),
        skill_call_payload("/wrapup", "wrapup", 1_784_768_700_000),
    ]
    .into_iter()
    .enumerate()
    {
        conn.execute(
            "INSERT INTO steps VALUES (?1, 14, ?2)",
            rusqlite::params![idx as i64, payload],
        )
        .expect("insert user step");
    }
    drop(conn);

    let session = parse_db(&path, 0, &HashMap::new()).expect("parse session");
    assert_eq!(session.user_turns, vec!["first question", "/wrapup"]);
    let _ = std::fs::remove_file(path);
}
