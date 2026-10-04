//! Non-interactive folder changes for an explicit, fully validated batch.

use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use crate::session_context::resolve;
use anyhow::{anyhow, bail, Result};
use clap::Args;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Args, Debug)]
#[command(after_help = "\
SCOPE:
  Changes only where the sessions open on their next s7s resume. Files and agent
  transcripts are not moved or rewritten; running sessions keep their current
  directory. Antigravity is not supported.

VALIDATION:
  Explicit full IDs only. All targets must resolve uniquely before any folder
  mapping is saved; use --agent/--profile to disambiguate. Duplicate IDs count
  once. The destination must exist; relative paths use the command's directory.
  The complete batch is merged, saved once, and read back under a shared lock.
  --dry-run prints the same validated plan without saving folder mappings.

EXAMPLES:
  s7s session change-folder <ID> --to ~/projects/app
  s7s session change-folder <ID1> <ID2> --to ./app --dry-run
  s7s session change-folder <ID> --to ./app --agent codex --profile builtin-codex")]
pub struct ChangeFolderArgs {
    /// Full session IDs to change (duplicates are ignored)
    #[arg(required = true, num_args = 1.., value_name = "ID")]
    pub session_ids: Vec<String>,
    /// Existing destination directory (relative to the command's directory)
    #[arg(long, value_name = "DIR")]
    pub to: PathBuf,
    /// Restrict resolution to one agent
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Option<String>,
    /// Restrict resolution to one profile ID
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,
    /// Validate and display the plan without saving folder mappings
    #[arg(long)]
    pub dry_run: bool,
}

pub(super) fn run(args: &ChangeFolderArgs) -> i32 {
    let profiles = ProfileStore::load();
    if let Err(err) = validate_profile(&profiles, args.profile.as_deref()) {
        eprintln!("error: {err:#}");
        eprintln!(
            "hint: known profile IDs: {}",
            super::known_profile_ids(&profiles)
        );
        return 1;
    }
    let index = crate::scan::scan(&profiles.profiles, false);
    match run_in_index(
        args,
        &profiles,
        &index.sessions,
        &crate::config::session_workspaces_path(),
    ) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err:#}");
            1
        }
    }
}

fn validate_profile(profiles: &ProfileStore, id: Option<&str>) -> Result<()> {
    if let Some(id) = id {
        if profiles.find(id).is_none() {
            bail!("profile '{id}' does not exist");
        }
    }
    Ok(())
}

fn targets<'a>(args: &ChangeFolderArgs, index: &'a [Session]) -> Result<Vec<&'a Session>> {
    let agent: Option<Agent> = args
        .agent
        .as_deref()
        .map(|s| resolve::parse_agent(s).ok_or_else(|| anyhow!("unknown agent '{s}'")))
        .transpose()?;
    let mut seen = HashSet::new();
    let mut sessions = Vec::new();
    for id in &args.session_ids {
        if !seen.insert(id) {
            continue;
        }
        let query = resolve::Query {
            session_id: id,
            agent,
            profile_id: args.profile.as_deref(),
        };
        let session = match resolve::resolve(index, &query) {
            Ok(session) => session,
            Err(resolve::ResolveError::NotFound) => bail!("no session found for ID '{id}'"),
            Err(resolve::ResolveError::Ambiguous(candidates)) => {
                let candidates = candidates
                    .iter()
                    .map(|c| format!("--agent {} --profile '{}'", c.agent.key(), c.profile_id))
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("session ID '{id}' is ambiguous; choose {candidates}");
            }
        };
        crate::session_folder::validate_agent(session.agent)?;
        sessions.push(session);
    }
    if sessions.is_empty() {
        bail!("No sessions selected");
    }
    Ok(sessions)
}

fn run_in_index(
    args: &ChangeFolderArgs,
    profiles: &ProfileStore,
    index: &[Session],
    store: &Path,
) -> Result<()> {
    validate_profile(profiles, args.profile.as_deref())?;
    let sessions = targets(args, index)?;
    let input = crate::config::expand(&args.to.to_string_lossy());
    let folder = crate::session_folder::validate_folder(&input)?;
    if !args.dry_run {
        crate::session_folder::change(&sessions, &folder, store)?;
    }
    let action = if args.dry_run {
        "Would change"
    } else {
        "Changed"
    };
    println!("{action} folder for {} session(s):", sessions.len());
    for session in sessions {
        println!(
            "  [{}/{}] {}",
            session.agent.key(),
            session.profile_id,
            session.id
        );
        println!("    before: {}", session.cwd.display());
        println!("    after:  {}", folder.display());
    }
    println!("\nApplies on the next s7s resume; running sessions and transcripts are unchanged.");
    Ok(())
}

#[cfg(test)]
mod tests;
