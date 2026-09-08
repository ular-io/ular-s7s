//! `s7s session` subcommand group: query and manage previous sessions without the TUI.
//!
//! Subcommands:
//!   show <id>       Render one session's context (reference / --turn / --bootstrap).
//!   search <query>  List sessions matching a keyword (+ folder/agent/profile filters).
//!   list            List sessions by filter only, with no keyword.
//!   rename <id> <t> Set one session's display title.
//!   delete <id>     Remove one session's on-disk artifacts (irreversible).
//!
//! Output discipline: primary output goes to stdout, errors/diagnostics to
//! stderr, no ANSI styling, no scan spinner. Exit codes: 0 success, 2 invalid
//! arguments (clap), 1 for lookup/parse failures. `delete` without `--yes` also
//! exits 1: nothing was removed, so it must not read as success in a script.

use crate::filter::Filter;
use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use crate::session_context::{self, render, resolve, ContextCompleteness};
use clap::{Args, Subcommand};
use std::collections::HashSet;

/// Query and manage previous sessions (search/list, show, rename, delete).
#[derive(Args, Debug)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub command: SessionCommand,
}

#[derive(Subcommand, Debug)]
pub enum SessionCommand {
    /// Read context from a previous session
    Show(ShowArgs),
    /// Search sessions by keyword (optionally filtered by folder/agent/profile)
    Search(SearchArgs),
    /// List sessions by filter only, without a keyword
    List(ListArgs),
    /// Set one session's display title
    Rename(RenameArgs),
    /// Delete one session's on-disk artifacts (irreversible)
    Delete(DeleteArgs),
}

/// Render one previous session's context.
#[derive(Args, Debug)]
#[command(after_help = "\
MODES:
  Reference (default)   Neutral historical context: header, trust boundary, every
                        active user turn with assistant excerpts. Contains no
                        stop/wait/language instructions — safe to run from inside
                        any existing agent session.
  Bootstrap             Adds an s7s-authored instruction envelope used only to
                        initialize a NEW session launched with a context source.

EXCERPT LIMITS (defaults):
  User turns            full up to 1,000 chars; longer keeps first/last 500 with
                        an explicit omission marker
  Assistant excerpts    500 chars (historical turns), 2,000 chars (latest turn)
  --turn N              full redacted user text + ordered work entries
  --turn N --user-only  complete redacted user text only

RESOLUTION:
  The full session ID is matched across every configured profile. Zero matches
  fail with a hint; multiple matches list each candidate's agent/profile and
  require --agent/--profile disambiguation. A requested profile that does not
  exist is an error — resolution never falls back to another profile.

EXAMPLES:
  s7s session show 019f36e8-9157-7c63-bee8-8937a6314982
  s7s session show 019f36e8-9157-7c63-bee8-8937a6314982 --user-only
  s7s session show 019f36e8-9157-7c63-bee8-8937a6314982 --turn 7
  s7s session show 019f36e8-9157-7c63-bee8-8937a6314982 --agent codex --profile builtin-codex --bootstrap")]
pub struct ShowArgs {
    /// Full session ID of the source session
    pub session_id: String,
    /// Restrict resolution to one agent
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Option<String>,
    /// Restrict resolution to one profile ID (e.g. builtin-claude)
    #[arg(long)]
    pub profile: Option<String>,
    /// Print user turns only (no assistant excerpts / work entries)
    #[arg(long)]
    pub user_only: bool,
    /// Print full redacted detail for one turn (1-based)
    #[arg(long, value_name = "NUMBER", conflicts_with = "bootstrap")]
    pub turn: Option<usize>,
    /// Emit new-session bootstrap instructions before the session context
    #[arg(long)]
    pub bootstrap: bool,
}

/// Search sessions by keyword across every configured profile.
#[derive(Args, Debug)]
#[command(after_help = "\
MATCHING:
  Space-separated query tokens are AND-matched, each against the user body,
  title, folder name, then the last assistant answer of every turn, then the
  session ID (tokens of 5+ chars). --folder/--agent/--profile are AND'd with the
  query; repeating an option OR's its values. Folder matches the cwd basename
  exactly. Results are most-recent first, capped by --limit (0 = no cap).

NOT SUPPORTED:
  Keyword OR (all tokens are AND); phrase/adjacency matching (quoting a query
  changes nothing — \"a b\" matches the same as a b); negation, regex, and
  substring folder matching.

EXAMPLES:
  s7s session search \"final message\"
  s7s session search test --folder vqs-gw --folder vqs-api --agent codex --agent claude
  s7s session search rename --profile builtin-claude --limit 50")]
pub struct SearchArgs {
    /// Keyword query; space-separated tokens are AND-matched
    #[arg(required = true, num_args = 1.., value_name = "QUERY")]
    pub query: Vec<String>,
    /// Restrict to folder name(s) (cwd basename, exact match; repeatable → OR)
    #[arg(long, value_name = "NAME")]
    pub folder: Vec<String>,
    /// Restrict to agent(s) (repeatable → OR)
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Vec<String>,
    /// Restrict to profile ID(s) (repeatable → OR)
    #[arg(long, value_name = "ID")]
    pub profile: Vec<String>,
    /// Maximum number of results, most recent first (0 = no limit)
    #[arg(long, default_value_t = 20, value_name = "N")]
    pub limit: usize,
}

/// List sessions with no keyword, narrowed by filters only.
#[derive(Args, Debug)]
#[command(after_help = "\
FILTERS:
  All filters are AND'd; repeating an option OR's its values. Folder matches the
  cwd basename exactly. With no filter at all every session is listed, so
  --limit applies (0 = no cap). Results are most-recent first.

USE search INSTEAD WHEN:
  A keyword is known. `list` exists for \"the recent sessions of this folder\",
  which `search` cannot express because its query is mandatory.

EXAMPLES:
  s7s session list --folder ular-s7s --limit 10
  s7s session list --agent codex --agent claude
  s7s session list --profile builtin-claude --limit 0")]
pub struct ListArgs {
    /// Restrict to folder name(s) (cwd basename, exact match; repeatable → OR)
    #[arg(long, value_name = "NAME")]
    pub folder: Vec<String>,
    /// Restrict to agent(s) (repeatable → OR)
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Vec<String>,
    /// Restrict to profile ID(s) (repeatable → OR)
    #[arg(long, value_name = "ID")]
    pub profile: Vec<String>,
    /// Maximum number of results, most recent first (0 = no limit)
    #[arg(long, default_value_t = 20, value_name = "N")]
    pub limit: usize,
}

/// Set the display title of one session.
#[derive(Args, Debug)]
#[command(after_help = "\
STORAGE:
  The title is written to the storage of the agent that owns the session, under
  the config root of the session's own profile. Claude is renamed through its
  CLI first and verified against the transcript; Codex and Antigravity are
  written directly because their CLI rename paths are unverified.

VERIFICATION:
  The stored title is re-read after the write and printed. A rename that reports
  success but leaves the stored title unchanged exits non-zero — an exit code
  from the agent CLI is never trusted on its own.

EXAMPLES:
  s7s session rename 019f36e8-9157-7c63-bee8-8937a6314982 \"cache rebuild bug\"
  s7s session rename 019f36e8-9157-7c63-bee8-8937a6314982 \"rewind parity\" --agent codex")]
pub struct RenameArgs {
    /// Full session ID to rename
    pub session_id: String,
    /// New display title (single line; surrounding whitespace is trimmed)
    pub title: String,
    /// Restrict resolution to one agent
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Option<String>,
    /// Restrict resolution to one profile ID (e.g. builtin-claude)
    #[arg(long)]
    pub profile: Option<String>,
}

/// Delete one session's on-disk artifacts.
#[derive(Args, Debug)]
#[command(after_help = "\
IRREVERSIBLE:
  The transcript file is removed, not archived, and s7s keeps no copy. For
  Antigravity the conversation metadata entry and the sqlite sidecars go too.
  There is no undo.

CONFIRMATION:
  Without --yes nothing is deleted: the target is printed and the command exits
  non-zero. Pass --yes only after the printed target has been checked.

EXAMPLES:
  s7s session delete 019f36e8-9157-7c63-bee8-8937a6314982
  s7s session delete 019f36e8-9157-7c63-bee8-8937a6314982 --yes")]
pub struct DeleteArgs {
    /// Full session ID to delete
    pub session_id: String,
    /// Restrict resolution to one agent
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Option<String>,
    /// Restrict resolution to one profile ID (e.g. builtin-claude)
    #[arg(long)]
    pub profile: Option<String>,
    /// Actually delete; without it the target is only printed
    #[arg(long)]
    pub yes: bool,
}

/// Executes the session subcommand. Returns the process exit code.
pub fn run(args: &SessionArgs) -> i32 {
    match &args.command {
        SessionCommand::Show(a) => run_show(a),
        SessionCommand::Search(a) => run_search(a),
        SessionCommand::List(a) => run_list(a),
        SessionCommand::Rename(a) => run_rename(a),
        SessionCommand::Delete(a) => run_delete(a),
    }
}

/// Shared resolution for the single-session subcommands (`show`, `rename`,
/// `delete`): validates a requested profile, parses `--agent`, runs a quiet
/// scan, then resolves the full session ID.
///
/// On failure the diagnostics are already printed and the returned value is the
/// exit code the caller must propagate.
fn resolve_target(
    session_id: &str,
    agent: Option<&str>,
    profile: Option<&str>,
) -> Result<(ProfileStore, Session), i32> {
    let profiles = ProfileStore::load();

    // A requested-but-missing profile must fail up front (account safety):
    // never scan and silently resolve against some other profile.
    if let Some(profile_id) = profile {
        if profiles.find(profile_id).is_none() {
            eprintln!("error: profile '{profile_id}' does not exist.");
            eprintln!("hint: known profile IDs: {}", known_profile_ids(&profiles));
            return Err(1);
        }
    }

    let agent: Option<Agent> = match agent {
        // Values are validated by clap; parse_agent stays as a defensive check.
        Some(raw) => match resolve::parse_agent(raw) {
            Some(a) => Some(a),
            None => {
                eprintln!("error: unknown agent '{raw}' (claude|codex|antigravity)");
                return Err(2);
            }
        },
        None => None,
    };

    // Quiet incremental scan (no TUI, no spinner). Uses the same mtime cache as
    // the TUI, so repeat queries are cheap.
    let result = crate::scan::scan(&profiles.profiles, false);

    let query = resolve::Query {
        session_id,
        agent,
        profile_id: profile,
    };
    match resolve::resolve(&result.sessions, &query) {
        Ok(s) => Ok((profiles, s.clone())),
        Err(resolve::ResolveError::NotFound) => {
            eprintln!("error: no session found for ID '{session_id}'.");
            eprintln!(
                "hint: use the full session ID; check constraints (--agent/--profile) or \
                 refresh with `s7s --rebuild-cache` if the session is brand new."
            );
            Err(1)
        }
        Err(resolve::ResolveError::Ambiguous(candidates)) => {
            eprintln!(
                "error: session ID '{}' matches {} sessions; disambiguate with --agent/--profile:",
                session_id,
                candidates.len()
            );
            for c in candidates {
                eprintln!(
                    "  --agent {} --profile '{}'  ({})",
                    c.agent.key(),
                    c.profile_id,
                    c.title
                );
            }
            Err(1)
        }
    }
}

/// Renders one session's context (reference / --turn / --bootstrap).
fn run_show(args: &ShowArgs) -> i32 {
    if let Some(turn) = args.turn {
        if turn == 0 {
            eprintln!("error: --turn is 1-based; 0 is not a valid turn number");
            return 2;
        }
    }

    let session = match resolve_target(
        &args.session_id,
        args.agent.as_deref(),
        args.profile.as_deref(),
    ) {
        Ok((_, session)) => session,
        Err(code) => return code,
    };

    let ctx = session_context::load(&session);

    // Bootstrap must never claim success when the expected full context could
    // not be parsed; the bootstrap prompt tells the agent to report failures.
    if args.bootstrap && ctx.completeness != ContextCompleteness::Full {
        eprintln!(
            "error: full context could not be read ({}).",
            ctx.completeness.label()
        );
        eprintln!("hint: the source transcript may be missing or in an unsupported format.");
        return 1;
    }

    let output = if let Some(turn) = args.turn {
        match render::render_turn(&ctx, turn, args.user_only) {
            Ok(out) => out,
            Err(msg) => {
                eprintln!("error: {msg}");
                return 1;
            }
        }
    } else if args.bootstrap {
        render::render_bootstrap(&ctx, args.user_only)
    } else {
        render::render_reference(&ctx, args.user_only)
    };

    println!("{output}");
    0
}

/// Lists sessions matching a keyword (+ optional folder/agent/profile filters).
fn run_search(args: &SearchArgs) -> i32 {
    let profiles = ProfileStore::load();

    // Warn (don't fail) on an unknown --profile: search is a discovery tool, so
    // an empty result set from a typo is more confusing than an up-front notice.
    for profile_id in &args.profile {
        if profiles.find(profile_id).is_none() {
            eprintln!("warning: profile '{profile_id}' does not exist (ignored).");
            eprintln!("hint: known profile IDs: {}", known_profile_ids(&profiles));
        }
    }

    let agents: HashSet<Agent> = args
        .agent
        .iter()
        // clap validates the values; parse_agent stays as a defensive check.
        .filter_map(|a| resolve::parse_agent(a))
        .collect();

    let filter = Filter {
        keyword: args.query.join(" "),
        agents,
        folders: args.folder.iter().cloned().collect(),
        profile_ids: args.profile.iter().cloned().collect(),
    };

    // Quiet incremental scan (shares the TUI mtime cache); scan() already sorts
    // by semantic activity, and filter::apply preserves that order.
    let result = crate::scan::scan(&profiles.profiles, false);
    let indices = crate::filter::apply(&result.sessions, &filter);

    if indices.is_empty() {
        println!("No sessions matched.");
        return 0;
    }

    print_session_rows(&result.sessions, &indices, args.limit, "match(es)");
    0
}

/// Lists sessions narrowed by filters only, with no keyword.
fn run_list(args: &ListArgs) -> i32 {
    let profiles = ProfileStore::load();

    // Warn (don't fail) on an unknown --profile, matching `search`: an empty
    // result set from a typo is more confusing than an up-front notice.
    for profile_id in &args.profile {
        if profiles.find(profile_id).is_none() {
            eprintln!("warning: profile '{profile_id}' does not exist (ignored).");
            eprintln!("hint: known profile IDs: {}", known_profile_ids(&profiles));
        }
    }

    let agents: HashSet<Agent> = args
        .agent
        .iter()
        // clap validates the values; parse_agent stays as a defensive check.
        .filter_map(|a| resolve::parse_agent(a))
        .collect();

    // An empty keyword matches every session, so this is the filter-only view.
    let filter = Filter {
        keyword: String::new(),
        agents,
        folders: args.folder.iter().cloned().collect(),
        profile_ids: args.profile.iter().cloned().collect(),
    };

    let result = crate::scan::scan(&profiles.profiles, false);
    let indices = crate::filter::apply(&result.sessions, &filter);

    if indices.is_empty() {
        println!("No sessions matched.");
        return 0;
    }

    print_session_rows(&result.sessions, &indices, args.limit, "session(s)");
    0
}

/// Sets one session's display title, then re-reads the stored title to confirm
/// the write actually landed.
fn run_rename(args: &RenameArgs) -> i32 {
    let (profiles, session) = match resolve_target(
        &args.session_id,
        args.agent.as_deref(),
        args.profile.as_deref(),
    ) {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    // Metadata paths and the Claude CLI env derive from the owning profile;
    // never fall back to the default root (wrong account store for extra
    // profiles).
    let Some(profile) = profiles.find(&session.profile_id).cloned() else {
        eprintln!(
            "error: profile '{}' of this session no longer exists.",
            session.profile_id
        );
        eprintln!("hint: known profile IDs: {}", known_profile_ids(&profiles));
        return 1;
    };

    let before = session.title();
    if let Err(err) = crate::rename::rename_session(&profile, &session, &args.title) {
        eprintln!("error: rename failed: {err}");
        return 1;
    }

    // An exit code is never trusted on its own: re-scan and compare the stored
    // title against what was asked for.
    let expected = crate::model::one_line(&args.title).trim().to_string();
    match reread_title(&profiles, &session) {
        Some(stored) if stored == expected => {
            println!("Renamed [{}] {}", session.agent.key(), session.id);
            println!("  before: {}", crate::model::one_line(&before));
            println!("  after:  {stored}");
            0
        }
        Some(stored) => {
            eprintln!(
                "error: rename reported success but the stored title is still '{}'.",
                crate::model::one_line(&stored)
            );
            eprintln!(
                "hint: the agent CLI may have changed its title storage; see \
                 docs/session-title-compat.md."
            );
            1
        }
        None => {
            eprintln!("error: the session could not be re-read after the rename.");
            eprintln!("hint: verify the storage file directly before trusting the result.");
            1
        }
    }
}

/// Re-reads one session's stored title after a write. Returns `None` when the
/// session is no longer resolvable (a missing store, not an unchanged title).
fn reread_title(profiles: &ProfileStore, session: &Session) -> Option<String> {
    let result = crate::scan::scan(&profiles.profiles, false);
    let query = resolve::Query {
        session_id: &session.id,
        agent: Some(session.agent),
        profile_id: Some(&session.profile_id),
    };
    resolve::resolve(&result.sessions, &query)
        .ok()
        .map(|s| s.title())
}

/// Deletes one session's on-disk artifacts. Without `--yes` the target is only
/// printed, because the removal cannot be undone.
fn run_delete(args: &DeleteArgs) -> i32 {
    let (profiles, session) = match resolve_target(
        &args.session_id,
        args.agent.as_deref(),
        args.profile.as_deref(),
    ) {
        Ok(pair) => pair,
        Err(code) => return code,
    };

    let source = session
        .source_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(source path missing)".to_string());

    if !args.yes {
        println!("Would delete this session (nothing was removed):");
        print_session_row(&session);
        println!("    file: {source}");
        println!("\nRe-run with --yes to delete it. This cannot be undone.");
        return 1;
    }

    if let Err(err) = crate::session_delete::delete_session_artifacts(&profiles, &session) {
        eprintln!("error: delete failed: {err}");
        return 1;
    }

    println!("Deleted this session:");
    print_session_row(&session);
    println!("    file: {source}");
    0
}

/// Prints one session as an identity line plus its title. Every subcommand that
/// names a session uses this, so a session looks the same in a list, in a delete
/// preview, and in a delete report.
fn print_session_row(session: &Session) {
    println!(
        "  {}  {}/{}  [{}]  {}  Q{}",
        session.id,
        session.agent.key(),
        session.profile_id,
        session.folder,
        session.updated_str(),
        session.user_turns.len(),
    );
    println!("    {}", crate::model::one_line(&session.title()));
}

/// Prints a capped, most-recent-first block of session rows with a count header.
/// Shared by `list` and `search` so both emit the same shape.
fn print_session_rows(sessions: &[Session], indices: &[usize], limit: usize, noun: &str) {
    let total = indices.len();
    let shown = if limit == 0 { total } else { total.min(limit) };

    if total == shown {
        println!("{total} {noun}, most recent first:\n");
    } else {
        println!("{total} {noun}, most recent first (showing {shown}):\n");
    }

    for &idx in indices.iter().take(shown) {
        print_session_row(&sessions[idx]);
    }

    println!("\nRead one:  s7s session show <ID> --agent <AGENT> --profile <PROFILE> [--turn N]");
}

/// Comma-separated list of configured profile IDs (for error/warning hints).
fn known_profile_ids(profiles: &ProfileStore) -> String {
    profiles
        .profiles
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
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
}
