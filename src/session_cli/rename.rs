//! `s7s session rename`: set one session's display title.

use super::{known_profile_ids, resolve_target, title_line, Target};
use crate::model::Session;
use crate::profile::ProfileStore;
use crate::session_context::resolve;
use clap::Args;

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

/// Sets one session's display title, then re-reads the stored title to confirm
/// the write actually landed.
pub(crate) fn run(args: &RenameArgs) -> i32 {
    let Target {
        profiles, session, ..
    } = match resolve_target(
        &args.session_id,
        args.agent.as_deref(),
        args.profile.as_deref(),
    ) {
        Ok(target) => target,
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

    // A live process would keep appending to the transcript this rename writes.
    if let Some(entry) = crate::agent_status::live_holder(&profile, &session) {
        crate::agent_status::print_live_refusal(&entry, "rename");
        return 1;
    }

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
            // A session the agent never titled carries its whole first message as
            // the title, so the old value is capped rather than dumped.
            println!("  before: {}", title_line(&before));
            println!("  after:  {}", title_line(&stored));
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
