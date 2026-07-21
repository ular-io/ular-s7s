//! `s7s session` subcommand group: query previous sessions without the TUI.
//!
//! Subcommands:
//!   show <id>       Render one session's context (reference / --turn / --bootstrap).
//!   search <query>  List sessions matching a keyword (+ folder/agent/profile filters).
//!
//! Output discipline: primary output goes to stdout, errors/diagnostics to
//! stderr, no ANSI styling, no scan spinner. Exit codes: 0 success, 2 invalid
//! arguments (clap), 1 for lookup/parse failures.

use crate::filter::Filter;
use crate::model::Agent;
use crate::profile::ProfileStore;
use crate::session_context::{self, render, resolve, ContextCompleteness};
use clap::{Args, Subcommand};
use std::collections::HashSet;

/// Query previous sessions (context and search).
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

/// Executes the session subcommand. Returns the process exit code.
pub fn run(args: &SessionArgs) -> i32 {
    match &args.command {
        SessionCommand::Show(a) => run_show(a),
        SessionCommand::Search(a) => run_search(a),
    }
}

/// Renders one session's context (reference / --turn / --bootstrap).
fn run_show(args: &ShowArgs) -> i32 {
    let profiles = ProfileStore::load();

    // A requested-but-missing profile must fail up front (account safety):
    // never scan and silently resolve against some other profile.
    if let Some(profile_id) = args.profile.as_deref() {
        if profiles.find(profile_id).is_none() {
            eprintln!("error: profile '{profile_id}' does not exist.");
            eprintln!("hint: known profile IDs: {}", known_profile_ids(&profiles));
            return 1;
        }
    }

    let agent: Option<Agent> = match args.agent.as_deref() {
        // Values are validated by clap; parse_agent stays as a defensive check.
        Some(raw) => match resolve::parse_agent(raw) {
            Some(a) => Some(a),
            None => {
                eprintln!("error: unknown agent '{raw}' (claude|codex|antigravity)");
                return 2;
            }
        },
        None => None,
    };

    if let Some(turn) = args.turn {
        if turn == 0 {
            eprintln!("error: --turn is 1-based; 0 is not a valid turn number");
            return 2;
        }
    }

    // Quiet incremental scan (no TUI, no spinner). Uses the same mtime cache as
    // the TUI, so repeat queries are cheap.
    let result = crate::scan::scan(&profiles.profiles, false);

    let query = resolve::Query {
        session_id: &args.session_id,
        agent,
        profile_id: args.profile.as_deref(),
    };
    let session = match resolve::resolve(&result.sessions, &query) {
        Ok(s) => s,
        Err(resolve::ResolveError::NotFound) => {
            eprintln!("error: no session found for ID '{}'.", args.session_id);
            eprintln!(
                "hint: use the full session ID; check constraints (--agent/--profile) or \
                 refresh with `s7s --rebuild-cache` if the session is brand new."
            );
            return 1;
        }
        Err(resolve::ResolveError::Ambiguous(candidates)) => {
            eprintln!(
                "error: session ID '{}' matches {} sessions; disambiguate with --agent/--profile:",
                args.session_id,
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
            return 1;
        }
    };

    let ctx = session_context::load(session);

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
    // most-recent first, and filter::apply preserves that order.
    let result = crate::scan::scan(&profiles.profiles, false);
    let indices = crate::filter::apply(&result.sessions, &filter);

    let total = indices.len();
    let shown = if args.limit == 0 {
        total
    } else {
        total.min(args.limit)
    };

    if total == 0 {
        println!("No sessions matched.");
        return 0;
    }

    if total == shown {
        println!("{total} match(es), most recent first:\n");
    } else {
        println!("{total} match(es), most recent first (showing {shown}):\n");
    }

    for &idx in indices.iter().take(shown) {
        let s = &result.sessions[idx];
        println!(
            "  {}  {}/{}  [{}]  {}  Q{}",
            s.id,
            s.agent.key(),
            s.profile_id,
            s.folder,
            s.updated_str(),
            s.user_turns.len(),
        );
        println!("    {}", crate::model::one_line(&s.title()));
    }

    println!("\nRead one:  s7s session show <ID> --agent <AGENT> --profile <PROFILE> [--turn N]");
    0
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
}
