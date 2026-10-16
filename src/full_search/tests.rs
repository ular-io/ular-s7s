use super::query::highlight_ranges;
use super::*;
use crate::model::Agent;
use crate::session_context::ContextEntry;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Collision-free temp file: a per-process counter, not wall-clock time.
fn write_temp(content: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "s7s-full-search-{}-{}.jsonl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, content).expect("write temp file");
    path
}

fn claude_session(id: &str, source: Option<PathBuf>, user_turns: &[&str]) -> Session {
    Session {
        agent: Agent::Claude,
        profile_id: "p".into(),
        id: id.into(),
        source_path: source,
        cwd: PathBuf::from("/tmp"),
        folder: "tmp".into(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: user_turns.iter().map(|t| t.to_string()).collect(),
        user_turn_timestamps_ms: Vec::new(),
        search_blob: crate::normalize::nfc_lower(&user_turns.join("\n")),
        assistant_blob: String::new(),
        title_hint: None,
        title_fixed: false,
        context_source: None,
    }
}

/// Two turns; the build failure exists only in the first turn's tool result.
const TRANSCRIPT: &str = r#"{"type":"user","uuid":"a","parentUuid":null,"message":{"role":"user","content":"빌드 해줘"}}
{"type":"assistant","uuid":"b","parentUuid":"a","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo build"}}]}}
{"type":"user","uuid":"c","parentUuid":"b","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"Compiling s7s\nerror[E0425]: cannot find value `Linker`\n  --> src/main.rs:3:5"}]}}
{"type":"assistant","uuid":"d","parentUuid":"c","message":{"role":"assistant","content":[{"type":"text","text":"빌드가 실패했다."}]}}
{"type":"user","uuid":"e","parentUuid":"d","message":{"role":"user","content":"두 번째 질문"}}
{"type":"assistant","uuid":"f","parentUuid":"e","message":{"role":"assistant","content":[{"type":"text","text":"두 번째 답"}]}}
"#;

fn turn(user: &str, entries: &[(ContextEntryKind, &str)], answer: Option<&str>) -> ContextTurn {
    ContextTurn {
        user: user.into(),
        submitted_at_ms: None,
        last_assistant_text: answer.map(str::to_string),
        entries: entries
            .iter()
            .map(|&(kind, text)| ContextEntry {
                kind,
                text: text.into(),
            })
            .collect(),
    }
}

#[test]
fn query_folds_case_and_drops_duplicate_tokens() {
    let query = Query::parse("  Cargo cargo  빌드 ").expect("tokens");
    assert_eq!(query.tokens, ["cargo", "빌드"]);
    assert!(Query::parse(" \t ").is_none());
}

#[test]
fn highlights_map_back_to_the_displayed_case() {
    let query = Query::parse("linker 실패").unwrap();
    let text = "LINKER error: 빌드 실패";
    let ranges = highlight_ranges(text, &query);
    let parts: Vec<&str> = ranges.iter().map(|&(s, e)| &text[s..e]).collect();
    assert_eq!(parts, ["LINKER", "실패"]);
}

#[test]
fn every_token_must_occur_but_not_in_the_same_text() {
    let turns = vec![
        turn(
            "first",
            &[(ContextEntryKind::ToolCall, "cargo build")],
            None,
        ),
        turn(
            "second",
            &[(
                ContextEntryKind::ToolResult,
                "warning: unused\nlinker failed",
            )],
            Some("fixed"),
        ),
    ];
    let m = match_turns(&turns, &Query::parse("cargo linker").unwrap()).expect("match");
    // Both texts hold one token each; the first one is the snippet.
    assert_eq!(
        (m.turn, m.field, m.hits),
        (0, MatchField::Entry(ContextEntryKind::ToolCall), 2)
    );
    assert!(match_turns(&turns, &Query::parse("cargo absent").unwrap()).is_none());

    // A text holding more tokens wins over an earlier one.
    let m = match_turns(&turns, &Query::parse("linker failed").unwrap()).expect("match");
    assert_eq!(
        (m.turn, m.field),
        (1, MatchField::Entry(ContextEntryKind::ToolResult))
    );
    assert_eq!(m.lines.len(), 2);
    assert_eq!(m.lines[0].text, "warning: unused");
    assert_eq!(m.lines[1].text, "linker failed");
    assert_eq!(m.lines[1].highlights, vec![(0, 6), (7, 13)]);
}

#[test]
fn long_lines_are_cut_around_the_first_highlight() {
    let line = format!("{}needle{}", "a".repeat(300), "b".repeat(300));
    let turns = vec![turn("q", &[(ContextEntryKind::ToolResult, &line)], None)];
    let m = match_turns(&turns, &Query::parse("NEEDLE").unwrap()).unwrap();
    let snippet = &m.lines[0];
    assert!(snippet.text.starts_with('…') && snippet.text.ends_with('…'));
    let (s, e) = snippet.highlights[0];
    assert_eq!(&snippet.text[s..e], "needle");
    assert_eq!(snippet.text[..s].chars().count(), 1 + 40);
}

#[test]
fn snippets_strip_terminal_escapes() {
    let turns = vec![turn(
        "q",
        &[(
            ContextEntryKind::ToolResult,
            "\u{1b}[31merror\u{1b}[0m:\tbad\u{7}",
        )],
        None,
    )];
    let m = match_turns(&turns, &Query::parse("error").unwrap()).unwrap();
    assert_eq!(m.lines[0].text, "error: bad ");
    assert_eq!(m.lines[0].highlights, vec![(0, 5)]);
}

#[test]
fn searches_tool_results_through_the_detailed_parser() {
    let path = write_temp(TRANSCRIPT);
    let session = claude_session("s1", Some(path.clone()), &["빌드 해줘", "두 번째 질문"]);
    let m = search_session(&session, &Query::parse("linker").unwrap()).expect("match");
    assert_eq!(m.turn, 0);
    assert_eq!(m.field, MatchField::Entry(ContextEntryKind::ToolResult));
    assert_eq!(m.lines[1].text, "error[E0425]: cannot find value `Linker`");
    // Korean in the assistant text matches through the raw prefilter.
    let m = search_session(&session, &Query::parse("실패했다").unwrap()).expect("match");
    assert_eq!((m.turn, m.field), (0, MatchField::Answer));
    assert!(search_session(&session, &Query::parse("linker 없는말").unwrap()).is_none());
    let _ = std::fs::remove_file(path);
}

#[test]
fn prefilter_rejects_absent_tokens_and_skips_unsafe_ones() {
    let path = write_temp(TRANSCRIPT);
    let session = claude_session("s1", Some(path.clone()), &[]);
    assert!(raw::may_match(&session, &Query::parse("LINKER").unwrap()));
    assert!(!raw::may_match(&session, &Query::parse("absent").unwrap()));
    // `"` is escaped in JSON, so the token cannot be ruled out by raw bytes.
    assert!(raw::may_match(&session, &Query::parse("\"absent").unwrap()));
    // Cased non-ASCII cannot be folded byte-wise.
    assert!(raw::may_match(&session, &Query::parse("Été").unwrap()));
    let _ = std::fs::remove_file(path);

    // Embedded JSON escapes disable the prefilter for non-ASCII tokens.
    let path = write_temp(r#"{"arguments":"{\"q\":\"\\uac00\"}"}"#);
    let session = claude_session("s2", Some(path.clone()), &[]);
    assert!(raw::may_match(&session, &Query::parse("가").unwrap()));
    assert!(!raw::may_match(&session, &Query::parse("absent").unwrap()));
    let _ = std::fs::remove_file(path);
}

#[test]
fn unreadable_sources_fall_back_to_indexed_user_turns() {
    let session = claude_session("s1", None, &["deploy the worker"]);
    let m = search_session(&session, &Query::parse("worker").unwrap()).expect("match");
    assert_eq!((m.turn, m.field), (0, MatchField::User));
}

#[test]
fn workers_stream_one_result_per_session_and_stop_on_cancel() {
    let path = write_temp(TRANSCRIPT);
    let jobs: Vec<(usize, Session)> = (0..6)
        .map(|i| {
            let source = (i % 2 == 0).then(|| path.clone());
            (i, claude_session(&format!("s{i}"), source, &["other"]))
        })
        .collect();
    let rx = spawn(jobs, Query::parse("linker").unwrap(), Arc::default());
    let mut results: Vec<(usize, bool)> = rx
        .iter()
        .map(|(key, found)| (key, found.is_some()))
        .collect();
    results.sort_unstable();
    let expected: Vec<(usize, bool)> = (0..6).map(|i| (i, i % 2 == 0)).collect();
    assert_eq!(results, expected);

    let cancel = Arc::new(AtomicBool::new(true));
    let jobs = vec![(0, claude_session("s", Some(path.clone()), &[]))];
    let rx = spawn(jobs, Query::parse("linker").unwrap(), cancel);
    assert!(rx.recv_timeout(Duration::from_secs(5)).is_err());
    let _ = std::fs::remove_file(path);
}

/// Real-store check of the prefilter contract and search time. For every
/// session whose parsed content matches a query, the prefiltered search must
/// match too (the prefilter may only produce false positives). Queries come
/// from `S7S_FULL_SEARCH_QUERIES` (`|`-separated) or a default mix of ASCII,
/// path, punctuation, and Hangul tokens. Run explicitly with:
/// `cargo test --release real_data_full_search -- --ignored --nocapture`
#[test]
#[ignore]
fn real_data_full_search() {
    let raw = std::env::var("S7S_FULL_SEARCH_QUERIES")
        .unwrap_or_else(|_| "cargo|linker|error[e|src/main.rs|빌드|실패 원인|rollout".into());
    let queries: Vec<Query> = raw.split('|').filter_map(Query::parse).collect();
    let profiles = crate::profile::ProfileStore::load();
    let sessions = crate::scan::scan(&profiles.profiles, false).sessions;
    let bytes: u64 = sessions.iter().map(|s| s.size_bytes).sum();
    eprintln!("{} sessions, {} MB", sessions.len(), bytes / 1_000_000);

    for query in &queries {
        let started = std::time::Instant::now();
        let jobs: Vec<(usize, Session)> = sessions.iter().cloned().enumerate().collect();
        let found = spawn(jobs, query.clone(), Arc::default())
            .iter()
            .filter(|(_, m)| m.is_some())
            .count();
        eprintln!(
            "{:?}: {found} matches in {:?}",
            query.tokens,
            started.elapsed()
        );
    }

    let mut missed = 0;
    for session in &sessions {
        let turns = session_context::load(session).turns;
        for query in &queries {
            if match_turns(&turns, query).is_some() && search_session(session, query).is_none() {
                missed += 1;
                eprintln!(
                    "MISSED {:?} {} {}",
                    query.tokens,
                    session.agent.key(),
                    session.id
                );
            }
        }
    }
    assert_eq!(missed, 0, "prefilter dropped a matching session");
}

#[test]
fn complete_queries_reject_empty_or_excess_tokens_instead_of_truncating() {
    assert!(Query::parse_complete(" \t ").is_err());
    let sixteen = (0..16)
        .map(|i| format!("token{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let query = Query::parse_complete(&sixteen).expect("16 distinct tokens");
    assert_eq!(query.tokens.len(), 16);
    assert_eq!(
        Query::parse_complete(&format!("{sixteen} TOKEN0")).unwrap(),
        query
    );
    assert!(Query::parse_complete(&format!("{sixteen} absent"))
        .unwrap_err()
        .contains("at most 16"));
    assert_eq!(Query::parse_complete("É e\u{301}").unwrap().tokens, ["é"]);
}
