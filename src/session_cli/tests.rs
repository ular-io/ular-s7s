use super::handoff::*;
use super::*;
use clap::Parser;

#[derive(Parser)]
struct TestCli {
    #[command(subcommand)]
    command: TestCmd,
}

#[derive(clap::Subcommand)]
enum TestCmd {
    Session(SessionArgs),
}

fn parse(args: &[&str]) -> Result<SessionArgs, clap::Error> {
    TestCli::try_parse_from(args).map(|c| match c.command {
        TestCmd::Session(s) => s,
    })
}

fn show(args: &[&str]) -> Result<ShowArgs, clap::Error> {
    parse(args).map(|s| match s.command {
        SessionCommand::Show(a) => a,
        _ => panic!("expected show subcommand"),
    })
}

fn search(args: &[&str]) -> Result<SearchArgs, clap::Error> {
    parse(args).map(|s| match s.command {
        SessionCommand::Search(a) => a,
        _ => panic!("expected search subcommand"),
    })
}

fn list(args: &[&str]) -> Result<ListArgs, clap::Error> {
    parse(args).map(|s| match s.command {
        SessionCommand::List(a) => a,
        _ => panic!("expected list subcommand"),
    })
}

fn rename(args: &[&str]) -> Result<RenameArgs, clap::Error> {
    parse(args).map(|s| match s.command {
        SessionCommand::Rename(a) => a,
        _ => panic!("expected rename subcommand"),
    })
}

fn delete(args: &[&str]) -> Result<DeleteArgs, clap::Error> {
    parse(args).map(|s| match s.command {
        SessionCommand::Delete(a) => a,
        _ => panic!("expected delete subcommand"),
    })
}

fn handoff(args: &[&str]) -> Result<HandoffArgs, clap::Error> {
    parse(args).map(|s| match s.command {
        SessionCommand::Handoff(a) => a,
        _ => panic!("expected handoff subcommand"),
    })
}

#[test]
fn show_parses_all_supported_options() {
    let s = show(&[
        "s7s",
        "session",
        "show",
        "abc-def",
        "--agent",
        "codex",
        "--profile",
        "builtin-codex",
        "--user-only",
        "--turn",
        "7",
    ])
    .expect("parse");
    assert_eq!(s.session_id, "abc-def");
    assert_eq!(s.agent.as_deref(), Some("codex"));
    assert_eq!(s.profile.as_deref(), Some("builtin-codex"));
    assert!(s.user_only);
    assert_eq!(s.turn, Some(7));
    assert!(!s.bootstrap);
}

#[test]
fn show_rejects_unknown_options_and_invalid_values() {
    // Unknown options must fail instead of being ignored.
    assert!(show(&["s7s", "session", "show", "abc", "--unknown-flag"]).is_err());
    // Missing session id fails.
    assert!(parse(&["s7s", "session", "show"]).is_err());
    // Invalid agent value fails at parse time.
    assert!(show(&["s7s", "session", "show", "abc", "--agent", "gpt"]).is_err());
    // Non-numeric turn fails.
    assert!(show(&["s7s", "session", "show", "abc", "--turn", "x"]).is_err());
}

#[test]
fn show_bootstrap_conflicts_with_turn() {
    assert!(show(&[
        "s7s",
        "session",
        "show",
        "abc",
        "--bootstrap",
        "--turn",
        "1"
    ])
    .is_err());
    assert!(show(&["s7s", "session", "show", "abc", "--bootstrap"]).is_ok());
}

#[test]
fn requires_a_known_subcommand() {
    // A bare `session <id>` is no longer valid; it must be `session show <id>`.
    assert!(parse(&["s7s", "session", "abc-def"]).is_err());
    assert!(parse(&["s7s", "session"]).is_err());
}

#[test]
fn search_parses_query_and_repeatable_filters() {
    let s = search(&[
        "s7s",
        "session",
        "search",
        "test",
        "--folder",
        "vqs-gw",
        "--folder",
        "vqs-api",
        "--agent",
        "codex",
        "--agent",
        "claude",
        "--profile",
        "builtin-codex",
        "--limit",
        "50",
    ])
    .expect("parse");
    assert_eq!(s.query, vec!["test"]);
    assert_eq!(s.folder, vec!["vqs-gw", "vqs-api"]);
    assert_eq!(s.agent, vec!["codex", "claude"]);
    assert_eq!(s.profile, vec!["builtin-codex"]);
    assert_eq!(s.limit, 50);
    assert!(!s.full);
}

#[test]
fn search_parses_full_flag() {
    let s = search(&["s7s", "session", "search", "test", "--full"]).expect("parse");
    assert!(s.full);
}

#[test]
fn search_joins_multiple_query_tokens() {
    let s = search(&["s7s", "session", "search", "final", "message"]).expect("parse");
    assert_eq!(s.query, vec!["final", "message"]);
    assert_eq!(s.query.join(" "), "final message");
}

#[test]
fn search_defaults_and_rejects_bad_input() {
    // limit defaults to 20.
    let s = search(&["s7s", "session", "search", "x"]).expect("parse");
    assert_eq!(s.limit, 20);
    // A query is required.
    assert!(parse(&["s7s", "session", "search"]).is_err());
    // Invalid agent value fails at parse time.
    assert!(search(&["s7s", "session", "search", "x", "--agent", "gpt"]).is_err());
    // Non-numeric limit fails.
    assert!(search(&["s7s", "session", "search", "x", "--limit", "many"]).is_err());
}

#[test]
fn list_takes_filters_without_a_query() {
    let l = list(&[
        "s7s",
        "session",
        "list",
        "--folder",
        "ular-s7s",
        "--folder",
        "ular-card",
        "--agent",
        "claude",
        "--profile",
        "builtin-claude",
        "--limit",
        "5",
    ])
    .expect("parse");
    assert_eq!(l.folder, vec!["ular-s7s", "ular-card"]);
    assert_eq!(l.agent, vec!["claude"]);
    assert_eq!(l.profile, vec!["builtin-claude"]);
    assert_eq!(l.limit, 5);

    // No filter at all is valid: it lists everything, capped by --limit.
    let bare = list(&["s7s", "session", "list"]).expect("parse");
    assert!(bare.folder.is_empty());
    assert_eq!(bare.limit, 20);

    // A positional keyword belongs to `search`, not `list`.
    assert!(list(&["s7s", "session", "list", "rename"]).is_err());
    assert!(list(&["s7s", "session", "list", "--agent", "gpt"]).is_err());
}

#[test]
fn rename_requires_both_id_and_title() {
    let r = rename(&[
        "s7s",
        "session",
        "rename",
        "abc-def",
        "cache rebuild bug",
        "--agent",
        "codex",
        "--profile",
        "builtin-codex",
    ])
    .expect("parse");
    assert_eq!(r.session_id, "abc-def");
    assert_eq!(r.title, "cache rebuild bug");
    assert_eq!(r.agent.as_deref(), Some("codex"));
    assert_eq!(r.profile.as_deref(), Some("builtin-codex"));

    // A missing title must fail rather than rename to an empty string.
    assert!(parse(&["s7s", "session", "rename", "abc-def"]).is_err());
    assert!(parse(&["s7s", "session", "rename"]).is_err());
}

#[test]
fn delete_defaults_to_no_confirmation() {
    let d = delete(&["s7s", "session", "delete", "abc-def"]).expect("parse");
    assert_eq!(d.session_id, "abc-def");
    // The destructive flag must never default to on.
    assert!(!d.yes);

    let confirmed = delete(&[
        "s7s",
        "session",
        "delete",
        "abc-def",
        "--agent",
        "antigravity",
        "--yes",
    ])
    .expect("parse");
    assert!(confirmed.yes);
    assert_eq!(confirmed.agent.as_deref(), Some("antigravity"));

    assert!(parse(&["s7s", "session", "delete"]).is_err());
    assert!(delete(&["s7s", "session", "delete", "abc", "--agent", "gpt"]).is_err());
}

#[test]
fn handoff_requires_only_a_title() {
    let h = handoff(&["s7s", "session", "handoff", "--title", "rewind parity"]).expect("parse");
    assert_eq!(h.title, "rewind parity");
    // Everything else follows the source session.
    assert!(h.agent.is_none());
    assert!(h.profile.is_none());
    assert!(h.folder.is_none());
    assert!(h.from.is_none());
    assert!(h.body_file.is_none());
    assert!(!h.no_source);

    assert!(parse(&["s7s", "session", "handoff"]).is_err());
    // The body arrives on stdin, never as a positional.
    assert!(handoff(&["s7s", "session", "handoff", "--title", "x", "body"]).is_err());
}

#[test]
fn handoff_rejects_naming_a_source_it_was_told_to_omit() {
    assert!(handoff(&[
        "s7s",
        "session",
        "handoff",
        "--title",
        "x",
        "--from",
        "abc-123",
        "--no-source",
    ])
    .is_err());

    // Each on its own is fine.
    assert!(handoff(&["s7s", "session", "handoff", "--title", "x", "--no-source"]).is_ok());
    assert!(handoff(&["s7s", "session", "handoff", "--title", "x", "--from", "abc"]).is_ok());
    assert!(handoff(&["s7s", "session", "handoff", "--title", "x", "--agent", "gpt"]).is_err());
}

#[test]
fn handoff_title_gains_the_prefix_once() {
    assert_eq!(handoff_title("rewind parity"), "HAND-OVER: rewind parity");
    assert_eq!(
        handoff_title("  rewind parity  "),
        "HAND-OVER: rewind parity"
    );
    // An author who already wrote the prefix must not get it twice.
    assert_eq!(
        handoff_title("HAND-OVER: rewind parity"),
        "HAND-OVER: rewind parity"
    );
}

#[test]
fn title_line_caps_an_untitled_session_first_message() {
    assert_eq!(title_line("  짧은 제목  "), "짧은 제목");
    // Newlines are flattened before the cap, so the output stays one line.
    assert_eq!(title_line("첫 줄\n둘째 줄"), "첫 줄 둘째 줄");

    let long: String = "가".repeat(200);
    let capped = title_line(&long);
    assert_eq!(capped.chars().count(), 121, "120 chars plus the ellipsis");
    assert!(capped.ends_with('…'));
}

fn store_with(entries: &[(&str, Agent)]) -> ProfileStore {
    let mut store = ProfileStore::load();
    store.profiles = entries
        .iter()
        .map(|(id, agent)| crate::profile::Profile {
            id: (*id).to_string(),
            agent: *agent,
            name: (*id).to_string(),
            path: std::path::PathBuf::from(format!("/tmp/{id}")),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: true,
        })
        .collect();
    store
}

#[test]
fn handoff_refuses_a_profile_belonging_to_another_agent() {
    let store = store_with(&[
        ("builtin-claude", Agent::Claude),
        ("builtin-codex", Agent::Codex),
    ]);

    // A cross-agent profile would send the rename into another account's
    // config root, so it is refused rather than quietly used.
    let err = handoff_profile(&store, Agent::Codex, Some("builtin-claude"))
        .expect_err("cross-agent profile must be refused");
    assert!(err.contains("belongs to claude"), "{err}");

    // Matching and defaulted lookups still work.
    assert_eq!(
        handoff_profile(&store, Agent::Codex, Some("builtin-codex"))
            .expect("match")
            .id,
        "builtin-codex"
    );
    assert_eq!(
        handoff_profile(&store, Agent::Codex, None)
            .expect("default")
            .id,
        "builtin-codex"
    );
    assert!(handoff_profile(&store, Agent::Codex, Some("ghost")).is_err());

    // An agent with no profile at all is an error, not a wrong-account guess.
    let claude_only = store_with(&[("builtin-claude", Agent::Claude)]);
    assert!(handoff_profile(&claude_only, Agent::Antigravity, None).is_err());
}
