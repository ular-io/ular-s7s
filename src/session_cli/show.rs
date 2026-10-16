//! `s7s session show`: render one previous session's context.

use super::{resolve_target, Target};
use crate::session_context::{self, render, ContextCompleteness};
use clap::Args;

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

pub(crate) fn run(args: &ShowArgs) -> i32 {
    if let Some(turn) = args.turn {
        if turn == 0 {
            eprintln!("error: --turn is 1-based; 0 is not a valid turn number");
            return 2;
        }
    }

    let Target { index, session, .. } = match resolve_target(
        &args.session_id,
        args.agent.as_deref(),
        args.profile.as_deref(),
    ) {
        Ok(target) => target,
        Err(code) => return code,
    };

    // Resolved against the index so a context source that no longer exists is
    // reported as unavailable instead of as a live reference.
    let ctx = session_context::load_in_index(&session, &index);

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
