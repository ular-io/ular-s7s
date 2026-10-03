//! Live session status for Claude Code (`claude agents --json`) and
//! Antigravity (`presence/<conversation id>.lock`). Codex is not covered: its
//! shared app-server daemon keeps threads loaded and locked after the terminal
//! that opened them exits, and nothing on disk or in its protocol says which
//! terminal shows which thread.
//!
//! Two kinds of live Claude holder are reported:
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
//! reported.
//!
//! A running agy holds an exclusive `flock` on its conversation's presence
//! file and releases it on exit; the file itself stays. Held files are
//! reported as open in another terminal. See `docs/background-sessions.md`.

use crate::model::Agent;
use crate::profile::Profile;
use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Upper bound for one `claude agents --json` call (normally ~0.3 s).
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Live holder of a session. The background variants are Claude-only and
/// mirror its agent view groups; `Open` covers Claude and agy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveStatus {
    /// Background `state: working` — a turn is running.
    Working,
    /// Background `state: blocked` — waiting on a permission prompt or input.
    NeedsInput,
    /// Background `state: done` — the turn finished but the worker is still
    /// alive, so a flagged resume is still refused until the daemon retires it.
    Done,
    /// Open in another terminal (Claude Code interactive session, or a
    /// running agy holding the conversation's presence lock).
    Open,
}

impl LiveStatus {
    /// Legend text shown beside the marker in the Prompt metadata.
    pub fn description(self) -> &'static str {
        match self {
            LiveStatus::Working => "Claude background session · working",
            LiveStatus::NeedsInput => "Claude background session · needs input",
            LiveStatus::Done => "Claude background session · done, still held (resume fails)",
            LiveStatus::Open => "Open in another terminal",
        }
    }

    /// Who holds the session, as one sentence for dialogs and CLI errors.
    pub fn holder_sentence(self) -> String {
        match self {
            LiveStatus::Open => "This session is open in another terminal.".to_string(),
            status => format!(
                "Claude Code is running this session in the background ({}).",
                status.state_label()
            ),
        }
    }

    /// Short state word for dialogs.
    pub fn state_label(self) -> &'static str {
        match self {
            LiveStatus::Working => "working",
            LiveStatus::NeedsInput => "needs input",
            LiveStatus::Done => "done",
            LiveStatus::Open => "open",
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

/// One live holder of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveEntry {
    pub status: LiveStatus,
    /// Background job id that `claude attach` takes (the agent view's short
    /// id); `None` for interactive holders.
    pub job: Option<String>,
}

/// Session id → live holder for one config dir.
pub type StatusMap = HashMap<String, LiveEntry>;

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
        let live = match entry.get("kind").and_then(|v| v.as_str()) {
            Some("background") => LiveEntry {
                status: match entry.get("state").and_then(|v| v.as_str()) {
                    Some("blocked") => LiveStatus::NeedsInput,
                    Some("done") => LiveStatus::Done,
                    _ => LiveStatus::Working,
                },
                job: entry.get("id").and_then(|v| v.as_str()).map(str::to_string),
            },
            Some("interactive") => LiveEntry {
                status: LiveStatus::Open,
                job: None,
            },
            _ => continue,
        };
        let id = id.to_ascii_lowercase();
        if live.status == LiveStatus::Open && map.contains_key(&id) {
            continue;
        }
        map.insert(id, live);
    }
    Some(map)
}

/// Live holders for `profile`'s sessions. `None` on any failure; callers then
/// show no marker for that profile. Codex profiles always yield `None`.
pub fn query(profile: &Profile) -> Option<StatusMap> {
    match profile.agent {
        Agent::Claude => query_claude(profile),
        Agent::Antigravity => query_agy(&profile.path.join("presence")),
        Agent::Codex => None,
    }
}

/// Reports every agy conversation whose presence lock another process holds.
/// A missing presence folder means no conversation has run there yet.
fn query_agy(presence: &Path) -> Option<StatusMap> {
    let entries = match std::fs::read_dir(presence) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Some(StatusMap::new()),
        Err(_) => return None,
    };
    let mut map = StatusMap::new();
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().and_then(|e| e.to_str()) != Some("lock") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if lock_held(&path) == Some(true) {
            map.insert(
                id.to_ascii_lowercase(),
                LiveEntry {
                    status: LiveStatus::Open,
                    job: None,
                },
            );
        }
    }
    Some(map)
}

/// Whether another process holds a `flock` on `path`. `None` when it cannot
/// be checked.
#[cfg(target_os = "macos")]
fn lock_held(path: &Path) -> Option<bool> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    // macOS reports flock locks through F_GETLK, which acquires nothing, so the
    // check can never collide with agy taking the lock at the same moment.
    let mut probe: libc::flock = unsafe { std::mem::zeroed() };
    probe.l_type = libc::F_WRLCK as _;
    probe.l_whence = libc::SEEK_SET as _;
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut probe) } == -1 {
        return None;
    }
    Some(i32::from(probe.l_type) != libc::F_UNLCK as i32)
}

/// Whether another process holds a `flock` on `path`. `None` when it cannot
/// be checked.
#[cfg(not(target_os = "macos"))]
fn lock_held(path: &Path) -> Option<bool> {
    use std::os::fd::AsRawFd;
    let file = std::fs::File::open(path).ok()?;
    // Linux keeps flock and fcntl locks apart, so probe with a non-blocking
    // shared flock and release it at once.
    let fd = file.as_raw_fd();
    if unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) } == 0 {
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        return Some(false);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EWOULDBLOCK) => Some(true),
        _ => None,
    }
}

/// Runs `claude agents --json` against `profile`'s config dir. `None` on any
/// failure (missing CLI, timeout, non-zero exit, unparsable output).
fn query_claude(profile: &Profile) -> Option<StatusMap> {
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

/// Live holder of `session` right now, for the `s7s session` mutations. `None`
/// for Codex sessions and when the query fails.
pub fn live_holder(profile: &Profile, session: &crate::model::Session) -> Option<LiveEntry> {
    query(profile)?.remove(&session.id.to_ascii_lowercase())
}

impl LiveEntry {
    /// How to release the session before deleting or renaming it.
    pub fn release_hint(&self) -> String {
        match self.status {
            LiveStatus::Open => "Exit it in that terminal first, then try again.".to_string(),
            _ => format!(
                "Stop it with `claude stop {}`, or wait until Claude Code retires it (1 hour idle).",
                self.job.as_deref().unwrap_or("<id>")
            ),
        }
    }
}

/// Explains a live holder on stderr for `s7s session` mutations (`what` is the
/// refused verb).
pub fn print_live_refusal(entry: &LiveEntry, what: &str) {
    eprintln!("error: cannot {what}: {}", entry.status.holder_sentence());
    eprintln!("hint: {}", entry.release_hint());
}

/// Queries every Claude and agy profile on a worker thread. Each profile
/// yields one `(profile_id, result)` message; the channel disconnects when all
/// are done.
pub fn spawn_fetch(profiles: Vec<Profile>) -> Receiver<(String, Option<StatusMap>)> {
    let (tx, rx) = mpsc::channel();
    let targets: Vec<Profile> = profiles
        .into_iter()
        .filter(|p| p.agent != Agent::Codex && p.path.is_dir())
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
            map["c3cbc2a0-0bd0-44b3-9fba-a782c578ec9d"].status,
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
            assert_eq!(parse(json).unwrap()["x"].status, LiveStatus::Done);
        }
    }

    #[test]
    fn maps_agent_view_states_and_lowercases_ids() {
        let map = parse(SAMPLE).unwrap();
        assert_eq!(
            map["8d1d04b2-b528-4be2-8401-de08b92c8c8d"].status,
            LiveStatus::Done
        );
        assert_eq!(
            map["09066cde-981a-4bd3-9ea1-fdbc014c3cc8"].status,
            LiveStatus::NeedsInput
        );
        assert_eq!(
            map["f4c965a4-b28a-4c37-8ccd-a5bffaf23cef"].status,
            LiveStatus::Working
        );
    }

    #[test]
    fn background_entries_carry_their_attach_id() {
        let map = parse(SAMPLE).unwrap();
        assert_eq!(
            map["8d1d04b2-b528-4be2-8401-de08b92c8c8d"].job.as_deref(),
            Some("8d1d04b2")
        );
        assert_eq!(map["c3cbc2a0-0bd0-44b3-9fba-a782c578ec9d"].job, None);
    }

    #[test]
    fn agy_presence_reports_only_conversations_another_process_locks() {
        use std::io::BufRead;
        let root = crate::ui::test_support::TempBookmarkStore::new();
        let presence = root.path.with_file_name("presence");
        assert!(query_agy(&presence).unwrap().is_empty(), "no folder yet");

        std::fs::create_dir_all(&presence).unwrap();
        let open = presence.join("AAAA-open.lock");
        std::fs::write(&open, "").unwrap();
        std::fs::write(presence.join("bbbb-closed.lock"), "").unwrap();
        std::fs::write(presence.join("notes.txt"), "").unwrap();
        assert!(query_agy(&presence).unwrap().is_empty(), "nothing locked");

        // Another process takes the exclusive flock agy holds while open.
        let Ok(mut child) = std::process::Command::new("perl")
            .args([
                "-e",
                "use Fcntl ':flock'; open(my $f, '<', $ARGV[0]) or die; \
                 flock($f, LOCK_EX) or die; $| = 1; print \"L\\n\"; sleep 30",
            ])
            .arg(&open)
            .stdout(std::process::Stdio::piped())
            .spawn()
        else {
            return; // perl is unavailable; the unlocked cases above still ran.
        };
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let map = query_agy(&presence).unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(line.trim(), "L");
        assert_eq!(map.len(), 1, "{map:?}");
        assert_eq!(map["aaaa-open"].status, LiveStatus::Open);
        assert_eq!(map["aaaa-open"].job, None);
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
        assert_eq!(map["x"].status, LiveStatus::Working);
    }
}
