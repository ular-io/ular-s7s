//! `s7s session delete`: remove one session's on-disk artifacts.

use super::search::print_session_row;
use super::{resolve_target, Target};
use clap::Args;

#[derive(Args, Debug)]
#[command(after_help = "\
IRREVERSIBLE:
  The transcript files are removed, not archived, and s7s keeps no copy. For
  Codex that is every rollout file of the thread (a rewind splits it) plus its
  rows in the state and thread-history databases; for Antigravity the
  conversation metadata entry and the sqlite sidecars go too. There is no undo.

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

/// Deletes one session's on-disk artifacts. Without `--yes` the target is only
/// printed, because the removal cannot be undone.
pub(crate) fn run(args: &DeleteArgs) -> i32 {
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

    // A live process would keep writing the files this would remove.
    if let Some(entry) = profiles
        .find(&session.profile_id)
        .and_then(|p| crate::agent_status::live_holder(p, &session))
    {
        crate::agent_status::print_live_refusal(&entry, "delete");
        return 1;
    }

    let files = crate::session_delete::transcript_files(&session);

    if !args.yes {
        println!("Would delete this session (nothing was removed):");
        print_session_row(&session);
        if files.is_empty() {
            println!("    file: (source path missing)");
        }
        for file in &files {
            println!("    file: {}", file.display());
        }
        println!("\nRe-run with --yes to delete it. This cannot be undone.");
        return 1;
    }

    let outcome = match crate::session_delete::delete_session(
        &profiles,
        &session,
        &crate::config::bookmarks_path(),
    ) {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("error: delete failed: {err}");
            return 1;
        }
    };

    println!("Deleted this session:");
    print_session_row(&session);
    for file in &files {
        println!("    file: {}", file.display());
    }
    if let Some(warning) = outcome.bookmark_warning {
        eprintln!("warning: session deleted; {warning}");
    }
    0
}
