//! Querying of agent CLI usage (remaining % and reset countdown).
//!
//! Runs each CLI inside an invisible PTY, inputs usage commands (`/usage` for claude/agy,
//! `/status` for codex), and parses the remaining percentage (%) from the vt100-reconstructed screen.
//! Reads only the official client interface without token extraction or unofficial API calls.
//!
//! Note that the meaning of % varies across tools (based on verified CLI screens):
//! - claude: `N% used`  -> remaining = 100 - N
//! - codex:  `N% left`  -> remaining directly
//! - agy:    `N%` on the gauge line -> remaining directly (verified as 0.00% when weekly limit is exhausted)
//!
//! Reset notations also differ:
//! - claude: Absolute time - `Resets 5am (Asia/Seoul)`, `Resets Jul 10 at 5pm (Asia/Seoul)`
//! - codex:  Absolute time - `(resets 04:45)`, `(resets Mon 14:30)`, `(resets Mon Jul 10)`, `(resets 17:33 on 15 Jul)`
//!   (verified by presence of chrono formats `%H:%M`/`%a %H:%M`/`%a %b %d` in binary)
//! - agy:    Relative time - `Refreshes in 16h 51m`
//!
//! Absolute times are parsed based on the next occurrence in local time and converted into countdowns.
//!
//! The header displays two windows together:
//! - current: 5-hour or current session type limit
//! - weekly:  weekly type limit

use crate::model::Agent;
use crate::profile::Profile;
use anyhow::{anyhow, Result};
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, NaiveDateTime, NaiveTime};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Phase of usage query for a single profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsagePhase {
    /// No query run yet.
    Idle,
    /// Query in progress.
    Loading,
    /// Query succeeded (last contains the latest snapshot).
    Ready,
    /// Query failed.
    Failed,
    /// Logged out (cannot query usage, requires login).
    NotLoggedIn,
    /// CLI not installed (executable not found in PATH).
    NotInstalled,
    /// Config folder missing (deleted, renamed, etc.).
    MissingDir,
    /// No query mechanism available (extra Antigravity profiles without env injection).
    Unavailable,
}

/// Result of usage query for a single profile (background thread -> UI channel payload).
#[derive(Debug, Clone, PartialEq)]
pub enum UsageResult {
    /// Query succeeded.
    Ready(UsageSnapshot),
    /// Judged as logged out.
    NotLoggedIn,
    /// Judged as CLI not installed.
    NotInstalled,
    /// Judged as config folder missing.
    MissingDir,
    /// Judged as no query mechanism available (extra Antigravity profiles).
    Unavailable,
    /// Query failed (reason string).
    Failed(String),
}

/// State of usage query for a single profile.
///
/// `last` preserves the last successful snapshot during Loading/Failed phases,
/// allowing the app to keep displaying previous values while updating.
///
/// The state changes only as a result of explicit updates (app startup, Ctrl+U, profile save).
/// During updates, it always displays as Loading regardless of the prior state
/// so users consistently perceive that a query check is taking place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageEntry {
    pub phase: UsagePhase,
    pub last: Option<UsageSnapshot>,
}

impl UsageEntry {
    pub fn idle() -> Self {
        UsageEntry {
            phase: UsagePhase::Idle,
            last: None,
        }
    }
}

/// Remaining usage in an individual window (e.g., 5h, weekly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageWindow {
    pub pct_left: u8,
    pub reset: Option<ResetCountdown>,
}

/// Two usage windows displayed in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UsageSnapshot {
    pub current: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
}

/// Reset countdown normalized into simple day/hour/minute units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetCountdown {
    pub days: u16,
    pub hours: u8,
    pub minutes: u8,
}

/// Usage states per profile (key = profile ID).
#[derive(Default)]
pub struct UsageState {
    entries: HashMap<String, UsageEntry>,
}

impl UsageState {
    pub fn new() -> Self {
        UsageState {
            entries: HashMap::new(),
        }
    }

    /// Returns a copy for viewing (defaults to Idle if missing).
    pub fn entry(&self, profile_id: &str) -> UsageEntry {
        self.entries
            .get(profile_id)
            .copied()
            .unwrap_or_else(UsageEntry::idle)
    }

    pub fn entry_mut(&mut self, profile_id: &str) -> &mut UsageEntry {
        self.entries
            .entry(profile_id.to_string())
            .or_insert_with(UsageEntry::idle)
    }

    /// Clears state when a profile is deleted.
    pub fn remove(&mut self, profile_id: &str) {
        self.entries.remove(profile_id);
    }
}

/// Static demo-mode snapshot per agent (all profiles read as logged-in with realistic numbers).
fn demo_usage(agent: Agent) -> UsageResult {
    let window = |pct_left: u8, days: u16, hours: u8, minutes: u8| UsageWindow {
        pct_left,
        reset: Some(ResetCountdown {
            days,
            hours,
            minutes,
        }),
    };
    let (current, weekly) = match agent {
        Agent::Claude => (window(72, 0, 2, 40), window(48, 3, 6, 0)),
        Agent::Codex => (window(64, 0, 4, 15), window(81, 2, 12, 30)),
        Agent::Antigravity => (window(89, 0, 1, 5), window(57, 5, 3, 20)),
    };
    UsageResult::Ready(UsageSnapshot {
        current: Some(current),
        weekly: Some(weekly),
    })
}

/// Minimum duration to show the Loading state so that users can perceive that a check has occurred.
const MIN_LOADING: Duration = Duration::from_millis(500);

/// Starts parallel queries for profiles and returns a receiver channel for (profile_id, UsageResult).
/// Each query holds for at least `MIN_LOADING` before sending the result, regardless of outcome.
pub fn spawn_fetch(profiles: Vec<Profile>) -> Receiver<(String, UsageResult)> {
    let (tx, rx) = mpsc::channel();
    for profile in profiles {
        let tx: Sender<(String, UsageResult)> = tx.clone();
        thread::spawn(move || {
            let started = Instant::now();
            let res = fetch(&profile);
            if let Some(rem) = MIN_LOADING.checked_sub(started.elapsed()) {
                thread::sleep(rem);
            }
            let _ = tx.send((profile.id, res));
        });
    }
    rx
}

/// Debug option (`--usage-probe`): Queries all profiles in parallel and prints the results.
pub fn probe() {
    let store = crate::profile::ProfileStore::load();
    let targets: Vec<Profile> = store.profiles.to_vec();
    let names: HashMap<String, String> = targets
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let count = targets.len();
    let rx = spawn_fetch(targets);
    for _ in 0..count {
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok((id, res)) => println!("{}: {:?}", names.get(&id).unwrap_or(&id), res),
            Err(e) => {
                println!("recv error: {e}");
                break;
            }
        }
    }
    // Allow time for the cleanup thread (child CLI termination) to finish before exiting the process.
    thread::sleep(Duration::from_secs(5));
}

/// Synchronously queries current/weekly usage for a single agent.
///
/// First checks for executable presence in PATH to determine installation status.
/// Then performs low-cost login checks before spawning a PTY (distinct method per tool):
/// - claude: JSON `loggedIn` key from `claude auth status` (takes ~0.3s)
/// - codex:  Exit code of `codex login status` (0 for login / 1 for logout, takes ~0.2s)
/// - agy:    Presence of macOS keychain entry (`svce=gemini`, `acct=antigravity`).
///   Verified that this entry is deleted upon logout, and if present, auto-relogin occurs on boot.
///   Since launching agy while logged out pops open a browser window, this check must run beforehand.
///
/// `ready_markers` contains screen strings indicating readiness for input; `min_wait` is the minimum wait
/// (boot stabilization) required after the marker appears before sending inputs. Claude/Codex show footer/status
/// instantly (0.4~0.5s), requiring stabilization via `min_wait`, whereas agy shows `? for shortcuts` only when
/// ready (~3.1s), meaning the marker itself serves as a gate even if `min_wait` is zero.
fn fetch(profile: &Profile) -> UsageResult {
    // Demo mode: return static plausible snapshots without launching real CLIs.
    // A real fetch would inject the sandbox path via CLAUDE_CONFIG_DIR/CODEX_HOME,
    // show logged-out states, and could let the CLI write state files into the sandbox.
    if crate::config::is_demo_mode() {
        return demo_usage(profile.agent);
    }
    // Verifying folder existence and query availability is part of the query process - determined
    // here only during explicit updates (avoiding auto-detection during rendering) and stored in the phase.
    if !profile.path.is_dir() {
        return UsageResult::MissingDir;
    }
    // Antigravity cannot have env variables injected, meaning only the default path profile is queried
    // (querying extra profiles would yield the default account values, causing confusion).
    if profile.agent == Agent::Antigravity && !profile.is_default_root() {
        return UsageResult::Unavailable;
    }
    // Inject the profile's config root via agent-specific env variables to query the correct account/subscription
    // (empty list for Antigravity due to lack of known variables).
    let envs: Vec<(&str, &Path)> = profile.env_var().into_iter().collect();
    let bin = match profile.agent {
        Agent::Claude => "claude",
        Agent::Antigravity => "agy",
        Agent::Codex => "codex",
    };
    if !installed(bin) {
        return UsageResult::NotInstalled;
    }
    match profile.agent {
        Agent::Claude => {
            // Fall back to PTY query on verification failure (e.g. older CLI versions).
            if claude_logged_in(&envs) == Some(false) {
                return UsageResult::NotLoggedIn;
            }
            drive(
                "claude",
                "/usage",
                CLAUDE_READY_MARKERS,
                &["% used"],
                &[],
                Duration::from_secs(2),
                Duration::from_millis(800),
                &envs,
                parse_claude,
            )
        }
        Agent::Antigravity => {
            if agy_logged_in() == Some(false) {
                return UsageResult::NotLoggedIn;
            }
            // Logout marker fallback: handles cases where keychain check returns None or the token
            // is invalid, causing automatic relogin to fail (confirmed via screen text).
            drive(
                "agy",
                "/usage",
                &["? for shortcuts"],
                &["Refreshes in", "Weekly Limit"],
                &["You are currently not signed in"],
                Duration::from_millis(0),
                Duration::from_millis(800),
                &envs,
                parse_agy,
            )
        }
        Agent::Codex => {
            if codex_logged_in(&envs) == Some(false) {
                return UsageResult::NotLoggedIn;
            }
            drive(
                "codex",
                "/status",
                &["OpenAI Codex"],
                &["% left (resets"],
                &[],
                Duration::from_millis(2500),
                Duration::from_millis(1500),
                &envs,
                parse_codex,
            )
        }
    }
}

/// Marker for Claude boot completion. Prefers stable, always-visible markers (mode toggles/footer)
/// over ephemeral welcome messages. Shared by `/usage` and `/model` queries.
pub(crate) const CLAUDE_READY_MARKERS: &[&str] = &[
    "shift+tab to cycle",
    "auto mode",
    "for shortcuts",
    "Tips for getting started",
];

/// Scans the PATH for executable binaries to check if the CLI is installed.
pub(crate) fn installed(cmd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        std::fs::metadata(dir.join(cmd))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

/// JSON `loggedIn` value from `claude auth status`. Returns None on execution failure or parsing error.
pub(crate) fn claude_logged_in(envs: &[(&str, &Path)]) -> Option<bool> {
    let mut cmd = std::process::Command::new("claude");
    cmd.args(["auth", "status"]);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let out = cmd.output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("loggedIn")?.as_bool()
}

/// agy login check: presence of macOS keychain entry (`svce=gemini`, `acct=antigravity`).
/// agy deletes this generic password entry upon logout; if present, it automatically re-logs in during execution.
/// Failures other than "not found" (e.g. locked keychain) return None (falling back to PTY query).
fn agy_logged_in() -> Option<bool> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", "gemini", "-a", "antigravity"])
        .output()
        .ok()?;
    if out.status.success() {
        return Some(true);
    }
    String::from_utf8_lossy(&out.stderr)
        .contains("could not be found")
        .then_some(false)
}

/// `codex login status`: returns exit code 0 if logged in, 1 if logged out. Returns None on execution failure.
fn codex_logged_in(envs: &[(&str, &Path)]) -> Option<bool> {
    let mut cmd = std::process::Command::new("codex");
    cmd.args(["login", "status"]);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let out = cmd.output().ok()?;
    Some(out.status.success())
}

/// Spawns the CLI inside a PTY, captures the usage screen text, and extracts remaining % using the parser.
///
/// Spawning PTY, typing commands, and waiting for markers are handled by `drive_screen`.
/// This function simply applies the usage-specific parser to the captured screen text.
#[allow(clippy::too_many_arguments)]
fn drive(
    cmd: &str,
    slash_cmd: &str,
    ready_markers: &[&str],
    done_markers: &[&str],
    logout_markers: &[&str],
    min_wait: Duration,
    enter_delay: Duration,
    envs: &[(&str, &Path)],
    parse: fn(&str) -> Option<UsageSnapshot>,
) -> UsageResult {
    match drive_screen(
        cmd,
        slash_cmd,
        ready_markers,
        done_markers,
        logout_markers,
        min_wait,
        enter_delay,
        envs,
    ) {
        Ok(DriveOutcome::Screen(text)) => parse(&text)
            .map(UsageResult::Ready)
            .unwrap_or_else(|| UsageResult::Failed(format!("{cmd}: failed to parse usage"))),
        Ok(DriveOutcome::NotLoggedIn) => UsageResult::NotLoggedIn,
        Err(e) => UsageResult::Failed(e.to_string()),
    }
}

/// Result of `drive_screen`: final screen text showing completion markers, or a logged out status.
pub(crate) enum DriveOutcome {
    Screen(String),
    NotLoggedIn,
}

/// Drives the CLI inside a PTY to execute a slash command and capture the final screen (generic).
/// Shared by usage queries (`/usage`, `/status`) and model list queries (`/model`).
///
/// Flow: wait for ready markers -> type the command character-by-character -> wait `enter_delay`
/// (stabilizes autocomplete popups; prevents wrong item selection on immediate Enter) ->
/// press Enter -> wait for completion markers -> return screen text. Child process is forcefully terminated on exit.
///
/// If `logout_markers` appear on screen, they are recorded as logout candidates but not finalized immediately
/// (e.g. if agy has a keychain token, a "not signed in" message appears briefly on boot before auto-relogin succeeds;
/// logout is confirmed only if grace time expires without ready markers).
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_screen(
    cmd: &str,
    slash_cmd: &str,
    ready_markers: &[&str],
    done_markers: &[&str],
    logout_markers: &[&str],
    min_wait: Duration,
    enter_delay: Duration,
    envs: &[(&str, &Path)],
) -> Result<DriveOutcome> {
    // Set a wide width so that agy's 5h `Disabled` message (~150 chars) fits on a single line
    // (at 120 width, the refresh time details get truncated).
    const COLS: u16 = 200;
    // codex `/status` output scrolls up, so allocate ample rows to prevent truncation.
    const ROWS: u16 = 60;
    let ready_timeout = Duration::from_secs(40);
    let done_timeout = Duration::from_secs(40);

    // agy and similar tools double-fork, leaving behind background servers that exit the PPID chain immediately.
    // We snapshot process PIDs of the same name before execution, and clean up newly created PPID=1 processes afterwards.
    let preexisting = pids_of(cmd);

    let pty = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| anyhow!("openpty failed: {e}"))?;

    let mut builder = CommandBuilder::new(cmd);
    builder.env("TERM", "xterm-256color");
    for (key, value) in envs {
        builder.env(key, value.as_os_str());
    }
    if let Ok(cwd) = std::env::current_dir() {
        builder.cwd(cwd);
    }
    let child = pty
        .slave
        .spawn_command(builder)
        .map_err(|e| anyhow!("failed to spawn {cmd}: {e}"))?;
    drop(pty.slave);
    // Keep master alive during cleanup for writer operations to succeed, so we separate ownership.
    let master = pty.master;

    // Read PTY output in the background and pass to channel (bypassing blocking read).
    let mut reader = master
        .try_clone_reader()
        .map_err(|e| anyhow!("PTY reader failed: {e}"))?;
    let (btx, brx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || btx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut writer = master
        .take_writer()
        .map_err(|e| anyhow!("PTY writer failed: {e}"))?;

    let mut parser = vt100::Parser::new(ROWS, COLS, 0);
    let start = Instant::now();
    let mut typed_at: Option<Instant> = None;
    let mut entered_at: Option<Instant> = None;
    // Tracks whether a logout marker was seen. Do not decide immediately as it may appear briefly
    // during automatic relogin; finalize as logged out only if grace time expires without ready markers.
    let mut logout_seen = false;
    let logout_decide = Duration::from_secs(15);

    let result = loop {
        // Receives screen updates (200ms polling).
        if let Ok(chunk) = brx.recv_timeout(Duration::from_millis(200)) {
            parser.process(&chunk);
            while let Ok(chunk) = brx.try_recv() {
                parser.process(&chunk);
            }
        }
        let screen = parser.screen().contents();
        if !logout_seen && logout_markers.iter().any(|m| screen.contains(m)) {
            logout_seen = true;
        }

        if typed_at.is_none() {
            // Fail immediately if Claude prompts for folder trust in an untrusted directory.
            if screen.contains("Do you trust the files in this folder")
                || screen.contains("Is this a project you created or one you trust")
            {
                break Err(anyhow!("{cmd}: untrusted folder (trust prompt)"));
            }
            let ready = ready_markers.iter().any(|m| screen.contains(m));
            // Write input after ready markers appear and `min_wait` (boot stabilization) elapses.
            // Claude/Codex (where markers show early) are gated by `min_wait`, whereas agy is gated by the marker itself.
            // Maintains a 20-second fallback timeout for exceptional cases where markers are missed (unless logout candidate).
            if (ready && start.elapsed() >= min_wait)
                || (!logout_seen && start.elapsed() > Duration::from_secs(20))
            {
                for ch in slash_cmd.chars() {
                    let mut b = [0u8; 4];
                    let _ = writer.write_all(ch.encode_utf8(&mut b).as_bytes());
                    let _ = writer.flush();
                    thread::sleep(Duration::from_millis(40));
                }
                typed_at = Some(Instant::now());
            } else if logout_seen && start.elapsed() >= logout_decide {
                break Ok(DriveOutcome::NotLoggedIn);
            } else if start.elapsed() > ready_timeout {
                break Err(anyhow!("{cmd}: timed out waiting for input readiness"));
            }
            continue;
        }

        if entered_at.is_none() {
            if typed_at.unwrap().elapsed() >= enter_delay {
                let _ = writer.write_all(b"\r");
                let _ = writer.flush();
                entered_at = Some(Instant::now());
            }
            continue;
        }

        if done_markers.iter().any(|m| screen.contains(m)) {
            // Allow a brief additional read duration for panel rendering to finish before finalizing.
            thread::sleep(Duration::from_millis(500));
            while let Ok(chunk) = brx.try_recv() {
                parser.process(&chunk);
            }
            break Ok(DriveOutcome::Screen(parser.screen().contents()));
        }
        if entered_at.unwrap().elapsed() > done_timeout {
            // If the logout screen was visible, treat as logged out instead of timing out.
            if logout_seen {
                break Ok(DriveOutcome::NotLoggedIn);
            }
            break Err(anyhow!("{cmd}: timed out waiting for {slash_cmd} screen"));
        }
    };

    // Debug: If ULAR_USAGE_DUMP=<dir> is set, dump the final screen contents to a file.
    // Includes the command name in the filename since the same CLI is queried with different slash commands.
    if let Some(dir) = std::env::var_os("ULAR_USAGE_DUMP") {
        let name = format!("{cmd}-{}.screen.txt", slash_cmd.trim_start_matches('/'));
        let path = std::path::Path::new(&dir).join(name);
        let _ = std::fs::write(path, parser.screen().contents());
    }

    // Move cleanup to a background thread. Returns parsed values immediately and handles process termination
    // and orphan recovery in a separate thread to eliminate UI delays (Claude was particularly slow during exit).
    // Moves master/writer/child to the thread to keep the PTY alive until cleanup completes.
    let cmd_owned = cmd.to_string();
    thread::spawn(move || {
        // master is kept alive until cleanup completes to ensure writer calls remain valid.
        let _keep_master = master;
        let mut writer = writer;
        let mut child = child;

        // Tools like agy leave a double-forked background server behind, meaning killing the parent
        // results in orphan processes. First send normal exit sequence (Ctrl+C x2 -> Ctrl+D)
        // to let the CLI clean up its children, falling back to forceful termination if it fails to exit.
        let _ = writer.write_all(b"\x03");
        let _ = writer.flush();
        thread::sleep(Duration::from_millis(200));
        let _ = writer.write_all(b"\x03\x04");
        let _ = writer.flush();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut exited = false;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                exited = true;
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if !exited {
            if let Some(pid) = child.process_id() {
                let descendants = collect_descendants(pid as i32);
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
                for d in descendants {
                    unsafe {
                        libc::kill(d, libc::SIGKILL);
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }

        // Recover newly created orphan daemons (PPID=1) from this execution.
        // Processes of the same name that existed before (e.g. user-initiated sessions) are in the snapshot and left untouched.
        for pid in pids_of(&cmd_owned) {
            if !preexisting.contains(&pid) && parent_pid(pid) == Some(1) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    });

    result
}

/// Gets the list of PIDs with an exact process name match via `pgrep -x`.
fn pids_of(name: &str) -> Vec<i32> {
    let Ok(o) = std::process::Command::new("pgrep")
        .args(["-x", name])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<i32>().ok())
        .collect()
}

/// Queries the parent PID using `ps`.
fn parent_pid(pid: i32) -> Option<i32> {
    let o = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&o.stdout)
        .trim()
        .parse::<i32>()
        .ok()
}

/// Recursively collects child processes of a PID via `pgrep -P` (must be called before the parent process dies).
fn collect_descendants(root: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let mut queue = vec![root];
    while let Some(pid) = queue.pop() {
        let Ok(o) = std::process::Command::new("pgrep")
            .args(["-P", &pid.to_string()])
            .output()
        else {
            continue;
        };
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Ok(child) = line.trim().parse::<i32>() {
                out.push(child);
                queue.push(child);
            }
        }
    }
    out
}

/// claude `/usage`: Reads `N% used` (calculated as 100-N) and `Resets <absolute time>`
/// lines from the `Current session` and `Current week` blocks.
fn parse_claude(text: &str) -> Option<UsageSnapshot> {
    parse_claude_at(text, Local::now().naive_local())
}

fn parse_claude_at(text: &str, now: NaiveDateTime) -> Option<UsageSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let current = claude_window(&lines, "current session", now);
    // For weekly, prioritize "all models" over model-specific entries (precedes them, but handled defensively against order changes).
    let weekly = claude_window(&lines, "current week (all models)", now)
        .or_else(|| claude_window(&lines, "current week", now));
    snapshot_or_none(current, weekly)
}

/// Searches for the `N% used` gauge and `Resets ...` line in the 3 lines following the label line.
fn claude_window(lines: &[&str], label: &str, now: NaiveDateTime) -> Option<UsageWindow> {
    let idx = lines
        .iter()
        .position(|l| l.trim().to_ascii_lowercase().starts_with(label))?;
    let mut pct = None;
    let mut reset = None;
    for line in lines.iter().skip(idx + 1).take(3) {
        let lower = line.to_ascii_lowercase();
        if pct.is_none() {
            pct = percent_before(&lower, "% used")
                .map(|used| (100.0 - used).clamp(0.0, 100.0).round() as u8);
        }
        if reset.is_none() {
            if let Some(p) = lower.find("resets") {
                reset = parse_reset_spec(&lower[p + "resets".len()..], now);
            }
        }
    }
    Some(UsageWindow {
        pct_left: pct?,
        reset,
    })
}

/// codex `/status`: Reads `N% left (resets <absolute time>)` from `5h limit:` and `Weekly limit:` lines.
fn parse_codex(text: &str) -> Option<UsageSnapshot> {
    parse_codex_at(text, Local::now().naive_local())
}

fn parse_codex_at(text: &str, now: NaiveDateTime) -> Option<UsageSnapshot> {
    let current = codex_window(text, "5h limit", now);
    let weekly = codex_window(text, "weekly limit", now);
    snapshot_or_none(current, weekly)
}

fn codex_window(text: &str, label: &str, now: NaiveDateTime) -> Option<UsageWindow> {
    let lower = text
        .lines()
        .map(|l| l.to_ascii_lowercase())
        .find(|l| l.contains(label) && l.contains("% left"))?;
    let pct = percent_before(&lower, "% left")?.clamp(0.0, 100.0).round() as u8;
    let reset = lower.find("(resets").and_then(|p| {
        let rest = &lower[p + "(resets".len()..];
        let end = rest.find(')').unwrap_or(rest.len());
        parse_reset_spec(&rest[..end], now)
    });
    Some(UsageWindow {
        pct_left: pct,
        reset,
    })
}

/// agy `/usage`: Divided into blocks of `Weekly Limit` and `Five Hour Limit` by model group
/// (GEMINI / CLAUDE AND GPT). Selects the group based on the active model shown in the bottom status line,
/// then reads the `N%` gauge (remaining) and `Refreshes in ...`.
fn parse_agy(text: &str) -> Option<UsageSnapshot> {
    parse_agy_at(text, Local::now().naive_local())
}

fn parse_agy_at(text: &str, now: NaiveDateTime) -> Option<UsageSnapshot> {
    let lines: Vec<&str> = text.lines().collect();
    let footer = lines
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .copied()
        .unwrap_or("");
    let group = if footer.contains("Gemini") {
        "GEMINI MODELS"
    } else {
        "CLAUDE AND GPT MODELS"
    };
    let start = lines.iter().position(|l| l.contains(group)).unwrap_or(0);
    // Group section: from the header to just before the next group header (uppercase line ending with "MODELS").
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, l)| l.trim().ends_with("MODELS"))
        .map(|(i, _)| i)
        .unwrap_or(lines.len());
    let section = &lines[start..end];
    let current = agy_window(section, &["five hour limit", "5h limit", "5-hour limit"]);
    let weekly = agy_window(section, &["weekly limit"]);
    let _ = now; // agy only uses relative notation, but signature is kept consistent with other parsers.
    snapshot_or_none(current, weekly)
}

/// Searches for the `N%` gauge (remaining) and `Refreshes in ...` in the 3 lines following the label line.
/// If `Disabled: ... will fully refresh in 16 hours, 37 minutes.` is shown due to weekly limit depletion,
/// records remaining usage as 0% and parses the refresh time as the countdown.
fn agy_window(lines: &[&str], labels: &[&str]) -> Option<UsageWindow> {
    let idx = lines.iter().position(|l| {
        let t = l.trim().to_ascii_lowercase();
        labels.iter().any(|label| t.starts_with(label))
    })?;
    let mut pct = None;
    let mut reset = None;
    for line in lines.iter().skip(idx + 1).take(3) {
        let lower = line.to_ascii_lowercase();
        if lower.contains("disabled") {
            let reset = lower
                .find("refresh in")
                .and_then(|p| parse_reset_relative(&lower[p + "refresh in".len()..]));
            return Some(UsageWindow { pct_left: 0, reset });
        }
        if pct.is_none() {
            pct = first_percent(&lower).map(|left| left.clamp(0.0, 100.0).round() as u8);
        }
        if reset.is_none() {
            if let Some(p) = lower.find("refreshes in") {
                reset = parse_reset_relative(&lower[p + "refreshes in".len()..]);
            }
        }
    }
    Some(UsageWindow {
        pct_left: pct?,
        reset,
    })
}

fn snapshot_or_none(
    current: Option<UsageWindow>,
    weekly: Option<UsageWindow>,
) -> Option<UsageSnapshot> {
    if current.is_none() && weekly.is_none() {
        None
    } else {
        Some(UsageSnapshot { current, weekly })
    }
}

/// Converts reset time expressions appearing after 'resets' or 'refreshes' into countdowns relative to 'now'.
/// Supported formats (assumes lowercase inputs, based on verified screens):
/// - Relative: `in 16h 51m`, `in 2 hours`
/// - Absolute time: `5am`, `5:30pm`, `04:45` -> next occurrence
/// - Absolute day of week (+ time): `mon 14:30` -> next occurrence of that weekday
/// - Absolute date (+ time): `jul 10 at 5pm`, `mon jul 10`, `17:33 on 15 jul` -> corresponding date this/next year
///   Timezone details in parentheses (e.g. `(asia/seoul)`) are ignored and parsed in local time.
fn parse_reset_spec(raw: &str, now: NaiveDateTime) -> Option<ResetCountdown> {
    let cleaned = raw.split('(').next().unwrap_or("");
    let s = cleaned
        .trim()
        .trim_matches(|c: char| matches!(c, ')' | ',' | '.'))
        .to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Some(rel) = s.strip_prefix("in ") {
        return parse_reset_relative(rel);
    }

    let mut weekday: Option<chrono::Weekday> = None;
    let mut month_day: Option<(u32, u32)> = None;
    let mut time: Option<NaiveTime> = None;
    let tokens: Vec<&str> = s
        .split_whitespace()
        .map(|tok| tok.trim_matches(|c: char| matches!(c, '(' | ')' | ',' | '.')))
        .filter(|tok| !tok.is_empty() && *tok != "at" && *tok != "on")
        .collect();
    for (idx, tok) in tokens.iter().enumerate() {
        if weekday.is_none() {
            if let Some(wd) = parse_weekday(tok) {
                weekday = Some(wd);
                continue;
            }
        }
        if month_day.is_none() {
            if let Some(month) = parse_month(tok) {
                let day = tokens.get(idx + 1).and_then(|d| parse_day_token(d));
                if let Some(day) = day {
                    month_day = Some((month, day));
                    continue;
                }
            }
            if let Some(day) = parse_day_token(tok) {
                if let Some(month) = tokens.get(idx + 1).and_then(|m| parse_month(m)) {
                    month_day = Some((month, day));
                    continue;
                }
            }
        }
        if time.is_none() {
            if let Some(t) = parse_time_token(tok) {
                time = Some(t);
                continue;
            }
        }
    }

    let target = if let Some((month, day)) = month_day {
        let t = time.unwrap_or(NaiveTime::MIN);
        let mut dt = NaiveDate::from_ymd_opt(now.year(), month, day)?.and_time(t);
        if dt <= now {
            dt = NaiveDate::from_ymd_opt(now.year() + 1, month, day)?.and_time(t);
        }
        dt
    } else if let Some(wd) = weekday {
        let t = time.unwrap_or(NaiveTime::MIN);
        let ahead = (wd.num_days_from_monday() as i64
            - now.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        let mut dt = (now.date() + ChronoDuration::days(ahead)).and_time(t);
        if dt <= now {
            dt += ChronoDuration::days(7);
        }
        dt
    } else if let Some(t) = time {
        let mut dt = now.date().and_time(t);
        if dt <= now {
            dt += ChronoDuration::days(1);
        }
        dt
    } else {
        // If not an absolute time format, retry as a relative notation without a prefix (e.g. "2h 2m").
        return parse_reset_relative(&s);
    };
    countdown_between(now, target)
}

/// Parses relative notation (e.g., `1d 14h`, `16h 51m`, `2 hours`) into a countdown.
fn parse_reset_relative(raw: &str) -> Option<ResetCountdown> {
    let mut days = 0u16;
    let mut hours = 0u16;
    let mut minutes = 0u16;
    let mut any = false;
    let mut pending: Option<u16> = None;
    for tok in raw.to_ascii_lowercase().split_whitespace() {
        let tok = tok.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let digits: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
        let unit = &tok[digits.len()..];
        let n = if digits.is_empty() {
            pending.take()
        } else {
            digits.parse::<u16>().ok()
        };
        let Some(n) = n else { continue };
        match unit {
            "" => pending = Some(n),
            u if u.starts_with('d') => {
                days = n;
                any = true;
            }
            u if u.starts_with('h') => {
                hours = n;
                any = true;
            }
            u if u.starts_with('m') => {
                minutes = n;
                any = true;
            }
            _ => {}
        }
    }
    if !any || (days == 0 && hours == 0 && minutes == 0) {
        None
    } else {
        let total_minutes = (days as u32 * 24 * 60) + (hours as u32 * 60) + minutes as u32;
        Some(ResetCountdown {
            days: (total_minutes / (24 * 60)) as u16,
            hours: ((total_minutes / 60) % 24) as u8,
            minutes: (total_minutes % 60) as u8,
        })
    }
}

fn countdown_between(now: NaiveDateTime, target: NaiveDateTime) -> Option<ResetCountdown> {
    let mins = (target - now).num_minutes();
    if mins <= 0 {
        return None;
    }
    Some(ResetCountdown {
        days: (mins / (24 * 60)) as u16,
        hours: ((mins / 60) % 24) as u8,
        minutes: (mins % 60) as u8,
    })
}

/// Parses `5am`, `5:30pm`, or `04:45` into a time object. Single numbers (like `10`) are not
/// considered valid times to avoid confusion with calendar days, so a `:` or am/pm suffix must be present.
fn parse_time_token(tok: &str) -> Option<NaiveTime> {
    let (body, meridiem) = if let Some(b) = tok.strip_suffix("am") {
        (b, Some(false))
    } else if let Some(b) = tok.strip_suffix("pm") {
        (b, Some(true))
    } else {
        (tok, None)
    };
    let (h_str, m_str) = match body.split_once(':') {
        Some((h, m)) => (h, m),
        None => (body, "0"),
    };
    if h_str.is_empty()
        || !h_str.chars().all(|c| c.is_ascii_digit())
        || !m_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut hours: u32 = h_str.parse().ok()?;
    let minutes: u32 = m_str.parse().ok()?;
    match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&hours) {
                return None;
            }
            if hours == 12 {
                hours = 0;
            }
            if pm {
                hours += 12;
            }
        }
        None => {
            if !tok.contains(':') {
                return None;
            }
        }
    }
    NaiveTime::from_hms_opt(hours, minutes, 0)
}

fn parse_weekday(tok: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday::*;
    match tok.get(..3)? {
        "mon" => Some(Mon),
        "tue" => Some(Tue),
        "wed" => Some(Wed),
        "thu" => Some(Thu),
        "fri" => Some(Fri),
        "sat" => Some(Sat),
        "sun" => Some(Sun),
        _ => None,
    }
}

fn parse_month(tok: &str) -> Option<u32> {
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let prefix = tok.get(..3)?;
    months
        .iter()
        .position(|m| *m == prefix)
        .map(|i| i as u32 + 1)
}

fn parse_day_token(tok: &str) -> Option<u32> {
    let digits: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let suffix = &tok[digits.len()..];
    if !matches!(suffix, "" | "st" | "nd" | "rd" | "th") {
        return None;
    }
    let day: u32 = digits.parse().ok()?;
    (1..=31).contains(&day).then_some(day)
}

fn percent_before(text: &str, suffix: &str) -> Option<f64> {
    text.find(suffix).and_then(|pos| number_before(text, pos))
}

fn first_percent(text: &str) -> Option<f64> {
    text.find('%').and_then(|pos| number_before(text, pos))
}

/// Parses a decimal number (including floating points) immediately preceding the `end` byte offset in the text.
fn number_before(text: &str, end: usize) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut start = end;
    while start > 0 {
        let c = bytes[start - 1] as char;
        if c.is_ascii_digit() || c == '.' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == end {
        return None;
    }
    text[start..end].parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-07-08(Wed) 00:09 - Captured screen time from live verification.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 8)
            .unwrap()
            .and_hms_opt(0, 9, 0)
            .unwrap()
    }

    fn cd(days: u16, hours: u8, minutes: u8) -> Option<ResetCountdown> {
        Some(ResetCountdown {
            days,
            hours,
            minutes,
        })
    }

    #[test]
    fn claude_real_screen() {
        // claude v2.1.202 /usage live verification: % used + absolute reset time.
        let s = "\
   Current session
   ██▌                                                5% used
   Resets 5am (Asia/Seoul)

   Current week (all models)
   ██████████████████████▌                            45% used
   Resets Jul 10 at 5pm (Asia/Seoul)

   Current week (Fable)
   █████████████████▌                                 35% used
   Resets Jul 10 at 5pm (Asia/Seoul)";
        assert_eq!(
            parse_claude_at(s, now()),
            Some(UsageSnapshot {
                current: Some(UsageWindow {
                    pct_left: 95,
                    reset: cd(0, 4, 51)
                }),
                weekly: Some(UsageWindow {
                    pct_left: 55,
                    reset: cd(2, 16, 51)
                })
            })
        );
    }

    #[test]
    fn codex_real_screen() {
        // codex v0.142.5 /status live verification: % left + absolute reset time (HH:MM).
        let s = "\
│  5h limit:             [███████████████████░] 95% left (resets 04:45) │
│  Weekly limit:         [██████████░░░░░░░░░░] 51% left (resets 13:25) │";
        assert_eq!(
            parse_codex_at(s, now()),
            Some(UsageSnapshot {
                current: Some(UsageWindow {
                    pct_left: 95,
                    reset: cd(0, 4, 36)
                }),
                weekly: Some(UsageWindow {
                    pct_left: 51,
                    reset: cd(0, 13, 16)
                })
            })
        );
    }

    #[test]
    fn codex_weekday_and_date_resets() {
        // Weekday/date formats (%a %H:%M, %a %b %d). now is Wednesday -> next Monday = 7/13.
        let s = "\
5h limit: [████] 99% left (resets Mon 14:30)
Weekly limit: [██░░] 51% left (resets Mon Jul 13)";
        assert_eq!(
            parse_codex_at(s, now()),
            Some(UsageSnapshot {
                current: Some(UsageWindow {
                    pct_left: 99,
                    reset: cd(5, 14, 21)
                }),
                weekly: Some(UsageWindow {
                    pct_left: 51,
                    reset: cd(4, 23, 51)
                })
            })
        );
    }

    #[test]
    fn codex_time_on_day_month_reset() {
        // codex weekly reset live verification: `HH:MM on DD Mon`.
        let n = NaiveDate::from_ymd_opt(2026, 7, 9)
            .unwrap()
            .and_hms_opt(17, 39, 0)
            .unwrap();
        let s = "\
│  Weekly limit:         [███████████░░░░░░░░░] 57% left (resets 17:33 on 15 Jul) │";
        assert_eq!(
            parse_codex_at(s, n),
            Some(UsageSnapshot {
                current: None,
                weekly: Some(UsageWindow {
                    pct_left: 57,
                    reset: cd(5, 23, 54)
                })
            })
        );
    }

    #[test]
    fn agy_real_screen_claude_group() {
        // agy 1.0.16 /usage live verification: reads group for the active model (Claude),
        // leaving the Disabled 5h window as None.
        let s = "\
GEMINI MODELS
  Models within this group: Gemini Flash, Gemini Pro

  Weekly Limit
    [░░░░░░░░░░] 0.00%
    Refreshes in 16h 51m

  Five Hour Limit
    Disabled: You have hit your weekly limit, the 5-hour limit does not currently apply.

CLAUDE AND GPT MODELS
  Models within this group: Claude Opus, Claude Sonnet, GPT-OSS

  Weekly Limit
    [█████░░░░░] 59.00%
    Refreshes in 17h 46m

  Five Hour Limit
    [████████░░] 80.00%
    Refreshes in 2h 02m

esc to cancel                              Claude Sonnet 4.6 (Thinking)";
        assert_eq!(
            parse_agy_at(s, now()),
            Some(UsageSnapshot {
                current: Some(UsageWindow {
                    pct_left: 80,
                    reset: cd(0, 2, 2)
                }),
                weekly: Some(UsageWindow {
                    pct_left: 59,
                    reset: cd(0, 17, 46)
                })
            })
        );
    }

    #[test]
    fn agy_gemini_group_and_disabled_5h() {
        // Disabled 5h: remaining usage 0% + countdown parsed from the "fully refresh in ..." message.
        let s = "\
GEMINI MODELS
  Weekly Limit
    [░░░░░░░░░░] 0.00%
    Refreshes in 16h 36m

  Five Hour Limit
    Disabled: You have hit your weekly limit, the 5-hour limit does not currently apply. Your weekly limit will fully refresh in 16 hours, 37 minutes.

CLAUDE AND GPT MODELS
  Weekly Limit
    [█████░░░░░] 59.00%
    Refreshes in 17h 46m

esc to cancel                              Gemini Pro";
        assert_eq!(
            parse_agy_at(s, now()),
            Some(UsageSnapshot {
                current: Some(UsageWindow {
                    pct_left: 0,
                    reset: cd(0, 16, 37)
                }),
                weekly: Some(UsageWindow {
                    pct_left: 0,
                    reset: cd(0, 16, 36)
                })
            })
        );
    }

    #[test]
    fn agy_disabled_without_refresh_text() {
        // Retains 0% but omits the countdown if the refresh message is truncated.
        let s = "\
CLAUDE AND GPT MODELS
  Weekly Limit
    [░░░░░░░░░░] 0.00%
    Refreshes in 17h 46m

  Five Hour Limit
    Disabled: You have hit your weekly limit, the 5-hour limit does not currently app

esc to cancel                              Claude Sonnet 4.6 (Thinking)";
        assert_eq!(
            parse_agy_at(s, now()),
            Some(UsageSnapshot {
                current: Some(UsageWindow {
                    pct_left: 0,
                    reset: None
                }),
                weekly: Some(UsageWindow {
                    pct_left: 0,
                    reset: cd(0, 17, 46)
                })
            })
        );
    }

    #[test]
    fn reset_spec_variants() {
        let n = now();
        // Absolute time: shifts to next day if the time has already passed today.
        assert_eq!(parse_reset_spec("5am (asia/seoul)", n), cd(0, 4, 51));
        assert_eq!(parse_reset_spec("11:30pm", n), cd(0, 23, 21));
        assert_eq!(parse_reset_spec("00:05", n), cd(0, 23, 56)); // 00:05 < now -> tomorrow
        assert_eq!(parse_reset_spec("jul 10 at 5pm", n), cd(2, 16, 51));
        assert_eq!(parse_reset_spec("17:33 on 15 jul", n), cd(7, 17, 24));
        assert_eq!(parse_reset_spec("15 jul at 17:33", n), cd(7, 17, 24));
        // Relative notation.
        assert_eq!(parse_reset_spec("in 1d 14h", n), cd(1, 14, 0));
        assert_eq!(parse_reset_spec("in 2 hours", n), cd(0, 2, 0));
        assert_eq!(parse_reset_spec("in 163h", n), cd(6, 19, 0));
        assert_eq!(parse_reset_spec("", n), None);
    }

    #[test]
    fn parse_failure_returns_none() {
        assert_eq!(parse_claude_at("no numbers here", now()), None);
        assert_eq!(parse_codex_at("", now()), None);
        assert_eq!(parse_agy_at("plain text", now()), None);
    }
}
