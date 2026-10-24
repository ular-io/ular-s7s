//! `s7s session` subcommand group: query and manage previous sessions without the TUI.
//!
//! Subcommands:
//!   show <id>       Render one session's context (reference / --turn / --bootstrap).
//!   search <query>  List sessions matching a keyword (+ folder/agent/profile filters).
//!   list            List sessions by filter only, with no keyword.
//!   bookmark        Add, remove, or list session bookmarks.
//!   rename <id> <t> Set one session's display title.
//!   change-folder  Set the next resume folder for explicit session IDs.
//!   delete <id>     Remove one session's on-disk artifacts (irreversible).
//!   handoff         Park a task in a new session to pick up later.
//!
//! Output discipline: primary output goes to stdout, errors/diagnostics to
//! stderr, no ANSI styling, no scan spinner. Exit codes: 0 success, 2 invalid
//! arguments (clap), 1 for lookup/parse failures. `delete` without `--yes` also
//! exits 1: nothing was removed, so it must not read as success in a script.

use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use crate::session_context::resolve;
use clap::{Args, Subcommand};

mod bookmark;
pub mod change_folder;
mod delete;
mod handoff;
mod rename;
mod search;
mod show;

#[cfg(test)]
mod tests;

pub use bookmark::BookmarkArgs;
pub use delete::DeleteArgs;
pub use handoff::HandoffArgs;
pub use rename::RenameArgs;
pub use search::{ListArgs, SearchArgs};
pub use show::ShowArgs;

/// Query and manage previous sessions, including next-resume folder changes.
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
    /// Add, remove, or list session bookmarks shared with the TUI
    Bookmark(BookmarkArgs),
    /// Set one session's display title
    Rename(RenameArgs),
    /// Change where explicit Claude or Codex sessions open next time
    ChangeFolder(change_folder::ChangeFolderArgs),
    /// Delete one session's on-disk artifacts (irreversible)
    Delete(DeleteArgs),
    /// Park a task in a new session to resume later
    Handoff(HandoffArgs),
}

pub fn run(args: &SessionArgs) -> i32 {
    match &args.command {
        SessionCommand::Show(a) => show::run(a),
        SessionCommand::Search(a) => search::run_search(a),
        SessionCommand::List(a) => search::run_list(a),
        SessionCommand::Bookmark(a) => bookmark::run(a),
        SessionCommand::Rename(a) => rename::run(a),
        SessionCommand::ChangeFolder(a) => change_folder::run(a),
        SessionCommand::Delete(a) => delete::run(a),
        SessionCommand::Handoff(a) => handoff::run(a),
    }
}

/// One resolved single-session target: the profile store, the whole scanned
/// index (kept so a caller can resolve cross-session references without a second
/// scan), and the session itself.
pub(crate) struct Target {
    pub(crate) profiles: ProfileStore,
    pub(crate) index: Vec<Session>,
    pub(crate) session: Session,
}

/// Shared resolution for the single-session subcommands (`show`, `rename`,
/// `delete`): validates a requested profile, parses `--agent`, runs a quiet
/// scan, then resolves the full session ID.
///
/// On failure the diagnostics are already printed and the returned value is the
/// exit code the caller must propagate.
pub(crate) fn resolve_target(
    session_id: &str,
    agent: Option<&str>,
    profile: Option<&str>,
) -> Result<Target, i32> {
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
        Ok(s) => {
            let session = s.clone();
            Ok(Target {
                profiles,
                index: result.sessions,
                session,
            })
        }
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

pub(crate) fn title_line(title: &str) -> String {
    const MAX: usize = 120;
    let line = crate::model::one_line(title);
    let mut out: String = line.chars().take(MAX).collect();
    if line.chars().count() > MAX {
        out.push('…');
    }
    out
}

/// Comma-separated list of configured profile IDs (for error/warning hints).
pub(crate) fn known_profile_ids(profiles: &ProfileStore) -> String {
    profiles
        .profiles
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
