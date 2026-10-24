//! CLI adapter for the same bookmark store used by the TUI.

use super::{resolve_target, search, ListArgs};
use crate::bookmarks::BookmarkStore;
use clap::{Args, Subcommand};

#[derive(Args, Debug)]
#[command(after_help = "\
STORAGE:
  Uses the TUI bookmark store. Agent-owned session files are unchanged.
  Add and remove are idempotent. In a running TUI, press Ctrl+U to reload.

EXAMPLES:
  s7s session bookmark add <ID> --agent codex
  s7s session bookmark remove <ID> --profile builtin-codex
  s7s session bookmark list --folder my-project --limit 0")]
pub struct BookmarkArgs {
    #[command(subcommand)]
    command: BookmarkCommand,
}

#[derive(Subcommand, Debug)]
enum BookmarkCommand {
    /// Bookmark one session (succeeds if already bookmarked)
    Add(BookmarkTargetArgs),
    /// Remove one session's bookmark, keeping the session itself
    Remove(BookmarkTargetArgs),
    /// List available bookmarked sessions, most recent first
    #[command(after_help = "\
FILTERS:
  Only bookmarked sessions in scanned profiles are listed. All filters are
  AND'd; repeating an option OR's its values. Folder matches the cwd basename
  exactly. Results are most-recent first (default limit 20; 0 = no cap).
  Unavailable sessions keep their stored bookmarks but do not produce rows.

EXAMPLES:
  s7s session bookmark list
  s7s session bookmark list --folder my-project --limit 0
  s7s session bookmark list --agent codex --profile builtin-codex")]
    List(ListArgs),
}

#[derive(Args, Debug)]
struct BookmarkTargetArgs {
    /// Full session ID
    session_id: String,
    /// Restrict resolution to one agent
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    agent: Option<String>,
    /// Restrict resolution to one profile ID
    #[arg(long)]
    profile: Option<String>,
}

pub(crate) fn run(args: &BookmarkArgs) -> i32 {
    let (args, bookmarked) = match &args.command {
        BookmarkCommand::List(args) => return search::run_bookmark_list(args),
        BookmarkCommand::Add(args) => (args, true),
        BookmarkCommand::Remove(args) => (args, false),
    };
    let target = match resolve_target(
        &args.session_id,
        args.agent.as_deref(),
        args.profile.as_deref(),
    ) {
        Ok(target) => target,
        Err(code) => return code,
    };
    let path = crate::config::bookmarks_path();
    let result = if bookmarked {
        BookmarkStore::add(&path, &target.session)
    } else {
        BookmarkStore::remove(&path, &target.session)
    };
    if let Err(error) = result {
        eprintln!("error: cannot update bookmark: {error:#}");
        return 1;
    }
    println!(
        "{} [{}] {}/{}",
        if bookmarked {
            "Bookmarked"
        } else {
            "Removed bookmark from"
        },
        target.session.agent.key(),
        target.session.profile_id,
        target.session.id,
    );
    0
}
