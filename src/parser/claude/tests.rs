use super::*;
use crate::parser::turn;

#[test]
fn title_meta_fixed_only_for_explicit_name_sources() {
    let root = std::env::temp_dir().join(format!("s7s-claude-meta-fixed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("sessions");
    std::fs::create_dir_all(&dir).expect("create sessions dir");
    for (file, body) in [
        (
            "a.json",
            r#"{"sessionId":"id-custom","name":"제목","nameSource":"custom"}"#,
        ),
        (
            "b.json",
            r#"{"sessionId":"id-user","name":"제목","nameSource":"user"}"#,
        ),
        (
            "c.json",
            r#"{"sessionId":"id-derived","name":"ular-s7s-2b","nameSource":"derived"}"#,
        ),
        ("d.json", r#"{"sessionId":"id-none","name":"4a72d37c"}"#),
    ] {
        std::fs::write(dir.join(file), body).expect("write meta");
    }

    let meta = load_title_meta(&root);
    assert!(meta.get("id-custom").expect("custom").fixed);
    assert!(meta.get("id-user").expect("user").fixed);
    assert!(!meta.get("id-derived").expect("derived").fixed);
    // Missing nameSource must not be fixed: degenerate CLI auto names
    // (session id as name) would otherwise bypass the bad-auto-title fallback.
    assert!(!meta.get("id-none").expect("none").fixed);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn extracts_question_answers_from_tool_use_result() {
    let v: Value = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "toolu_123",
                "content": "Your questions have been answered..."
            }]
        },
        "toolUseResult": {
            "questions": [{
                "question": "헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요?",
                "header": "질문",
                "multiSelect": false
            }],
            "answers": {
                "헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요?": "적용"
            }
        }
    });

    let out = turn::extract_question_answers(&v).expect("expected question/answer turn");
    assert_eq!(
        out,
        "· 헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요? → 적용"
    );
}

#[test]
fn extracts_plain_user_text_without_tool_result_noise() {
    let v: Value = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": "안녕하세요"
        }
    });

    let out = turn::extract_user_text(&v).expect("expected plain user text");
    assert_eq!(out, "안녕하세요");
}

fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ular-s7s-{}-{}.jsonl",
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, content).expect("write temp file");
    path
}

#[test]
fn rewind_hides_turns_on_abandoned_branch() {
    // Q2 was abandoned by /rewind: Q3 branches from the same parent (b) as Q2.
    let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"question 1"}}
{"timestamp":"2026-07-23T01:03:03.456Z","type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"answer 1"}]}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"abandoned question 2"}}
{"timestamp":"2026-07-23T05:00:00Z","type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"abandoned answer 2"}]}}
{"timestamp":"2026-07-23T05:01:00Z","type":"system","subtype":"turn_duration","uuid":"x","parentUuid":"d"}
{"timestamp":"2026-07-23T03:04:05.678Z","type":"user","uuid":"e","parentUuid":"b","message":{"role":"user","content":"replacement question 3"}}
{"timestamp":"2026-07-23T03:05:05.678Z","type":"assistant","uuid":"f","parentUuid":"e","message":{"role":"assistant","content":[{"type":"text","text":"answer 3"}]}}
"#;
    let path = write_temp("rewind-branch", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(
        session.user_turns,
        vec!["question 1", "replacement question 3"]
    );
    assert_eq!(
        session.user_turn_timestamps_ms,
        vec![Some(1_784_768_523_456), Some(1_784_775_845_678)]
    );
    assert_eq!(session.updated_at_ms, 1_784_775_905_678);
    let _ = std::fs::remove_file(path);
}

#[test]
fn linear_session_keeps_all_turns() {
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"질문2"}}
"#;
    let path = write_temp("linear", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["질문1", "질문2"]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn activity_time_ignores_resume_only_last_prompt_write() {
    let content = r#"
{"timestamp":"2026-07-22T08:58:46.672Z","type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"question"}}
{"timestamp":"2026-07-22T08:59:43.293Z","type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"answer"}]}}
{"timestamp":"2026-07-22T08:59:43.525Z","type":"system","subtype":"turn_duration","uuid":"c","parentUuid":"b","durationMs":56847}
{"type":"last-prompt","lastPrompt":"question","leafUuid":"c","sessionId":"test"}
"#;
    let path = write_temp("activity-last-prompt", content);
    let later_mtime = 1_784_769_999_999;
    let session = parse_file(&path, later_mtime, None).expect("expected session");
    assert_eq!(session.updated_at_ms, 1_784_710_783_525);
    assert_ne!(session.updated_at_ms, later_mtime);
    let _ = std::fs::remove_file(path);
}

#[test]
fn activity_time_uses_new_query_when_response_is_not_complete() {
    let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"first question"}}
{"timestamp":"2026-07-23T01:03:03.456Z","type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"first answer"}]}}
{"timestamp":"2026-07-23T01:03:04.456Z","type":"system","subtype":"turn_duration","uuid":"c","parentUuid":"b"}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","uuid":"d","parentUuid":"c","message":{"role":"user","content":"second question"}}
"#;
    let path = write_temp("activity-open-query", content);
    let session = parse_file(&path, 1_784_799_999_999, None).expect("expected session");
    assert_eq!(session.updated_at_ms, 1_784_772_184_567);
    let _ = std::fs::remove_file(path);
}

#[test]
fn list_session_keeps_user_and_promoted_qa_submit_times() {
    let content = r#"
{"timestamp":"2026-07-23T01:02:03.456Z","type":"user","message":{"role":"user","content":"first question"}}
{"timestamp":"2026-07-23T02:03:04.567Z","type":"user","toolUseResult":{"questions":[{"question":"Continue?"}],"answers":{"Continue?":"Yes"}}}
"#;
    let path = write_temp("turn-times", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns.len(), 2);
    assert_eq!(
        session.user_turn_timestamps_ms,
        vec![Some(1_784_768_523_456), Some(1_784_772_184_567)]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn compaction_keeps_earlier_turns_and_drops_the_summary() {
    // Compaction splits the file: the boundary record has no parentUuid and
    // the CLI-written summary sits in the user role. Earlier turns must stay
    // countable, and the summary must not be counted as a question.
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"system","subtype":"compact_boundary","uuid":"c","parentUuid":null,"logicalParentUuid":"b","content":"Conversation compacted","compactMetadata":{"trigger":"auto"}}
{"type":"user","uuid":"d","parentUuid":"c","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued from a previous conversation. Summary: 질문1 정리"}}
{"type":"assistant","uuid":"e","parentUuid":"d","message":{"role":"assistant","content":[{"type":"text","text":"압축 후 이어서 한 작업"}]}}
{"type":"user","uuid":"f","parentUuid":"e","message":{"role":"user","content":"질문2"}}
{"type":"assistant","uuid":"g","parentUuid":"f","message":{"role":"assistant","content":[{"type":"text","text":"답2"}]}}
"#;
    let path = write_temp("compaction", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["질문1", "질문2"]);
    assert!(!session
        .search_blob
        .contains("this session is being continued"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn summary_without_earlier_turns_keeps_the_session_listed() {
    // Nothing precedes the summary (truncated file): dropping it would leave
    // no turns and hide the session entirely.
    let content = r#"
{"type":"user","uuid":"d","parentUuid":null,"isCompactSummary":true,"message":{"role":"user","content":"This session is being continued from a previous conversation. Summary: 앞선 대화 정리"}}
{"type":"assistant","uuid":"e","parentUuid":"d","message":{"role":"assistant","content":[{"type":"text","text":"이어서 한 작업"}]}}
"#;
    let path = write_temp("compaction-only", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns.len(), 1);
    assert!(session.user_turns[0].contains("앞선 대화 정리"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn broken_chain_falls_back_to_all_turns() {
    // The leaf references a uuid missing from the file: the chain cannot be
    // trusted, so every turn must survive.
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"user","uuid":"c","parentUuid":"ghost","message":{"role":"user","content":"질문2"}}
"#;
    let path = write_temp("broken-chain", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["질문1", "질문2"]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sidechain_entries_do_not_become_leaf() {
    // A trailing subagent (sidechain) entry must not divert the active-path walk.
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"s1","parentUuid":null,"isSidechain":true,"message":{"role":"user","content":"사이드체인 프롬프트"}}
"#;
    let path = write_temp("sidechain-leaf", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["질문1"]);
    let _ = std::fs::remove_file(path);
}

#[test]
fn prefers_custom_title_over_ai_title() {
    let content = r#"
{"type":"ai-title","aiTitle":"GPS 센서 데이터 결합하여 정차 판정 개선","sessionId":"923fd752-5ba7-4cdb-8b54-d1074b046b7c"}
{"type":"custom-title","customTitle":"26-07 주행정보 생성 성능 개선 검토","sessionId":"923fd752-5ba7-4cdb-8b54-d1074b046b7c"}
{"type":"user","message":{"role":"user","content":"첫 질문"}}
"#;

    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "ular-s7s-test-{}.jsonl",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, content).expect("write temp file");

    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.title(), "26-07 주행정보 생성 성능 개선 검토");
    assert!(session.title_fixed);
    let _ = std::fs::remove_file(path);
}

#[test]
fn assistant_blob_indexes_last_answer_and_drops_intermediate() {
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b1","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"중간설명 intermediatekw"}]}}
{"type":"assistant","uuid":"b2","parentUuid":"b1","message":{"role":"assistant","content":[{"type":"text","text":"최종답변 finalkw"}]}}
{"type":"user","uuid":"c","parentUuid":"b2","message":{"role":"user","content":"질문2"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"두번째답변 secondkw"}]}}
"#;
    let path = write_temp("asst-blob", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    // Last answer of each turn is indexed; intermediate answers within a turn are not.
    assert!(session.assistant_blob.contains("finalkw"));
    assert!(session.assistant_blob.contains("secondkw"));
    assert!(!session.assistant_blob.contains("intermediatekw"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn assistant_blob_excludes_rewound_answer() {
    // Q2/A2 abandoned by /rewind: Q3 branches from the same parent (b).
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1 keepkw"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"질문2 버려진 분기"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"버려진답 dropkw"}]}}
{"type":"user","uuid":"e","parentUuid":"b","message":{"role":"user","content":"질문3 리와인드 후"}}
{"type":"assistant","uuid":"f","parentUuid":"e","message":{"role":"assistant","content":[{"type":"text","text":"답3 keep3kw"}]}}
"#;
    let path = write_temp("asst-rewind", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert!(session.assistant_blob.contains("keepkw"));
    assert!(session.assistant_blob.contains("keep3kw"));
    assert!(!session.assistant_blob.contains("dropkw"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn assistant_blob_excludes_bootstrap_ready_response() {
    // The bootstrap ready response precedes the first real question and must not
    // be indexed; the bootstrap prompt itself is not a user turn.
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"<s7s-context-bootstrap>\nRun something\n</s7s-context-bootstrap>"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"readykw 확인했습니다"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"진짜 질문"}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"realkw 답변"}]}}
"#;
    let path = write_temp("asst-bootstrap", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["진짜 질문"]);
    assert!(session.assistant_blob.contains("realkw"));
    assert!(!session.assistant_blob.contains("readykw"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn user_skill_call_is_a_turn_whose_answer_is_indexed() {
    // `/wrapup` is followed by its skill body; the answer after the body
    // belongs to the `/wrapup` turn. `/model` stays out of the turn list.
    let content = r#"
{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"질문1"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"text","text":"답1"}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":"<command-name>/model</command-name>\n<command-message>model</command-message>\n<command-args></command-args>"}}
{"type":"user","uuid":"d","parentUuid":"c","message":{"role":"user","content":"<local-command-stdout>Set model</local-command-stdout>"}}
{"type":"user","uuid":"e","parentUuid":"d","message":{"role":"user","content":"<command-message>wrapup</command-message>\n<command-name>/wrapup</command-name>"}}
{"type":"user","uuid":"f","parentUuid":"e","isMeta":true,"message":{"role":"user","content":[{"type":"text","text":"Base directory for this skill: /x/wrapup"}]}}
{"type":"assistant","uuid":"g","parentUuid":"f","message":{"role":"assistant","content":[{"type":"text","text":"커밋 완료 wrapupkw"}]}}
"#;
    let path = write_temp("skill-call", content);
    let session = parse_file(&path, 0, None).expect("expected session");
    assert_eq!(session.user_turns, vec!["질문1", "/wrapup"]);
    assert!(session.assistant_blob.contains("wrapupkw"));
    let _ = std::fs::remove_file(path);
}
