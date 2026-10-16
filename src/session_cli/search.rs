//! `s7s session search` and `list`: query sessions by keyword or filters.

use super::known_profile_ids;
use crate::filter::Filter;
use crate::full_search::{self, Query, SessionMatch};
use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use crate::session_context::resolve;
use clap::Args;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Args, Debug)]
#[command(after_help = "\
MATCHING:
  Space-separated query tokens are AND-matched, each against the user body,
  title, folder name, then the last assistant answer of every turn, then the
  session ID (tokens of 5+ chars).

  With --full, every assistant message, tool call argument, and tool result is
  searched in addition to user turns, using the shared detailed parser. Each
  match displays the best snippet and hit count.

  --folder/--agent/--profile are AND'd with the query; repeating an option
  OR's its values. Folder matches the cwd basename exactly. Results are
  most-recent first, capped by --limit (0 = no cap).

NOT SUPPORTED:
  Keyword OR (all tokens are AND); phrase/adjacency matching (quoting a query
  changes nothing — \"a b\" matches the same as a b); negation, regex, and
  substring folder matching.

EXAMPLES:
  s7s session search \"final message\"
  s7s session search \"astro-azcopy-probe-\" --full
  s7s session search test --folder vqs-gw --folder vqs-api --agent codex --agent claude
  s7s session search \"cargo build\" --full --limit 10")]
pub struct SearchArgs {
    /// Keyword query; space-separated tokens are AND-matched
    #[arg(required = true, num_args = 1.., value_name = "QUERY")]
    pub query: Vec<String>,
    /// Search across all session content (user turns, assistant messages, tool calls, and tool results)
    #[arg(long)]
    pub full: bool,
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

/// Lists sessions matching a keyword (+ optional folder/agent/profile filters).
pub(crate) fn run_search(args: &SearchArgs) -> i32 {
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

    if args.full {
        return run_full_search(args, &profiles, agents);
    }

    let filter = Filter {
        keyword: args.query.join(" "),
        agents,
        folders: args.folder.iter().cloned().collect(),
        profile_ids: args.profile.iter().cloned().collect(),
        bookmarked_only: false,
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

/// Searches all session content across candidate sessions matching metadata filters.
fn run_full_search(args: &SearchArgs, profiles: &ProfileStore, agents: HashSet<Agent>) -> i32 {
    let query_text = args.query.join(" ");
    let Some(query) = Query::parse(&query_text) else {
        eprintln!("error: query holds no valid search token.");
        return 2;
    };

    // Candidate sessions narrowed by metadata filters first (parity with TUI).
    let filter = Filter {
        keyword: String::new(),
        agents,
        folders: args.folder.iter().cloned().collect(),
        profile_ids: args.profile.iter().cloned().collect(),
        bookmarked_only: false,
    };

    let result = crate::scan::scan(&profiles.profiles, false);
    let candidate_indices = crate::filter::apply(&result.sessions, &filter);

    if candidate_indices.is_empty() {
        println!("No sessions matched.");
        return 0;
    }

    let jobs: Vec<(usize, Session)> = candidate_indices
        .iter()
        .map(|&idx| (idx, result.sessions[idx].clone()))
        .collect();

    let cancel = Arc::new(AtomicBool::new(false));
    let matches: Vec<(usize, SessionMatch)> = full_search::spawn(jobs, query, cancel)
        .into_iter()
        .filter_map(|(idx, m)| m.map(|mat| (idx, mat)))
        .collect();

    if matches.is_empty() {
        println!("No sessions matched.");
        return 0;
    }

    let total = matches.len();
    let shown = if args.limit == 0 {
        total
    } else {
        total.min(args.limit)
    };

    if total == shown {
        println!("{total} full-content match(es), most recent first:\n");
    } else {
        println!("{total} full-content match(es), most recent first (showing {shown}):\n");
    }

    for (idx, m) in matches.iter().take(shown) {
        let session = &result.sessions[*idx];
        print_session_row(session);
        println!(
            "    [Match · Turn {} · {} · {} hit(s)]",
            m.turn + 1,
            m.field.label(),
            m.hits
        );
        for line in &m.lines {
            println!("      {}", line.text);
        }
        println!();
    }

    println!("Read one:  s7s session show <ID> --agent <AGENT> --profile <PROFILE> [--turn N]");
    0
}

/// Lists sessions narrowed by filters only, with no keyword.
pub(crate) fn run_list(args: &ListArgs) -> i32 {
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
        bookmarked_only: false,
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

pub(crate) fn print_session_row(session: &Session) {
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
