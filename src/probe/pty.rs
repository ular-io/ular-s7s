//! PTY lifecycle, screen capture, and child-process cleanup for CLI screen probes.

use super::process::{collect_descendants, parent_pid, pids_of};
use anyhow::{anyhow, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Screen text of the folder-trust dialogs that block a CLI before it reaches the
/// prompt. Version-specific wording (claude 2.1.220, agy 1.1.8) — recheck on upgrade.
const TRUST_PROMPT_MARKERS: &[&str] = &[
    "Do you trust the files in this folder",
    "Is this a project you created or one you trust",
    "Do you trust the contents of this project",
];

/// Row label of the approval choice in those dialogs (identical across claude and agy).
const TRUST_APPROVE_ROW: &str = "Yes, I trust this folder";

/// How long the dialog may stay on screen after Enter was sent before the probe gives
/// up: if confirming did not dismiss it, the dialog is not the one we recognized.
const TRUST_CONFIRM_GRACE: Duration = Duration::from_secs(8);

/// Whether the trust dialog's *highlighted* row is the approval choice, i.e. whether
/// Enter confirms trust rather than declining it. Guards the auto-confirm against a
/// reordered menu: the caret has to sit on the row that grants trust.
fn trust_approval_selected(screen: &str) -> bool {
    screen.lines().any(|line| {
        let row = line.trim_start();
        (row.starts_with('❯') || row.starts_with('>')) && row.contains(TRUST_APPROVE_ROW)
    })
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
///
/// `dump_env` names an environment variable that, when set to a directory, receives a dump of the
/// final screen text (client-chosen so this driver stays neutral about what is being probed).
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
    dump_env: Option<&str>,
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
    // Fixed probe directory, never the s7s launch directory: startup dialogs keyed on
    // the working directory (claude trust prompt, project MCP approval) would otherwise
    // make the query fail for whole folders. See `probe::probe_cwd`.
    let probe_cwd = super::probe_cwd();
    if let Some(cwd) = &probe_cwd {
        builder.cwd(&cwd.path);
    }
    let may_confirm_trust = probe_cwd.is_some_and(|cwd| cwd.dedicated);
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
    // When the trust dialog was confirmed, so a dialog that survives the grace period
    // is reported as untrusted instead of silently eating the whole ready timeout.
    let mut trust_confirmed_at: Option<Instant> = None;

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
            // A folder-trust dialog blocks the boot this probe waits for, and agy raises it
            // for every workspace missing from `trustedWorkspaces`, so the dedicated probe
            // folder needs confirming once per agent config. Gated on that folder and on the
            // caret sitting on the approval row. Never falls through to typing while a dialog
            // is up (`/usage` would land in the menu), and runs before the logout grace so
            // agy's boot-time "not signed in" line cannot become `NotLoggedIn`.
            if TRUST_PROMPT_MARKERS.iter().any(|m| screen.contains(m)) {
                match trust_confirmed_at {
                    Some(at) if at.elapsed() > TRUST_CONFIRM_GRACE => {
                        break Err(anyhow!("{cmd}: trust prompt not dismissed by confirm"));
                    }
                    Some(_) => continue,
                    None if may_confirm_trust && trust_approval_selected(&screen) => {
                        let _ = writer.write_all(b"\r");
                        let _ = writer.flush();
                        trust_confirmed_at = Some(Instant::now());
                        continue;
                    }
                    None => break Err(anyhow!("{cmd}: untrusted folder (trust prompt)")),
                }
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

    // Debug: If the client-provided dump env variable is set to a directory, dump the final screen contents to a file.
    // Includes the command name in the filename since the same CLI is queried with different slash commands.
    if let Some(dir) = dump_env.and_then(std::env::var_os) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// agy 1.1.8 dialog, approval row selected.
    const AGY_TRUST: &str = "\n Accessing workspace:\n\n /Users/x/.config/s7s/probe\n\n \
         Do you trust the contents of this project?\n\n > Yes, I trust this folder\n   No, exit\n";

    /// claude 2.1.220 dialog, approval row selected (numbered rows).
    const CLAUDE_TRUST: &str = "\n Quick safety check: Is this a project you created or one \
         you trust?\n\n ❯ 1. Yes, I trust this folder\n   2. No, continue without these \
         permissions\n";

    #[test]
    fn approval_row_is_recognized_for_both_dialog_layouts() {
        assert!(trust_approval_selected(AGY_TRUST));
        assert!(trust_approval_selected(CLAUDE_TRUST));
        assert!(TRUST_PROMPT_MARKERS.iter().any(|m| AGY_TRUST.contains(m)));
        assert!(TRUST_PROMPT_MARKERS
            .iter()
            .any(|m| CLAUDE_TRUST.contains(m)));
    }

    #[test]
    fn confirm_is_withheld_unless_the_caret_sits_on_the_approval_row() {
        // Reordered menu with the decline row selected: Enter would decline, so the
        // probe must not send it and instead report the folder as untrusted.
        let decline_selected = AGY_TRUST
            .replace("> Yes, I trust this folder", "  Yes, I trust this folder")
            .replace("   No, exit", " > No, exit");
        assert!(!trust_approval_selected(&decline_selected));
        // Approval text alone (no caret anywhere) is not a selection either.
        assert!(!trust_approval_selected("  Yes, I trust this folder\n"));
        // An ordinary boot screen carries neither the prompt nor a selection.
        assert!(!trust_approval_selected("? for shortcuts\n"));
    }
}
