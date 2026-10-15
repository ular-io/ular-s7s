use super::*;

fn entry(kind: ContextEntryKind, text: &str) -> ContextEntry {
    ContextEntry {
        kind,
        text: text.to_string(),
    }
}

#[test]
fn strip_final_answer_echo_removes_trailing_duplicate() {
    let mut turn = ContextTurn {
        user: "질문".to_string(),
        submitted_at_ms: None,
        last_assistant_text: Some("최종 답변".to_string()),
        entries: vec![
            entry(ContextEntryKind::AssistantText, "중간 설명"),
            entry(ContextEntryKind::ToolCall, "cargo test"),
            entry(ContextEntryKind::AssistantText, "최종 답변"),
        ],
    };
    strip_final_answer_echo(&mut turn);
    assert_eq!(turn.entries.len(), 2);
    assert!(turn.entries.iter().all(|e| e.text != "최종 답변"));
    assert_eq!(turn.last_assistant_text.as_deref(), Some("최종 답변"));
}

#[test]
fn strip_final_answer_echo_removes_even_when_tools_follow() {
    // task-notification flow: the last assistant text is not the last entry.
    let mut turn = ContextTurn {
        user: "질문".to_string(),
        submitted_at_ms: None,
        last_assistant_text: Some("답변".to_string()),
        entries: vec![
            entry(ContextEntryKind::AssistantText, "답변"),
            entry(ContextEntryKind::ToolResult, "<task-notification>done"),
        ],
    };
    strip_final_answer_echo(&mut turn);
    assert_eq!(turn.entries.len(), 1);
    assert_eq!(turn.entries[0].kind, ContextEntryKind::ToolResult);
}

#[test]
fn strip_final_answer_echo_keeps_earlier_identical_text() {
    // A mid-turn repetition of the final answer text is genuine work; only
    // the trailing echo (the entry that produced last_assistant_text) goes.
    let mut turn = ContextTurn {
        user: "질문".to_string(),
        submitted_at_ms: None,
        last_assistant_text: Some("같은 문장".to_string()),
        entries: vec![
            entry(ContextEntryKind::AssistantText, "같은 문장"),
            entry(ContextEntryKind::ToolCall, "ls"),
            entry(ContextEntryKind::AssistantText, "같은 문장"),
        ],
    };
    strip_final_answer_echo(&mut turn);
    assert_eq!(turn.entries.len(), 2);
    assert_eq!(turn.entries[0].kind, ContextEntryKind::AssistantText);
    assert_eq!(turn.entries[0].text, "같은 문장");
}

#[test]
fn strip_final_answer_echo_no_op_without_match() {
    let mut turn = ContextTurn {
        user: "질문".to_string(),
        submitted_at_ms: None,
        last_assistant_text: None,
        entries: vec![entry(ContextEntryKind::AssistantText, "텍스트")],
    };
    strip_final_answer_echo(&mut turn);
    assert_eq!(turn.entries.len(), 1);

    let mut turn = ContextTurn {
        user: "질문".to_string(),
        submitted_at_ms: None,
        last_assistant_text: Some("다른 답".to_string()),
        entries: vec![entry(ContextEntryKind::AssistantText, "텍스트")],
    };
    strip_final_answer_echo(&mut turn);
    assert_eq!(turn.entries.len(), 1);
}

fn derived_session(source_id: &str) -> Session {
    Session {
        agent: Agent::Claude,
        profile_id: "builtin-claude".to_string(),
        id: "derived-1".to_string(),
        source_path: None,
        cwd: std::path::PathBuf::from("/tmp/demo"),
        folder: "demo".to_string(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["question".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: None,
        title_fixed: false,
        context_source: Some(crate::model::ContextSource {
            id: source_id.to_string(),
            agent: Agent::Codex,
            profile: "builtin-codex".to_string(),
        }),
    }
}

fn indexed_source(id: &str) -> Session {
    let mut s = derived_session("unused");
    s.agent = Agent::Codex;
    s.profile_id = "builtin-codex".to_string();
    s.id = id.to_string();
    s.context_source = None;
    s
}

#[test]
fn context_source_state_follows_the_index() {
    let session = derived_session("source-1");

    // No index: the reference is carried, never judged.
    let unverified = load(&session);
    let reference = unverified.source.context_source.expect("source ref");
    assert_eq!(reference.state, ContextSourceState::Unverified);
    assert_eq!(reference.source.id, "source-1");

    let index = vec![indexed_source("source-1"), session.clone()];
    assert_eq!(
        load_in_index(&session, &index)
            .source
            .context_source
            .unwrap()
            .state,
        ContextSourceState::Present
    );

    // Source deleted (or in a profile that is not configured).
    let index = vec![session.clone()];
    assert_eq!(
        load_in_index(&session, &index)
            .source
            .context_source
            .unwrap()
            .state,
        ContextSourceState::Missing
    );
}

#[test]
fn an_ordinary_session_has_no_context_source() {
    let mut session = derived_session("source-1");
    session.context_source = None;
    assert!(load_in_index(&session, &[]).source.context_source.is_none());
}

/// Manual parity audit over the machine's real sessions: for every session
/// whose detailed parse succeeds, the context turn count must equal the list
/// parser's Q count (list == Detail == CLI invariant). Antigravity is
/// report-only in the strict assertion: its list (SQLite DB) and detail
/// (transcript log) come from different stores, and the DB list parser can
/// undercount newer payload shapes — detail >= list is tolerated there
/// (transcript truncation already falls back to UserTurnsOnly in `load`).
/// Run explicitly with:
/// `cargo test real_data_turn_parity -- --ignored --nocapture`
#[test]
#[ignore]
fn real_data_turn_parity() {
    let profiles = crate::profile::ProfileStore::load();
    let result = crate::scan::scan(&profiles.profiles, false);
    let mut checked = 0usize;
    let mut mismatches = 0usize;
    for s in &result.sessions {
        let ctx = load(s);
        if ctx.completeness != ContextCompleteness::Full {
            continue;
        }
        checked += 1;
        if ctx.turns.len() == s.user_turns.len() {
            continue;
        }
        if s.agent == Agent::Antigravity && ctx.turns.len() > s.user_turns.len() {
            eprintln!(
                "INFO agy detail>list (DB list undercount) {} list={} ctx={}",
                s.id,
                s.user_turns.len(),
                ctx.turns.len()
            );
            continue;
        }
        mismatches += 1;
        eprintln!(
            "MISMATCH {} {} list={} ctx={}",
            s.agent.key(),
            s.id,
            s.user_turns.len(),
            ctx.turns.len()
        );
    }
    eprintln!("checked {checked} full-context sessions, {mismatches} mismatches");
    assert_eq!(mismatches, 0, "turn-count parity violated");
}
