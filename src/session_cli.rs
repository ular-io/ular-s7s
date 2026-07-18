//! `s7s session` subcommand: query context from a previous session without TUI.
//!
//! Output discipline: primary context goes to stdout, errors/diagnostics to
//! stderr, no ANSI styling, no scan spinner. Exit codes: 0 success, 2 invalid
//! arguments (clap), 1 for lookup/parse failures.

use crate::model::Agent;
use crate::profile::ProfileStore;
use crate::session_context::{self, render, resolve, ContextCompleteness};
use clap::Args;

/// Read context from a previous session.
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
  s7s session 019f36e8-9157-7c63-bee8-8937a6314982
  s7s session 019f36e8-9157-7c63-bee8-8937a6314982 --user-only
  s7s session 019f36e8-9157-7c63-bee8-8937a6314982 --turn 7
  s7s session 019f36e8-9157-7c63-bee8-8937a6314982 --agent codex --profile builtin-codex --bootstrap")]
pub struct SessionArgs {
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

/// Executes the session query. Returns the process exit code.
pub fn run(args: &SessionArgs) -> i32 {
    let profiles = ProfileStore::load();

    // A requested-but-missing profile must fail up front (account safety):
    // never scan and silently resolve against some other profile.
    if let Some(profile_id) = args.profile.as_deref() {
        if profiles.find(profile_id).is_none() {
            eprintln!("error: profile '{profile_id}' does not exist.");
            eprintln!(
                "hint: known profile IDs: {}",
                profiles
                    .profiles
                    .iter()
                    .map(|p| p.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
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

    #[test]
    fn parses_all_supported_options() {
        let s = parse(&[
            "s7s",
            "session",
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
    fn rejects_unknown_options_and_invalid_values() {
        // Unknown options must fail instead of being ignored.
        assert!(parse(&["s7s", "session", "abc", "--unknown-flag"]).is_err());
        // Missing session id fails.
        assert!(parse(&["s7s", "session"]).is_err());
        // Invalid agent value fails at parse time.
        assert!(parse(&["s7s", "session", "abc", "--agent", "gpt"]).is_err());
        // Non-numeric turn fails.
        assert!(parse(&["s7s", "session", "abc", "--turn", "x"]).is_err());
    }

    #[test]
    fn bootstrap_conflicts_with_turn() {
        assert!(parse(&["s7s", "session", "abc", "--bootstrap", "--turn", "1"]).is_err());
        assert!(parse(&["s7s", "session", "abc", "--bootstrap"]).is_ok());
    }
}
