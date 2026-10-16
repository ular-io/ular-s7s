//! `s7s session handoff`: park a task in a new session to resume later.

use super::known_profile_ids;
use crate::model::{Agent, Session};
use crate::profile::ProfileStore;
use crate::session_context::resolve;
use clap::Args;
use std::path::PathBuf;

/// Park a task in a new session.
#[derive(Args, Debug)]
#[command(after_help = "\
BODY:
  Read from stdin unless --body-file is given. Write it as a work order: what the
  task is, what to check, and what counts as done. The new session records it and
  stops; it does not act on it.

WHAT IS ADDED:
  A stop instruction (config.toml `handoff_instruction`, English by default) and,
  when the source is known, the origin session plus an `<s7s-context-bootstrap>`
  envelope so s7s links the two and `ctrl+o` opens the origin.

DEFAULTS:
  --agent/--profile follow the source session; --folder follows its working
  directory, so resuming lands in the project the work belongs to. Without a
  resolvable source, --agent is required and --folder defaults to the current
  directory.

SOURCE:
  --from, else $CLAUDE_CODE_SESSION_ID, else the most recently active session whose
  working directory is the current one. The last of those is a guess and yields no
  link when it misses; --no-source skips the link entirely.

NOT DONE HERE:
  Nothing is tracked or reminded. The parked session sits in the list with a
  single turn (Q1), which is what marks it as not started yet.

EXAMPLES:
  s7s session handoff --title 'rewind parity check' < notes.md
  s7s session handoff --title 'migration risk' --agent codex --body-file notes.md
  s7s session handoff --title 'skill frontmatter' --folder ~/Script --no-source < notes.md")]
pub struct HandoffArgs {
    /// Title of the parked session (a `HAND-OVER: ` prefix is added when absent)
    #[arg(long, value_name = "TITLE")]
    pub title: String,
    /// Agent to park the task in (defaults to the source session's agent)
    #[arg(long, value_parser = ["claude", "codex", "antigravity"])]
    pub agent: Option<String>,
    /// Profile ID to park under (defaults to the source session's profile)
    #[arg(long, value_name = "ID")]
    pub profile: Option<String>,
    /// Existing directory the new session runs in (defaults to the source's cwd)
    #[arg(long, value_name = "DIR")]
    pub folder: Option<PathBuf>,
    /// Full session ID to record as the origin
    #[arg(long, value_name = "ID", conflicts_with = "no_source")]
    pub from: Option<String>,
    /// Record no origin: omit the source block and the context envelope
    #[arg(long)]
    pub no_source: bool,
    /// Read the body from this file instead of stdin
    #[arg(long, value_name = "PATH")]
    pub body_file: Option<PathBuf>,
}

/// Parks a task in a new session and prints how to resume it.
pub(crate) fn run(args: &HandoffArgs) -> i32 {
    let body = match read_handoff_body(args.body_file.as_deref()) {
        Ok(body) => body,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 2;
        }
    };
    if body.trim().is_empty() {
        eprintln!("error: the handoff body is empty.");
        eprintln!("hint: pipe it on stdin, or pass --body-file <PATH>.");
        return 2;
    }

    let profiles = ProfileStore::load();
    let cfg = crate::config::Config::load();
    let result = crate::scan::scan(&profiles.profiles, false);

    // The source decides the defaults, so it is resolved before anything else.
    let source = if args.no_source {
        None
    } else {
        match resolve_handoff_source(args, &result.sessions) {
            Ok(found) => found,
            Err(code) => return code,
        }
    };

    let agent = match handoff_agent(args, source) {
        Ok(agent) => agent,
        Err(code) => return code,
    };

    // The source's profile is inherited only when the target agent is the same
    // one. Carrying a claude profile into a codex handoff would point the rename
    // at another account's config root.
    let requested_profile = args.profile.clone().or_else(|| {
        source
            .filter(|s| s.agent == agent)
            .map(|s| s.profile_id.clone())
    });
    let profile = match handoff_profile(&profiles, agent, requested_profile.as_deref()) {
        Ok(profile) => profile,
        Err(err) => {
            eprintln!("error: {err}");
            eprintln!("hint: known profile IDs: {}", known_profile_ids(&profiles));
            return 1;
        }
    };

    let folder = match handoff_folder(args, source) {
        Ok(folder) => folder,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 2;
        }
    };

    let title = handoff_title(&args.title);
    let request = crate::session_handoff::HandoffRequest {
        agent,
        profile: &profile,
        folder: &folder,
        title: &title,
        body: &body,
        source: source.map(crate::session_handoff::SourceRef::from_session),
        instruction: &cfg.handoff_instruction,
    };

    let outcome = match crate::session_handoff::create(&request) {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("error: handoff failed: {err}");
            return 1;
        }
    };
    if let Some(warning) = &outcome.warning {
        eprintln!("warning: {warning}");
    }

    // Rescan rather than trust the agent's exit: the handoff counts only when the
    // session is actually on disk. It also supplies the record the rename and the
    // resume command need.
    let after = crate::scan::scan(&profiles.profiles, false);
    let parked = after.sessions.iter().find(|s| s.id == outcome.id);

    // claude names the session as it starts; the others are renamed here, where
    // the scan above has already made the new session addressable.
    let mut titled = outcome.titled;
    if !titled {
        match parked {
            Some(session) => match crate::rename::rename_session(&profile, session, &title) {
                Ok(()) => titled = true,
                Err(err) => eprintln!("warning: the title could not be applied: {err}"),
            },
            None => eprintln!(
                "warning: the title could not be applied: the new session is not in the index yet"
            ),
        }
    }

    println!("Parked this handoff:");
    println!(
        "  {}  {}/{}  [{}]",
        outcome.id,
        outcome.agent.key(),
        outcome.profile_id,
        folder.display()
    );
    println!("    {title}");
    if !titled {
        println!("    (untitled — find it by its body until a rename succeeds)");
    }

    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| crate::config::APP_NAME.to_string());
    // The default projection elides the middle of a long body, so the full-text
    // command is printed alongside it.
    println!(
        "\nRead it:   '{exe}' session show '{}' --agent {} --profile '{}' --turn 1 --user-only",
        outcome.id,
        outcome.agent.key(),
        outcome.profile_id
    );
    match parked {
        Some(session) => println!(
            "Resume it: {}",
            crate::resume::preview_command(session, &cfg, Some(&profile))
        ),
        None => {
            eprintln!(
                "warning: the new session is not in the index yet; refresh with `s7s --rebuild-cache`."
            );
        }
    }
    0
}

/// Reads the handoff body from a file, or from stdin when none is given.
fn read_handoff_body(path: Option<&std::path::Path>) -> Result<String, String> {
    match path {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display())),
        None => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|err| format!("cannot read the body from stdin: {err}"))?;
            Ok(buf)
        }
    }
}

/// Resolves the origin session: `--from`, else the environment, else the most
/// recent session of the target agent in the folder.
fn resolve_handoff_source<'s>(
    args: &HandoffArgs,
    sessions: &'s [Session],
) -> Result<Option<&'s Session>, i32> {
    if let Some(id) = args.from.as_deref() {
        // An explicitly named source that cannot be found is an error: silently
        // dropping the link would hide the origin the caller asked to record.
        return match sessions.iter().find(|s| s.id == id) {
            Some(session) => Ok(Some(session)),
            None => {
                eprintln!("error: no session found for --from '{id}'.");
                eprintln!("hint: use the full session ID, or pass --no-source.");
                Err(1)
            }
        };
    }

    if let Ok(id) = std::env::var("CLAUDE_CODE_SESSION_ID") {
        if let Some(session) = sessions.iter().find(|s| s.id == id) {
            return Ok(Some(session));
        }
    }

    // Last resort: the calling session is the most recently active one in this
    // directory. A miss is not an error — the handoff proceeds without a link.
    let cwd = std::env::current_dir().unwrap_or_default();
    Ok(sessions.iter().find(|s| s.cwd == cwd))
}

/// Target agent: the flag, else the source's agent.
fn handoff_agent(args: &HandoffArgs, source: Option<&Session>) -> Result<Agent, i32> {
    if let Some(raw) = args.agent.as_deref() {
        // Values are validated by clap; parse_agent stays as a defensive check.
        return resolve::parse_agent(raw).ok_or_else(|| {
            eprintln!("error: unknown agent '{raw}' (claude|codex|antigravity)");
            2
        });
    }
    match source {
        Some(session) => Ok(session.agent),
        None => {
            eprintln!("error: --agent is required when no source session is known.");
            eprintln!("hint: pass --agent, or --from <ID> to inherit it.");
            Err(2)
        }
    }
}

/// Profile to park under: the requested id, else the agent's first configured
/// profile.
///
/// A profile belonging to a different agent is refused rather than used. Every
/// title store a rename writes derives from `Profile.path`, so a mismatch would
/// write into another account's config root.
pub(crate) fn handoff_profile(
    profiles: &ProfileStore,
    agent: Agent,
    requested: Option<&str>,
) -> Result<crate::profile::Profile, String> {
    match requested {
        Some(id) => match profiles.find(id) {
            Some(profile) if profile.agent == agent => Ok(profile.clone()),
            Some(profile) => Err(format!(
                "profile '{id}' belongs to {}, not {}.",
                profile.agent.key(),
                agent.key()
            )),
            None => Err(format!("profile '{id}' does not exist.")),
        },
        None => profiles
            .profiles
            .iter()
            .find(|p| p.agent == agent)
            .cloned()
            .ok_or_else(|| format!("no profile is configured for agent '{}'.", agent.key())),
    }
}

/// Folder the new session runs in: the flag, else the source's cwd, else the
/// current directory. It must exist, because it becomes the session's cwd.
fn handoff_folder(args: &HandoffArgs, source: Option<&Session>) -> Result<PathBuf, String> {
    let inherited = args.folder.clone().or_else(|| {
        source
            .map(|s| s.cwd.clone())
            .filter(|cwd| !cwd.as_os_str().is_empty())
    });
    let folder = match inherited {
        Some(folder) => folder,
        None => std::env::current_dir().map_err(|err| format!("cannot read cwd: {err}"))?,
    };
    if !folder.is_dir() {
        return Err(format!("folder does not exist: {}", folder.display()));
    }
    std::fs::canonicalize(&folder)
        .map_err(|err| format!("cannot resolve folder {}: {err}", folder.display()))
}

/// Prefixes the title so parked handoffs stand out in the session list, without
/// doubling a prefix the caller already wrote.
pub(crate) fn handoff_title(title: &str) -> String {
    let title = title.trim();
    if title.starts_with("HAND-OVER:") {
        title.to_string()
    } else {
        format!("HAND-OVER: {title}")
    }
}
