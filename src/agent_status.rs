//! Live Claude Code session status, read from `claude agents --json`.
//!
//! Two kinds of live holder are reported:
//!
//! - **Background** sessions run inside Claude Code's daemon (`←` on an empty
//!   prompt, `/bg`, `claude --bg`, or the agent view). While that worker is
//!   alive, `claude --resume <id>` with a session-config flag such as
//!   `--dangerously-skip-permissions` is refused with exit 1.
//! - **Interactive** sessions open in another terminal. Claude Code does not
//!   block resuming those, so a second process would append to the same
//!   transcript.
//!
//! The status only marks such sessions in the list; it does not change how they
//! are opened. `claude agents --json` reads the on-disk registry (it does not
//! start the daemon), lists one config dir, and already omits an interactive
//! process whose session was moved to the background. An entry carries `pid`
//! only while its process is alive; retired, stopped, and daemon-less background
//! entries have none and resume normally, so only entries with a `pid` are
//! reported. See `docs/background-sessions.md`.

use crate::model::Agent;
use crate::profile::Profile;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Upper bound for one `claude agents --json` call (normally ~0.3 s).
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Live holder of a Claude session. The background variants mirror the agent
/// view groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveStatus {
    /// Background `state: working` — a turn is running.
    Working,
    /// Background `state: blocked` — waiting on a permission prompt or input.
    NeedsInput,
    /// Background `state: done` — the turn finished but the worker is still
    /// alive, so a flagged resume is still refused until the daemon retires it.
    Done,
    /// Interactive — open in another Claude Code terminal.
    Open,
}

impl LiveStatus {
    /// Legend text shown beside the marker in the Prompt metadata.
    pub fn description(self) -> &'static str {
        match self {
            LiveStatus::Working => "Claude background session · working",
            LiveStatus::NeedsInput => "Claude background session · needs input",
            LiveStatus::Done => "Claude background session · done, still held (resume fails)",
            LiveStatus::Open => "Open in another Claude Code terminal",
        }
    }

    /// Title marker. Every background state shares `Ⓑ` because they open the
    /// same way; the state itself is told by [`Self::description`].
    pub fn glyph(self) -> char {
        match self {
            LiveStatus::Working | LiveStatus::NeedsInput | LiveStatus::Done => 'Ⓑ',
            LiveStatus::Open => 'Ⓞ',
        }
    }
}

/// Session id → status for the live sessions of one config dir.
pub type StatusMap = HashMap<String, LiveStatus>;

/// Parses `claude agents --json`. Unknown or missing fields are skipped per
/// entry; a document that is not a JSON array yields `None`. When one session
/// has both kinds of holder, the background status wins: it is the one that
/// makes a resume fail.
pub fn parse(json: &str) -> Option<StatusMap> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let mut map = StatusMap::new();
    for entry in &entries {
        // Without a live process the session resumes like any other.
        if entry.get("pid").and_then(|v| v.as_u64()).is_none() {
            continue;
        }
        let Some(id) = entry.get("sessionId").and_then(|v| v.as_str()) else {
            continue;
        };
        let status = match entry.get("kind").and_then(|v| v.as_str()) {
            Some("background") => match entry.get("state").and_then(|v| v.as_str()) {
                Some("blocked") => LiveStatus::NeedsInput,
                Some("done") => LiveStatus::Done,
                _ => LiveStatus::Working,
            },
            Some("interactive") => LiveStatus::Open,
            _ => continue,
        };
        let id = id.to_ascii_lowercase();
        if status == LiveStatus::Open && map.contains_key(&id) {
            continue;
        }
        map.insert(id, status);
    }
    Some(map)
}

/// Runs `claude agents --json` against `profile`'s config dir. `None` on any
/// failure (missing CLI, timeout, non-zero exit, unparsable output); callers
/// then show no marker for that profile.
pub fn query(profile: &Profile) -> Option<StatusMap> {
    let bin = std::env::var_os("S7S_AGENTS_CLAUDE_BIN").unwrap_or_else(|| "claude".into());
    let mut cmd = Command::new(bin);
    cmd.args(["agents", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Same env rules as resume: a nested call inherits Claude Code's session
    // variables when s7s itself runs inside Claude Code.
    crate::resume::sanitize_agent_env(&mut cmd);
    if let Some((key, value)) = profile.env_var() {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Drain stdout on a thread so a large listing cannot block the child on a
    // full pipe while this thread waits for it to exit.
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut buf).ok()?;
        Some(buf)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < QUERY_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()??;
    if !status.success() {
        return None;
    }
    parse(&out)
}

/// Queries every Claude profile on a worker thread. Each profile yields one
/// `(profile_id, result)` message; the channel disconnects when all are done.
pub fn spawn_fetch(profiles: Vec<Profile>) -> Receiver<(String, Option<StatusMap>)> {
    let (tx, rx) = mpsc::channel();
    let targets: Vec<Profile> = profiles
        .into_iter()
        .filter(|p| p.agent == Agent::Claude && p.path.is_dir())
        .collect();
    let _ = std::thread::Builder::new()
        .name("s7s-agent-status".into())
        .spawn(move || {
            for profile in targets {
                let result = query(&profile);
                if tx.send((profile.id.clone(), result)).is_err() {
                    break;
                }
            }
        });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape observed from Claude Code 2.1.288 (`claude agents --json`).
    const SAMPLE: &str = r#"[
      {"id":"8403d2ca","cwd":"/w/a","kind":"background","startedAt":1,
       "sessionId":"8403d2ca-4333-4edd-a0e7-8c045e064663","name":"retired","state":"blocked"},
      {"cwd":"/w/probe","kind":"interactive","startedAt":2,
       "sessionId":"c3cbc2a0-0bd0-44b3-9fba-a782c578ec9d","name":"probe-cf","status":"idle","pid":39118},
      {"id":"8d1d04b2","cwd":"/w/b","kind":"background","startedAt":3,
       "sessionId":"8D1D04B2-B528-4BE2-8401-DE08B92C8C8D","name":"done","status":"idle","state":"done","pid":95897},
      {"id":"09066cde","cwd":"/w/b","kind":"background","startedAt":4,
       "sessionId":"09066cde-981a-4bd3-9ea1-fdbc014c3cc8","name":"blocked","status":"idle","state":"blocked","pid":95904},
      {"id":"f4c965a4","cwd":"/w/c","kind":"background","startedAt":5,
       "sessionId":"f4c965a4-b28a-4c37-8ccd-a5bffaf23cef","name":"working","status":"busy","state":"working","pid":99137}
    ]"#;

    #[test]
    fn keeps_only_entries_with_a_live_process() {
        let map = parse(SAMPLE).unwrap();
        assert_eq!(map.len(), 4);
        // A background job whose worker is gone resumes normally.
        assert!(!map.contains_key("8403d2ca-4333-4edd-a0e7-8c045e064663"));
        assert_eq!(
            map["c3cbc2a0-0bd0-44b3-9fba-a782c578ec9d"],
            LiveStatus::Open
        );
        let unknown_kind = parse(r#"[{"kind":"something-new","sessionId":"x","pid":1}]"#).unwrap();
        assert!(unknown_kind.is_empty());
    }

    #[test]
    fn background_status_wins_over_an_interactive_holder() {
        for json in [
            r#"[{"kind":"interactive","sessionId":"x","pid":1},
                {"kind":"background","sessionId":"x","pid":2,"state":"done"}]"#,
            r#"[{"kind":"background","sessionId":"x","pid":2,"state":"done"},
                {"kind":"interactive","sessionId":"x","pid":1}]"#,
        ] {
            assert_eq!(parse(json).unwrap()["x"], LiveStatus::Done);
        }
    }

    #[test]
    fn maps_agent_view_states_and_lowercases_ids() {
        let map = parse(SAMPLE).unwrap();
        assert_eq!(
            map["8d1d04b2-b528-4be2-8401-de08b92c8c8d"],
            LiveStatus::Done
        );
        assert_eq!(
            map["09066cde-981a-4bd3-9ea1-fdbc014c3cc8"],
            LiveStatus::NeedsInput
        );
        assert_eq!(
            map["f4c965a4-b28a-4c37-8ccd-a5bffaf23cef"],
            LiveStatus::Working
        );
    }

    #[test]
    fn rejects_non_array_output_and_tolerates_empty_lists() {
        assert!(parse("not json").is_none());
        assert!(parse(r#"{"sessions":[]}"#).is_none());
        assert!(parse("[]").unwrap().is_empty());
    }

    #[test]
    fn unknown_state_on_a_live_worker_counts_as_working() {
        let map =
            parse(r#"[{"kind":"background","sessionId":"x","pid":1,"state":"something-new"}]"#)
                .unwrap();
        assert_eq!(map["x"], LiveStatus::Working);
    }
}
