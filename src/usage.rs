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
use crate::probe::process::installed;
use crate::probe::pty::{drive_screen, DriveOutcome};
use crate::probe::{claude_logged_in, CLAUDE_READY_MARKERS};
use crate::profile::Profile;
use parse::{parse_agy, parse_claude, parse_codex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

mod parse;
mod shared;

pub(crate) use shared::{entry_for, load as load_shared};
pub use shared::{now_ms, Fetched, Origin, Outcome, SharedEntry};

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
/// The state changes on this instance's queries (app startup, Ctrl+U, profile
/// save) and when another instance stores a result in the shared `usage.json`.
/// `last_at` (Unix ms) is when `last` was read from the CLI; the two are set
/// together, and reset countdowns in `last` count from `last_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageEntry {
    pub phase: UsagePhase,
    pub last: Option<UsageSnapshot>,
    pub last_at: Option<i64>,
}

impl UsageEntry {
    pub fn idle() -> Self {
        UsageEntry {
            phase: UsagePhase::Idle,
            last: None,
            last_at: None,
        }
    }

    /// Takes the phase and last success from a shared entry. A `Loading` entry
    /// keeps its phase (its own query is still running) and only picks up a
    /// newer value. Returns whether anything changed.
    pub fn apply(&mut self, shared: &SharedEntry) -> bool {
        let before = *self;
        match shared.success {
            Some(s) if self.phase != UsagePhase::Loading || self.last_at < Some(s.at_ms) => {
                self.last = Some(s.snapshot);
                self.last_at = Some(s.at_ms);
            }
            None if self.phase != UsagePhase::Loading => {
                self.last = None;
                self.last_at = None;
            }
            _ => {}
        }
        if self.phase != UsagePhase::Loading {
            if let Some(attempt) = shared.attempt {
                self.phase = match attempt.outcome {
                    Outcome::Ready => UsagePhase::Ready,
                    Outcome::Failed => UsagePhase::Failed,
                    Outcome::NotLoggedIn => UsagePhase::NotLoggedIn,
                    Outcome::NotInstalled => UsagePhase::NotInstalled,
                    Outcome::MissingDir => UsagePhase::MissingDir,
                    Outcome::Unavailable => UsagePhase::Unavailable,
                };
            }
        }
        *self != before
    }
}

/// Remaining usage in an individual window (e.g., 5h, weekly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UsageWindow {
    pub pct_left: u8,
    pub reset: Option<ResetCountdown>,
}

/// Two usage windows displayed in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UsageSnapshot {
    pub current: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
}

/// Reset countdown normalized into simple day/hour/minute units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetCountdown {
    pub days: u16,
    pub hours: u8,
    pub minutes: u8,
}

impl ResetCountdown {
    fn total_minutes(self) -> i64 {
        self.days as i64 * 24 * 60 + self.hours as i64 * 60 + self.minutes as i64
    }

    fn from_minutes(total: i64) -> Self {
        ResetCountdown {
            days: (total / (24 * 60)) as u16,
            hours: (total / 60 % 24) as u8,
            minutes: (total % 60) as u8,
        }
    }

    /// This countdown, read at `taken_at_ms`, as seen at `now_ms`: rounded up
    /// to the minute so a fresh value reads as captured. `None` once the reset
    /// time has passed (the stored percentage no longer applies).
    pub fn remaining(self, taken_at_ms: i64, now_ms: i64) -> Option<ResetCountdown> {
        let left_ms = self.total_minutes() * 60_000 - now_ms.saturating_sub(taken_at_ms).max(0);
        (left_ms > 0).then(|| Self::from_minutes((left_ms + 59_999) / 60_000))
    }
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

/// Starts parallel queries through the shared cache at `store` (see `shared`)
/// and returns a receiver for (profile_id, result). Like [`spawn_fetch`], each
/// result is held for at least `MIN_LOADING`.
pub fn spawn_fetch_shared(profiles: Vec<Profile>, store: PathBuf) -> Receiver<(String, Fetched)> {
    let (tx, rx) = mpsc::channel();
    let requested = now_ms();
    for profile in profiles {
        let tx = tx.clone();
        let store = store.clone();
        thread::spawn(move || {
            let started = Instant::now();
            let res = shared::fetch_shared(&profile, &store, requested);
            if let Some(rem) = MIN_LOADING.checked_sub(started.elapsed()) {
                thread::sleep(rem);
            }
            let _ = tx.send((profile.id, res));
        });
    }
    rx
}

/// Debug option (`--usage-probe`): Queries all profiles in parallel and prints
/// the results. Always runs the CLIs and never touches the shared cache.
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
    // The dump env name is shared with the model probe (see docs/usage-display.md, docs/models.md).
    match drive_screen(
        cmd,
        slash_cmd,
        ready_markers,
        done_markers,
        logout_markers,
        min_wait,
        enter_delay,
        envs,
        Some("ULAR_USAGE_DUMP"),
        None,
    ) {
        Ok(DriveOutcome::Screen(text)) => parse(&text)
            .map(UsageResult::Ready)
            .unwrap_or_else(|| UsageResult::Failed(format!("{cmd}: failed to parse usage"))),
        Ok(DriveOutcome::NotLoggedIn) => UsageResult::NotLoggedIn,
        Err(e) => UsageResult::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod parse_tests;
