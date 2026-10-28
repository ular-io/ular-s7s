//! CLI dispatch, the TUI event loop, agent handovers, and terminal lifecycle.
//! Entered via [`run`] from the thin `main.rs` binary shim.

use crate::ui::{App, Screen, UiMode};
use crate::{
    config, demo, handoff, model, models, profile, resume, scan, scratch, session_cli,
    session_context, ui, usage,
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    cursor::{MoveTo, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, Stdout, Write},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

type Tui = Terminal<CrosstermBackend<Stdout>>;

mod handover;
mod terminal;

use handover::*;
use terminal::*;

/// s7s — Search, inspect, and resume AI CLI sessions.
#[derive(Parser)]
#[command(
    name = "s7s",
    version,
    disable_version_flag = true,
    about = "s7s — Search, inspect, and resume AI CLI sessions (TUI when run without a command)",
    after_help = "\
DIR:      `s7s <dir>` starts the TUI with the New Session dialog already open on
          that folder (OK focused). The path is resolved against the current
          directory and must be an existing directory — unlike a bare name typed
          into the dialog, it is never resolved under ~/.config/s7s/projects.
          A folder whose name collides with a subcommand (session/demo/version/
          help) needs a path form: `s7s ./demo` or `s7s -- demo`.

PROFILES: ~/.config/s7s/profiles.json (builtin Claude/Antigravity/Codex + user-defined)
  Claude/Codex profiles support multiple subscriptions via CLAUDE_CONFIG_DIR/CODEX_HOME
CONFIG:   ~/.config/s7s/config.toml overrides command templates ({prompt} token supported
          in new_* templates for contextual launches)
CACHE:    <OS cache dir>/s7s/index.bin — macOS ~/Library/Caches/s7s
          (mtime incremental; rebuild with --rebuild-cache)

SESSION:   `s7s session search <query>` lists matching sessions and
           `s7s session list` lists them by filter alone; `s7s session show <id>`
           renders one session's context; `s7s session rename <id> <title>` sets a
           title; `s7s session change-folder <id> --to <dir>` changes the next
           resume folder; `s7s session delete <id> --yes` removes a session for good.
           `s7s session handoff --title <t>` parks a task in a new session to pick
           up later. Run `s7s session --help` or `s7s session <cmd> --help`."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
    /// Open the New Session dialog on this folder at startup (existing directory)
    // Subcommand names win over this positional: a folder named `session`/`demo`/
    // `version`/`help` needs a path form (`./demo`) or must follow `--`.
    #[arg(
        value_name = "DIR",
        conflicts_with_all = ["print", "usage_probe", "model_probe", "handoff_samples"]
    )]
    dir: Option<String>,
    /// Print version
    // Replaces clap's built-in flag (disabled above) so the short form is `-v`, not `-V`.
    // clap handles the action and exits; the field itself is never read.
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    version: Option<bool>,
    /// Force rebuild the entire session cache
    #[arg(long)]
    rebuild_cache: bool,
    /// Print the session list only, without TUI (debug)
    #[arg(long)]
    print: bool,
    /// Print usage probe results only, without TUI (debug)
    #[arg(long)]
    usage_probe: bool,
    /// Print model list probe results only, without TUI (debug; no cache update)
    #[arg(long)]
    model_probe: bool,
    /// Generate one deterministic handoff Markdown sample per agent
    #[arg(long, value_name = "DIR", num_args = 0..=1)]
    handoff_samples: Option<Option<std::path::PathBuf>>,
}

impl Cli {
    /// Whether `<DIR>` was given together with a subcommand. clap accepts that
    /// shape (the subcommand name matches at the first position, the path lands in
    /// the positional), so the rejection is owned here.
    fn dir_conflicts_with_subcommand(&self) -> bool {
        self.dir.is_some() && self.command.is_some()
    }
}

#[derive(Subcommand)]
enum CliCommand {
    /// Query and manage previous sessions: search/list, show, rename, change-folder, delete, handoff
    Session(session_cli::SessionArgs),
    /// Run s7s in demo mode using mock English sessions (disposable sandbox under the OS cache dir)
    Demo,
    /// Print version
    Version,
}

/// Runs s7s: parses the CLI, dispatches subcommands/debug modes, then drives the
/// TUI event loop until exit. This is the whole application entry point; `main`
/// only forwards to it.
pub fn run() -> Result<()> {
    let cli = Cli::parse();

    if cli.dir_conflicts_with_subcommand() {
        eprintln!("error: <DIR> cannot be combined with a subcommand.");
        eprintln!(
            "hint: `s7s <dir>` stands alone; a folder named like a subcommand needs a path form \
             (`s7s ./demo`)."
        );
        std::process::exit(2);
    }

    // Resolved before the index scan so a typo fails immediately instead of after
    // a full (first-run: slow) scan.
    let startup_dir = match cli.dir.as_deref() {
        Some(raw) => match resolve_startup_dir(raw) {
            Ok(dir) => Some(dir),
            Err(msg) => {
                eprintln!("error: {msg}");
                std::process::exit(2);
            }
        },
        None => None,
    };

    // `s7s version` mirrors the `-v` / `--version` flag output.
    if let Some(CliCommand::Version) = &cli.command {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(CliCommand::Demo) = &cli.command {
        config::set_demo_mode(true);
        demo::ensure_demo_sandbox(&config::demo_root())?;
    }

    // Session CLI mode: no TUI, no scan spinner; context to stdout, errors to stderr.
    if let Some(CliCommand::Session(args)) = &cli.command {
        std::process::exit(session_cli::run(args));
    }

    let rebuild_cache = cli.rebuild_cache;
    // Hidden debug: usage lookup only, without TUI.
    // When used with ULAR_USAGE_DUMP=<dir>, dumps the final screen text of each CLI.
    if cli.usage_probe {
        usage::probe();
        return Ok(());
    }
    // Hidden debug: model list lookup only, without TUI (no cache update).
    // Used for validation against actual CLI output (e.g. `/model`) after agent CLI upgrades.
    if cli.model_probe {
        models::probe();
        return Ok(());
    }
    let handoff_samples_dir = cli.handoff_samples.clone().map(|dir| {
        dir.unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("handoff-samples")
        })
    });

    let cfg = config::Config::load();
    let profiles = profile::ProfileStore::load();

    let cache_exists = config::cache_path().exists();
    let scan_message = if rebuild_cache {
        "Rebuilding session index"
    } else if cache_exists {
        "Updating session index"
    } else {
        "Building session index for the first run"
    };
    if rebuild_cache {
        eprintln!("Forcing a full cache rebuild. This may take a while.");
    } else if !cache_exists {
        eprintln!("First run may take a while. Later runs will be much faster.");
    }
    let spinner_done = Arc::new(AtomicBool::new(false));
    let spinner_flag = Arc::clone(&spinner_done);
    let spinner_message = scan_message.to_string();
    let spinner = thread::spawn(move || {
        let frames = ["|", "/", "-", "\\"];
        let mut i = 0usize;
        while !spinner_flag.load(Ordering::Relaxed) {
            eprint!("\r\x1b[K{} {}", frames[i % frames.len()], spinner_message);
            io::stderr().flush().ok();
            i += 1;
            thread::sleep(Duration::from_millis(120));
        }
    });

    let result = scan::scan(&profiles.profiles, rebuild_cache);
    spinner_done.store(true, Ordering::Relaxed);
    let _ = spinner.join();
    let scan_info = format!(
        "{} sessions · reparsed {}/{}",
        result.sessions.len(),
        result.reparsed_files,
        result.scanned_files
    );
    eprintln!("\r\x1b[K✓ {} complete ({})", scan_message, scan_info);

    if let Some(out_dir) = handoff_samples_dir {
        let reports = handoff::write_agent_samples(&result.sessions, &out_dir)?;
        if reports.is_empty() {
            println!("No handoff samples were generated.");
        } else {
            println!("Generated handoff samples:");
            for report in reports {
                println!(
                    "- {}\t{}\t{} turns\t{}",
                    report.agent.label(),
                    report.title,
                    report.turn_count,
                    report.path.to_string_lossy()
                );
            }
        }
        return Ok(());
    }

    // --print: Print session list only without TUI (for debugging/scripts).
    if cli.print {
        for s in &result.sessions {
            println!(
                "{}\t{}\t{}\t{}",
                s.agent.key(),
                s.date_str(),
                s.folder,
                s.title()
            );
        }
        return Ok(());
    }

    let mut app = App::new(cfg, profiles, result.sessions, scan_info);
    // `s7s <dir>`: the dialog opens over the ordinary session list, so cancelling
    // lands in the normal TUI instead of exiting.
    if let Some(dir) = startup_dir {
        app.open_new_session_for_dir(dir);
    }
    // Query agent usage in the background at app startup (shown in the header).
    app.start_usage_fetch();
    // Also update model lists in the background (version gate - keeps cache if CLI version is unchanged).
    app.start_models_fetch(false);

    // Installed before the TUI takes the screen so a panic inside the loop prints
    // its message on the restored main screen instead of the alternate screen.
    install_panic_hook();
    let mut session = TerminalSession::enter()?;
    // Debug-only fault injection for the manual restoration check in
    // docs/testing.md: an unwind panic while the TUI owns the screen must restore
    // the terminal AND leave its message readable on the main screen. Compiled out
    // of release builds.
    #[cfg(debug_assertions)]
    if std::env::var_os("S7S_PANIC_PROBE").is_some() {
        panic!("panic probe: verifying terminal restoration");
    }
    let loop_result = run_loop(&mut session, &mut app);
    // Explicit cleanup so its failure is reported; `Drop` still retries whatever
    // stayed active.
    let cleanup = session.suspend();
    drop(session);
    match (loop_result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(err)) => Err(anyhow::Error::new(err).context("restoring the terminal failed")),
        // Never let cleanup noise replace the real failure; keep both.
        (Err(err), Ok(())) => Err(err),
        (Err(err), Err(cleanup_err)) => {
            Err(err.context(format!("terminal cleanup also failed: {cleanup_err}")))
        }
    }
}

/// Resolves the positional `<DIR>` into an absolute, canonical directory.
///
/// Command-line semantics deliberately differ from the New Session dialog, where a
/// separator-less name resolves under `config::projects_dir()`: a path given on the
/// command line always means what the shell means by it, so `s7s .` is the process
/// cwd. Handing the dialog an absolute path keeps it from re-interpreting the input.
fn resolve_startup_dir(raw: &str) -> Result<std::path::PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("<DIR> is empty".to_string());
    }
    // `~/…` is normally expanded by the shell; this covers the quoted form.
    let expanded = config::expand(trimmed);
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|e| format!("cannot resolve the current directory: {e}"))?
            .join(expanded)
    };
    let canonical = std::fs::canonicalize(&joined)
        .map_err(|e| format!("cannot open '{}': {e}", joined.display()))?;
    if !canonical.is_dir() {
        return Err(format!("'{}' is not a directory", canonical.display()));
    }
    Ok(canonical)
}

/// Main event loop.
/// Coalesces input events: waits blocks for the first event, but drains any pending
/// remaining events to update the state, then redraws **exactly once**.
/// Rapidly pressing or holding navigation keys won't redraw on every frame, ensuring immediate cursor movement.
///
/// Global refresh draws its preparing frame before spawning a worker. Polling
/// at every frame also applies ready results during sustained keyboard input.
fn run_loop(session: &mut TerminalSession, app: &mut App) -> Result<()> {
    loop {
        app.poll_background();
        session.terminal_mut().draw(|f| ui::render::draw(f, app))?;
        app.finish_refresh_cycle();
        app.start_scheduled_refresh_scan();

        // 1) Wait for the first event. If background jobs are in progress, poll with a short
        //    timeout so that background updates trigger a redraw.
        loop {
            // 100ms: Keep at half the pulse step duration (render.rs PULSE_STEP_MS 200ms)
            //        to prevent step skipping between redraws.
            let timeout = if app.background_in_flight() {
                Duration::from_millis(100)
            } else {
                // Wake for the periodic live-session status sweep.
                app.agent_status_wait().max(Duration::from_millis(100))
            };
            if event::poll(timeout)? {
                dispatch_event(app, event::read()?);
                app.apply_effect();
                break;
            }
            let updated = app.poll_background();
            if updated || app.background_in_flight() {
                break; // Background update or loading animation frame -> redraw
            }
        }

        // 2) Drain remaining queued events immediately (reflecting state without redrawing).
        drain_queued_events(app)?;

        // Process resume request: exit TUI -> execute agent -> return to TUI.
        if let Some(idx) = app.resume_request.take() {
            let target = app.sessions[idx].clone();
            handover(session, app, &target)?;
        }
        if let Some(req) = app.attach_request.take() {
            if let Some(target) = app.sessions.get(req.idx).cloned() {
                handover_attach(session, app, &target, &req.job)?;
            }
        }
        if let Some(req) = app.new_session_request.take() {
            handover_new_session(session, app, req)?;
        }
        if let Some(profile_id) = app.login_request.take() {
            handover_login(session, app, profile_id)?;
        }
        if let Some(req) = app.terminal_request.take() {
            handover_terminal(session, app, req)?;
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Drains queued input events without redrawing, applying each event and its
/// effect. Stops at quit or any handover request so the caller processes those
/// on the current pass instead of blocking on further input.
fn drain_queued_events(app: &mut App) -> Result<()> {
    while !app.should_quit
        && app.resume_request.is_none()
        && app.attach_request.is_none()
        && app.new_session_request.is_none()
        && app.login_request.is_none()
        && app.terminal_request.is_none()
        && event::poll(Duration::from_millis(0))?
    {
        dispatch_event(app, event::read()?);
        app.apply_effect();
    }
    Ok(())
}

/// Reflects a single input event into the App state based on the current UI mode (redrawing is handled by caller).
fn dispatch_event(app: &mut App, ev: Event) {
    match ev {
        Event::Key(key) => {
            if key.kind != KeyEventKind::Press {
                return;
            }
            match app.mode {
                UiMode::Table => match app.screen {
                    Screen::Session => app.on_key_table(key),
                    Screen::Profile => app.on_key_profile_table(key),
                    Screen::Detail => app.on_key_detail(key),
                },
                UiMode::Keyword => app.on_key_keyword(key),
                UiMode::AgentModal => app.on_key_agent_modal(key),
                UiMode::FolderModal => app.on_key_folder_modal(key),
                UiMode::DeleteConfirm => app.on_key_delete_confirm(key),
                UiMode::AttachConfirm => app.on_key_attach_confirm(key),
                UiMode::Rename => app.on_key_rename_modal(key),
                UiMode::ProfileForm => app.on_key_profile_form(key),
                UiMode::ProfileDeleteConfirm => app.on_key_profile_delete_confirm(key),
                UiMode::ProfileDirConfirm => app.on_key_profile_dir_confirm(key),
                UiMode::NewSession => app.on_key_new_session(key),
                UiMode::ChangeFolder => app.on_key_change_folder(key),
                UiMode::ProjectDirConfirm => app.on_key_project_dir_confirm(key),
                UiMode::QuickCommand => app.on_key_quick(key),
                UiMode::ThemeSelect => app.on_key_theme_select(key),
                UiMode::Help => app.on_key_help(key),
                UiMode::Message => app.on_key_message(key),
                UiMode::WorkspaceEdit => app.on_key_workspace_dialog(key),
                UiMode::WorkspaceDeleteConfirm => app.on_key_workspace_delete_confirm(key),
            }
        }
        // One bracketed paste stays one event: it is routed to the focused editable
        // field as text and must never be expanded into synthetic key events (a
        // pasted newline would otherwise submit a dialog or run a `!` command).
        Event::Paste(text) => app.on_paste(&text),
        Event::Resize(_, _) => { /* Reflect in next redraw */ }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
