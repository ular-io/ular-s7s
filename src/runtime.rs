//! CLI dispatch, the TUI event loop, agent handovers, and terminal lifecycle.
//! Entered via [`run`] from the thin `main.rs` binary shim.

use crate::ui::{App, Screen, UiMode};
use crate::{
    config, demo, handoff, model, models, profile, resume, scan, session_cli, session_context, ui,
    usage,
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

SESSION:   `s7s session show <id>` renders one session's context;
           `s7s session search <query>` lists matching sessions.
           Run `s7s session --help` / `s7s session search --help` for examples."
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
    /// Query previous sessions: `show` one session's context or `search` by keyword
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
/// Two-phase global refresh (Ctrl+U): the effect only prepares (background
/// probes + status), so the draw above shows the loading state first; the
/// scheduled synchronous session scan then runs here right after that draw,
/// without waiting for another input event. Input queued while the scan ran is
/// drained before the cycle ends, so repeat Ctrl+U presses merge into one scan.
fn run_loop(session: &mut TerminalSession, app: &mut App) -> Result<()> {
    loop {
        session.terminal_mut().draw(|f| ui::render::draw(f, app))?;

        if app.refresh_scan_scheduled() {
            // The preparing frame is on screen: run the scheduled scan now.
            app.run_scheduled_refresh_scan();
            // Keys queued during the scan apply normally, but a queued Ctrl+U
            // merges into this still-active cycle instead of rescanning.
            drain_queued_events(app)?;
            app.finish_refresh_cycle();
            // Fall through to the request handling below; the next iteration's
            // draw renders the scan result without waiting for input.
        } else {
            // 1) Wait for the first event. If usage or model queries are in progress, poll with a short
            //    timeout so that background updates trigger a redraw.
            loop {
                // 100ms: Keep at half the pulse step duration (render.rs PULSE_STEP_MS 200ms)
                //        to prevent step skipping between redraws.
                let timeout = if app.background_in_flight() {
                    Duration::from_millis(100)
                } else {
                    Duration::from_secs(3600)
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
        }

        // Process resume request: exit TUI -> execute agent -> return to TUI.
        if let Some(idx) = app.resume_request.take() {
            let target = app.sessions[idx].clone();
            handover(session, app, &target)?;
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
                UiMode::Rename => app.on_key_rename_modal(key),
                UiMode::ProfileForm => app.on_key_profile_form(key),
                UiMode::ProfileDeleteConfirm => app.on_key_profile_delete_confirm(key),
                UiMode::ProfileDirConfirm => app.on_key_profile_dir_confirm(key),
                UiMode::NewSession => app.on_key_new_session(key),
                UiMode::ProjectDirConfirm => app.on_key_project_dir_confirm(key),
                UiMode::QuickCommand => app.on_key_quick(key),
                UiMode::ThemeSelect => app.on_key_theme_select(key),
                UiMode::Help => app.on_key_help(key),
                UiMode::Message => app.on_key_message(key),
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

/// Hands over TUI control to the agent CLI, then returns. Filter state remains intact in App.
fn handover(tui: &mut TerminalSession, app: &mut App, session: &model::Session) -> Result<()> {
    // 1) Temporarily release the terminal (raw, alternate screen, paste, keyboard).
    tui.suspend()?;

    // Inject environmental variables of the session's profile to run under the correct subscription/account.
    let profile = app.profiles.find(&session.profile_id).cloned();

    // 2) Synchronous execution after printing notice.
    print_handover_screen(
        &format!(
            "[{}] resume: {}",
            session.agent.label(),
            session.cwd.to_string_lossy()
        ),
        &resume::preview_command(session, &app.cfg, profile.as_ref()),
    );
    match resume::run(session, &app.cfg, profile.as_ref()) {
        Ok(status) => {
            print_returning_notice();
            // On abnormal exit (command missing/immediate failure etc.), wait so the error doesn't vanish instantly.
            // Note: User interruption (like rapid Ctrl+C) is treated as a normal return.
            if !status.success() && !resume::interrupted_by_user(&status) {
                eprintln!(
                    "\n⚠ Agent exited abnormally (exit code: {}).",
                    status.code().unwrap_or(-1)
                );
                eprintln!(
                    "  command: {}",
                    resume::preview_command(session, &app.cfg, profile.as_ref())
                );
                pause_before_return();
            }
        }
        Err(e) => {
            print_returning_notice();
            eprintln!("\n⚠ failed to run resume: {e}");
            eprintln!(
                "  command: {}",
                resume::preview_command(session, &app.cfg, profile.as_ref())
            );
            pause_before_return();
        }
    }

    // 3) Re-acquire the terminal and force a full redraw.
    tui.resume()?;
    // Reflect new conversations continued during resume: perform an incremental
    // rescan and sort by semantic activity (selection tracks the same session).
    app.refresh_sessions();
    drain_pending_input();
    app.begin_quit_grace();
    app.status_msg = Some(format!("Returned from resume: {}", session.folder));
    Ok(())
}

/// Hands over TUI control to the agent CLI to start a new session in the specified folder.
fn handover_new_session(
    tui: &mut TerminalSession,
    app: &mut App,
    req: ui::NewSessionRequest,
) -> Result<()> {
    tui.suspend()?;

    let profile = app.profiles.find(&req.profile_id).cloned();
    let Some(profile) = profile else {
        tui.resume()?;
        app.status_msg = Some("Profile no longer exists".to_string());
        return Ok(());
    };

    let model = req.model.as_deref();
    // Contextual launch: inject the short English bootstrap prompt derived from the
    // immutable SOURCE reference. Only the target profile's env is used for the
    // agent itself; the source profile ID travels inside the generated `s7s session`
    // command so the child s7s process resolves the correct source independently.
    let bootstrap = req
        .context
        .as_ref()
        .map(|c| session_context::render::bootstrap_prompt(c.agent, &c.profile_id, &c.session_id));
    let prompt = bootstrap.as_deref();
    let header = match &req.context {
        Some(c) => format!(
            "[{}] new session with context ({} · {}): {}",
            profile.agent.label(),
            c.agent.label(),
            c.session_id,
            req.cwd.to_string_lossy()
        ),
        None => format!(
            "[{}] new session: {}",
            profile.agent.label(),
            req.cwd.to_string_lossy()
        ),
    };
    print_handover_screen(
        &header,
        &resume::preview_new_command(
            profile.agent,
            &req.cwd,
            &app.cfg,
            Some(&profile),
            model,
            prompt,
        ),
    );
    match resume::run_new(
        profile.agent,
        &req.cwd,
        &app.cfg,
        Some(&profile),
        model,
        prompt,
    ) {
        Ok(status) => {
            print_returning_notice();
            if !status.success() && !resume::interrupted_by_user(&status) {
                eprintln!(
                    "\n⚠ Agent exited abnormally (exit code: {}).",
                    status.code().unwrap_or(-1)
                );
                eprintln!(
                    "  command: {}",
                    resume::preview_new_command(
                        profile.agent,
                        &req.cwd,
                        &app.cfg,
                        Some(&profile),
                        model,
                        prompt
                    )
                );
                pause_before_return();
            }
        }
        Err(e) => {
            print_returning_notice();
            eprintln!("\n⚠ failed to start new session: {e}");
            eprintln!(
                "  command: {}",
                resume::preview_new_command(
                    profile.agent,
                    &req.cwd,
                    &app.cfg,
                    Some(&profile),
                    model,
                    prompt
                )
            );
            pause_before_return();
        }
    }

    tui.resume()?;
    app.screen = Screen::Session;
    app.refresh_sessions();
    drain_pending_input();
    app.begin_quit_grace();
    app.status_msg = Some(format!(
        "Returned from new session: {}",
        req.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| req.cwd.to_string_lossy().into_owned())
    ));
    Ok(())
}

/// Hands over TUI control to the agent CLI to perform login (initial setup) in a new config folder.
/// Unlike resume/new, executes the base flag-less command from the current directory of s7s without cd.
fn handover_login(tui: &mut TerminalSession, app: &mut App, profile_id: String) -> Result<()> {
    tui.suspend()?;

    let profile = app.profiles.find(&profile_id).cloned();
    let Some(profile) = profile else {
        tui.resume()?;
        app.status_msg = Some("Profile no longer exists".to_string());
        return Ok(());
    };

    print_handover_screen(
        &format!(
            "[{}] login: complete login, then exit the agent to return to s7s",
            profile.agent.label()
        ),
        &resume::preview_login_command(&profile),
    );
    match resume::run_login(&profile) {
        Ok(status) => {
            print_returning_notice();
            if !status.success() && !resume::interrupted_by_user(&status) {
                eprintln!(
                    "\n⚠ Agent exited abnormally (exit code: {}).",
                    status.code().unwrap_or(-1)
                );
                eprintln!("  command: {}", resume::preview_login_command(&profile));
                pause_before_return();
            }
        }
        Err(e) => {
            print_returning_notice();
            eprintln!("\n⚠ failed to run agent for login: {e}");
            eprintln!("  command: {}", resume::preview_login_command(&profile));
            pause_before_return();
        }
    }

    tui.resume()?;
    // Reflect changes immediately after login: rescan sessions + incrementally query usage for this profile.
    app.refresh_sessions();
    app.start_usage_fetch_for(&[profile_id]);
    drain_pending_input();
    app.begin_quit_grace();
    app.status_msg = Some(format!("Returned from login: {}", profile.name));
    Ok(())
}

/// Hands over TUI control to run a user shell command in the session's folder (`!` terminal mode).
///
/// Unlike agent handovers, waits for a keypress after the command exits so that short-lived
/// output is not wiped by the immediate TUI redraw — unless the request opts out
/// (`pause: false`, Edit Config: an interactive editor leaves no output to read).
/// Failures always wait so the error message stays visible.
fn handover_terminal(
    tui: &mut TerminalSession,
    app: &mut App,
    req: ui::TerminalRequest,
) -> Result<()> {
    tui.suspend()?;

    let editor = app.cfg.editor.clone();
    let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
    println!(
        "▶ [terminal] {}\n  {}\n",
        req.cwd.to_string_lossy(),
        resume::preview_terminal_command(&req.cwd, &req.command, editor.as_deref())
    );
    match resume::run_terminal(&req.cwd, &req.command, editor.as_deref()) {
        Ok(status) => {
            // User interruption (Ctrl+C) is a normal way to stop a command; no warning for it.
            if !status.success() && !resume::interrupted_by_user(&status) {
                eprintln!(
                    "\n⚠ Command exited abnormally (exit code: {}).",
                    status.code().unwrap_or(-1)
                );
                after_terminal_failure(req.kind);
            } else if req.kind == ui::TerminalKind::Command {
                pause_before_return();
            }
        }
        Err(e) => {
            eprintln!("\n⚠ failed to run command: {e}");
            after_terminal_failure(req.kind);
        }
    }

    tui.resume()?;
    // The command may have edited config.toml (Edit Config palette command) or touched
    // session files/workspace folders; reload config before the (mtime-cached) rescan.
    app.cfg = config::Config::load();
    app.refresh_sessions();
    drain_pending_input();
    app.begin_quit_grace();
    app.status_msg = Some(format!("Returned from terminal: {}", req.command));
    Ok(())
}

/// Post-failure handling for a terminal handover: `!` commands wait for a keypress
/// (keeping the error readable); Edit Config offers a vim fallback instead, since a
/// broken `editor` value in config.toml could not be fixed from within s7s otherwise.
fn after_terminal_failure(kind: ui::TerminalKind) {
    match kind {
        ui::TerminalKind::Command => pause_before_return(),
        ui::TerminalKind::EditConfig => offer_vim_retry(),
    }
}

/// Asks whether to reopen config.toml with vim after the configured editor failed,
/// and runs it on confirmation. vim is a deliberate fixed fallback: it is present on
/// virtually every system and independent of the (possibly broken) `editor` value.
fn offer_vim_retry() {
    eprintln!(
        "\nPress y to open the config with vim instead, any other key to return to the TUI..."
    );
    let yes = if let Some(_raw) = RawModeGuard::enter() {
        drain_pending_input();
        // The guard disables raw mode on every exit path, including a panic.
        matches!(
            event::read(),
            Ok(Event::Key(k))
                if k.kind == KeyEventKind::Press
                    && matches!(k.code, event::KeyCode::Char('y' | 'Y'))
        )
    } else {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).ok();
        buf.trim().eq_ignore_ascii_case("y")
    };
    if !yes {
        return;
    }
    let path = config::config_file_path();
    let cmd = format!("vim {}", resume::shell_quote(&path.to_string_lossy()));
    match resume::run_terminal(&config::config_base_dir(), &cmd, None) {
        Ok(status) if !status.success() && !resume::interrupted_by_user(&status) => {
            eprintln!(
                "\n⚠ vim exited abnormally (exit code: {}).",
                status.code().unwrap_or(-1)
            );
            pause_before_return();
        }
        Err(e) => {
            eprintln!("\n⚠ failed to run vim: {e}");
            pause_before_return();
        }
        _ => {}
    }
}

/// Clears the main screen and prints a handover notice banner immediately before handover.
/// This main screen is what gets revealed the moment a fullscreen (alternate screen) agent exits
/// and closes its display. We pre-render a "closing/returning" notice to show during the 1-3 second cleanup
/// instead of leftover shell command residues (the same banner appears briefly right before starting).
/// For inline agents, output appends below, pushing the banner out of sight; in that case,
/// print_returning_notice handles the return notice.
fn print_handover_screen(header: &str, command: &str) {
    let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
    println!("▶ {header}\n  {command}\n");
    println!("⏳ Agent is starting or closing — s7s will return automatically. Please wait…\n");
}

/// Clears screen and prints return-in-progress notice immediately after child process exits (visible until TUI re-entry).
fn print_returning_notice() {
    let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
    println!("⏳ Agent exited — returning to s7s…");
}

/// Discards pending keyboard inputs in the buffer immediately after returning from the agent.
/// Prevents keys pressed during the wait (such as rapid Ctrl+C) from flooding the TUI as event queue triggers.
fn drain_pending_input() {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }
}

/// Wait for any keypress so the user has time to read the last screen (agent execution
/// failures and terminal command completion). Flushes buffered keys first so leftover
/// keystrokes (e.g. rapid Ctrl+C spam) do not satisfy the wait instantly.
fn pause_before_return() {
    eprintln!("\nPress any key to return to the TUI...");
    if let Some(_raw) = RawModeGuard::enter() {
        drain_pending_input();
        let _ = event::read();
    } else {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).ok();
    }
}

/// A terminal state s7s turns on while the TUI owns the screen. Each one must be
/// turned off again before a child process or the parent shell sees the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalMode {
    Raw,
    AlternateScreen,
    BracketedPaste,
    KeyboardEnhancement,
}

impl TerminalMode {
    fn name(self) -> &'static str {
        match self {
            TerminalMode::Raw => "raw mode",
            TerminalMode::AlternateScreen => "alternate screen",
            TerminalMode::BracketedPaste => "bracketed paste",
            TerminalMode::KeyboardEnhancement => "keyboard enhancement flags",
        }
    }

    fn bit(self) -> u8 {
        match self {
            TerminalMode::Raw => 1,
            TerminalMode::AlternateScreen => 1 << 1,
            TerminalMode::BracketedPaste => 1 << 2,
            TerminalMode::KeyboardEnhancement => 1 << 3,
        }
    }
}

/// Low-level terminal state operations. Abstracted so the lifecycle policy in
/// [`TerminalModes`] — ordering, flag bookkeeping, and failure recovery — is unit
/// testable with injected failures instead of only against a real terminal.
trait TerminalOps {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()>;
    fn disable(&mut self, mode: TerminalMode) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    /// Whether the terminal speaks the kitty keyboard enhancement protocol.
    fn supports_keyboard_enhancement(&mut self) -> bool;
}

/// Modes currently enabled by [`CrosstermOps`], mirrored into a global so the
/// panic hook can restore the terminal. `Drop` cannot serve that purpose alone:
/// the runtime prints the panic message *before* unwinding runs destructors, so
/// the message would land on the alternate screen and be erased with it.
static ACTIVE_MODES: AtomicU8 = AtomicU8::new(0);

/// Real terminal operations against `stdout`.
struct CrosstermOps;

impl TerminalOps for CrosstermOps {
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        // [`ACTIVE_MODES`] is the single source of truth for the real terminal, so
        // a redundant enable/disable is a no-op here. That matters after a panic:
        // the hook already restored the terminal, and the still-unwinding
        // `TerminalSession::drop` must not pop a second keyboard-enhancement level
        // — which would belong to whatever process launched s7s.
        if ACTIVE_MODES.load(Ordering::Relaxed) & mode.bit() != 0 {
            return Ok(());
        }
        let mut out = io::stdout();
        match mode {
            TerminalMode::Raw => enable_raw_mode()?,
            // App does not handle mouse events. Capturing mouse events intercepts default terminal
            // text selection, so we only activate the alternate screen.
            TerminalMode::AlternateScreen => execute!(out, EnterAlternateScreen)?,
            TerminalMode::BracketedPaste => execute!(out, EnableBracketedPaste)?,
            // Enhanced keyboard protocol (where supported): lets the terminal report
            // Ctrl+Shift+N distinctly from Ctrl+N (legacy encoding sends the same control
            // byte for both). DISAMBIGUATE_ESCAPE_CODES is sufficient and keeps plain
            // printable-key handling unchanged. Unsupported terminals keep legacy input;
            // the Quick Command palette is the functional fallback there.
            TerminalMode::KeyboardEnhancement => execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?,
        }
        ACTIVE_MODES.fetch_or(mode.bit(), Ordering::Relaxed);
        Ok(())
    }

    fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
        if ACTIVE_MODES.load(Ordering::Relaxed) & mode.bit() == 0 {
            return Ok(());
        }
        let mut out = io::stdout();
        match mode {
            TerminalMode::Raw => disable_raw_mode()?,
            TerminalMode::AlternateScreen => execute!(out, LeaveAlternateScreen)?,
            TerminalMode::BracketedPaste => execute!(out, DisableBracketedPaste)?,
            TerminalMode::KeyboardEnhancement => execute!(out, PopKeyboardEnhancementFlags)?,
        }
        ACTIVE_MODES.fetch_and(!mode.bit(), Ordering::Relaxed);
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(io::stdout(), Show)
    }

    fn supports_keyboard_enhancement(&mut self) -> bool {
        keyboard_enhancement_supported()
    }
}

/// Whether the terminal supports the kitty keyboard enhancement protocol.
/// Queried once per process (the query needs raw mode and one terminal roundtrip);
/// re-entering the TUI after agent handovers reuses the cached answer.
fn keyboard_enhancement_supported() -> bool {
    use std::sync::OnceLock;
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        matches!(
            crossterm::terminal::supports_keyboard_enhancement(),
            Ok(true)
        )
    })
}

/// Names the terminal mode in an I/O failure, so a cleanup error reaching the
/// user says which state could not be restored.
fn named(mode: TerminalMode, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", mode.name()))
}

/// Ordered setup and teardown of the terminal modes, tracking exactly which ones
/// are currently on. Every enable records its success immediately, so a failure
/// halfway through setup only undoes what actually took effect.
struct TerminalModes<O: TerminalOps> {
    ops: O,
    raw: bool,
    alternate: bool,
    bracketed_paste: bool,
    keyboard_enhanced: bool,
}

impl<O: TerminalOps> TerminalModes<O> {
    fn new(ops: O) -> Self {
        TerminalModes {
            ops,
            raw: false,
            alternate: false,
            bracketed_paste: false,
            keyboard_enhanced: false,
        }
    }

    /// Rebuilds the flag set from an [`ACTIVE_MODES`] bitmask, for restoring the
    /// terminal from a context that does not own the session (the panic hook).
    fn from_active_mask(ops: O, mask: u8) -> Self {
        let mut modes = TerminalModes::new(ops);
        for mode in [
            TerminalMode::Raw,
            TerminalMode::AlternateScreen,
            TerminalMode::BracketedPaste,
            TerminalMode::KeyboardEnhancement,
        ] {
            *modes.flag(mode) = mask & mode.bit() != 0;
        }
        modes
    }

    fn flag(&mut self, mode: TerminalMode) -> &mut bool {
        match mode {
            TerminalMode::Raw => &mut self.raw,
            TerminalMode::AlternateScreen => &mut self.alternate,
            TerminalMode::BracketedPaste => &mut self.bracketed_paste,
            TerminalMode::KeyboardEnhancement => &mut self.keyboard_enhanced,
        }
    }

    /// Enables one mode unless it is already on, recording success before returning.
    fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
        if *self.flag(mode) {
            return Ok(());
        }
        self.ops.enable(mode).map_err(|err| named(mode, err))?;
        *self.flag(mode) = true;
        Ok(())
    }

    /// Disables one mode, clearing its flag only on success (see [`Self::restore`]).
    /// Returns the failure so the caller can keep going and report the first one.
    fn disable(&mut self, mode: TerminalMode) -> Option<io::Error> {
        if !*self.flag(mode) {
            return None;
        }
        match self.ops.disable(mode) {
            Ok(()) => {
                *self.flag(mode) = false;
                None
            }
            Err(err) => Some(named(mode, err)),
        }
    }

    /// Turns on every mode the TUI needs. Raw mode goes first because the
    /// keyboard-enhancement capability query needs it. On failure the modes
    /// already enabled are turned back off before the error propagates, so a
    /// half-configured terminal is never handed back to the shell.
    ///
    /// Idempotent, which makes it double as the resume path after a handover.
    fn enter(&mut self) -> io::Result<()> {
        let steps = [
            TerminalMode::Raw,
            TerminalMode::AlternateScreen,
            TerminalMode::BracketedPaste,
        ];
        for mode in steps {
            if let Err(err) = self.enable(mode) {
                let _ = self.restore();
                return Err(err);
            }
        }
        // Optional feature: an unsupported or failing terminal keeps legacy input.
        if self.ops.supports_keyboard_enhancement() {
            let _ = self.enable(TerminalMode::KeyboardEnhancement);
        }
        Ok(())
    }

    /// Turns off every mode that is still on, in reverse-dependency order, and
    /// restores cursor visibility.
    ///
    /// Every applicable step is attempted even when an earlier one fails, and the
    /// first error is returned. Only successful steps clear their flag, so a later
    /// retry (an explicit call, or `Drop`) redoes exactly what is still active.
    fn restore(&mut self) -> io::Result<()> {
        let mut first_err: Option<io::Error> = None;
        // Keyboard enhancement and bracketed paste come off first: they must not
        // leak into a child agent CLI or the parent shell.
        for mode in [
            TerminalMode::KeyboardEnhancement,
            TerminalMode::BracketedPaste,
            TerminalMode::AlternateScreen,
        ] {
            if let Some(err) = self.disable(mode) {
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
        // Cursor visibility is restored after leaving the alternate screen: some
        // terminals track it per screen buffer, so showing it first can leave the
        // main screen with a hidden cursor. ratatui hides the cursor while drawing,
        // so this runs unconditionally rather than behind a mode flag.
        if let Err(err) = self.ops.show_cursor() {
            if first_err.is_none() {
                first_err = Some(err);
            }
        }
        if let Some(err) = self.disable(TerminalMode::Raw) {
            if first_err.is_none() {
                first_err = Some(err);
            }
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

/// Owns the TUI terminal handle together with the modes enabled for it, so the
/// terminal is restored on every exit path — ordinary return, `?` propagation, or
/// an unwind panic.
///
/// `Drop` cannot cover `SIGKILL`, `process::abort`, `panic = "abort"`, power loss,
/// or an unhandled terminating signal (`SIGTERM`/`SIGHUP`); `reset` remains the
/// documented manual recovery.
struct TerminalSession {
    terminal: Tui,
    modes: TerminalModes<CrosstermOps>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        let mut modes = TerminalModes::new(CrosstermOps);
        modes.enter()?;
        let terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => terminal,
            Err(err) => {
                // The modes are on but nothing owns them yet: undo them here.
                let _ = modes.restore();
                return Err(err.into());
            }
        };
        Ok(TerminalSession { terminal, modes })
    }

    fn terminal_mut(&mut self) -> &mut Tui {
        &mut self.terminal
    }

    /// Releases the terminal for a child process (or for good).
    fn suspend(&mut self) -> io::Result<()> {
        self.modes.restore()
    }

    /// Re-acquires the terminal after a child process exited and forces a full
    /// redraw. The clear runs only once every required mode is back on.
    fn resume(&mut self) -> Result<()> {
        self.modes.enter()?;
        self.terminal.clear()?;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.modes.restore();
    }
}

/// Restores the terminal from the panic hook, before the panic message is printed.
///
/// The panicking thread does not own the [`TerminalSession`], so the modes are
/// reconstructed from [`ACTIVE_MODES`]. Doing this in the hook rather than relying
/// on `Drop` is what keeps the message readable: unwinding prints it first and
/// only then runs destructors, so a `Drop`-only restore would erase the message
/// together with the alternate screen.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mask = ACTIVE_MODES.load(Ordering::Relaxed);
        if mask != 0 {
            let _ = TerminalModes::from_active_mask(CrosstermOps, mask).restore();
        }
        previous(info);
    }));
}

/// RAII raw-mode guard for the temporary prompts shown while the main
/// [`TerminalSession`] is suspended. Without it, an early return or a panic inside
/// a prompt would leave raw mode enabled on the parent shell.
///
/// Only valid while no session holds raw mode: the guard's `Drop` disables raw
/// mode unconditionally, so nesting it inside a live TUI session would break that
/// session's input.
struct RawModeGuard;

impl RawModeGuard {
    fn enter() -> Option<Self> {
        CrosstermOps
            .enable(TerminalMode::Raw)
            .ok()
            .map(|()| RawModeGuard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = CrosstermOps.disable(TerminalMode::Raw);
    }
}

#[cfg(test)]
mod terminal_lifecycle_tests {
    use super::*;

    /// Recorded terminal operation, used to assert ordering and balance.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Enable(TerminalMode),
        Disable(TerminalMode),
        ShowCursor,
    }

    /// In-memory [`TerminalOps`] with per-mode failure injection. Touches no real
    /// terminal, so these tests are safe under the default threaded test runner.
    struct MockOps {
        calls: Vec<Call>,
        fail_enable: Vec<TerminalMode>,
        fail_disable: Vec<TerminalMode>,
        fail_show_cursor: bool,
        supports_enhancement: bool,
    }

    impl MockOps {
        fn new() -> Self {
            MockOps {
                calls: Vec::new(),
                fail_enable: Vec::new(),
                fail_disable: Vec::new(),
                fail_show_cursor: false,
                supports_enhancement: true,
            }
        }
    }

    fn failure(mode: TerminalMode) -> io::Error {
        io::Error::other(format!("injected failure: {}", mode.name()))
    }

    impl TerminalOps for MockOps {
        fn enable(&mut self, mode: TerminalMode) -> io::Result<()> {
            self.calls.push(Call::Enable(mode));
            if self.fail_enable.contains(&mode) {
                return Err(failure(mode));
            }
            Ok(())
        }

        fn disable(&mut self, mode: TerminalMode) -> io::Result<()> {
            self.calls.push(Call::Disable(mode));
            if self.fail_disable.contains(&mode) {
                return Err(failure(mode));
            }
            Ok(())
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.calls.push(Call::ShowCursor);
            if self.fail_show_cursor {
                return Err(io::Error::other("injected failure: show cursor"));
            }
            Ok(())
        }

        fn supports_keyboard_enhancement(&mut self) -> bool {
            self.supports_enhancement
        }
    }

    fn enabled(modes: &TerminalModes<MockOps>) -> Vec<TerminalMode> {
        [
            (TerminalMode::Raw, modes.raw),
            (TerminalMode::AlternateScreen, modes.alternate),
            (TerminalMode::BracketedPaste, modes.bracketed_paste),
            (TerminalMode::KeyboardEnhancement, modes.keyboard_enhanced),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(mode, _)| mode)
        .collect()
    }

    #[test]
    fn enter_turns_on_raw_first_then_screen_paste_and_keyboard() {
        let mut modes = TerminalModes::new(MockOps::new());
        modes.enter().expect("enter");
        assert_eq!(
            modes.ops.calls,
            vec![
                // Raw mode must precede the keyboard-enhancement query.
                Call::Enable(TerminalMode::Raw),
                Call::Enable(TerminalMode::AlternateScreen),
                Call::Enable(TerminalMode::BracketedPaste),
                Call::Enable(TerminalMode::KeyboardEnhancement),
            ]
        );
        assert_eq!(
            enabled(&modes),
            vec![
                TerminalMode::Raw,
                TerminalMode::AlternateScreen,
                TerminalMode::BracketedPaste,
                TerminalMode::KeyboardEnhancement,
            ]
        );
    }

    #[test]
    fn unsupported_keyboard_enhancement_is_skipped_not_failed() {
        let mut ops = MockOps::new();
        ops.supports_enhancement = false;
        let mut modes = TerminalModes::new(ops);
        modes.enter().expect("enter");
        assert!(!modes.keyboard_enhanced);
        assert!(!modes
            .ops
            .calls
            .contains(&Call::Enable(TerminalMode::KeyboardEnhancement)));
    }

    #[test]
    fn failed_keyboard_enhancement_leaves_the_session_usable() {
        let mut ops = MockOps::new();
        ops.fail_enable = vec![TerminalMode::KeyboardEnhancement];
        let mut modes = TerminalModes::new(ops);
        modes
            .enter()
            .expect("enter must succeed without enhancement");
        assert!(!modes.keyboard_enhanced);
        assert!(modes.raw && modes.alternate && modes.bracketed_paste);
    }

    #[test]
    fn partial_setup_restores_every_mode_already_enabled() {
        let mut ops = MockOps::new();
        ops.fail_enable = vec![TerminalMode::BracketedPaste];
        let mut modes = TerminalModes::new(ops);
        let err = modes.enter().expect_err("setup must fail");
        assert!(err.to_string().contains("bracketed paste"));
        assert!(enabled(&modes).is_empty(), "no mode may stay on");
        assert_eq!(
            modes.ops.calls,
            vec![
                Call::Enable(TerminalMode::Raw),
                Call::Enable(TerminalMode::AlternateScreen),
                Call::Enable(TerminalMode::BracketedPaste),
                // Cleanup of what actually took effect:
                Call::Disable(TerminalMode::AlternateScreen),
                Call::ShowCursor,
                Call::Disable(TerminalMode::Raw),
            ]
        );
    }

    #[test]
    fn cleanup_order_pops_keyboard_and_paste_before_leaving_the_screen() {
        let mut modes = TerminalModes::new(MockOps::new());
        modes.enter().expect("enter");
        modes.ops.calls.clear();
        modes.restore().expect("restore");
        assert_eq!(
            modes.ops.calls,
            vec![
                Call::Disable(TerminalMode::KeyboardEnhancement),
                Call::Disable(TerminalMode::BracketedPaste),
                Call::Disable(TerminalMode::AlternateScreen),
                // Cursor is shown after leaving the alternate screen.
                Call::ShowCursor,
                Call::Disable(TerminalMode::Raw),
            ]
        );
        assert!(enabled(&modes).is_empty());
    }

    #[test]
    fn one_cleanup_failure_does_not_skip_later_steps() {
        let mut ops = MockOps::new();
        ops.fail_disable = vec![TerminalMode::AlternateScreen];
        let mut modes = TerminalModes::new(ops);
        modes.enter().expect("enter");
        modes.ops.calls.clear();

        let err = modes.restore().expect_err("cleanup must report failure");
        assert!(err.to_string().contains("alternate screen"));
        // Raw mode and the cursor are still handled after the failing step.
        assert_eq!(
            modes.ops.calls,
            vec![
                Call::Disable(TerminalMode::KeyboardEnhancement),
                Call::Disable(TerminalMode::BracketedPaste),
                Call::Disable(TerminalMode::AlternateScreen),
                Call::ShowCursor,
                Call::Disable(TerminalMode::Raw),
            ]
        );
        // Only the failed step keeps its flag, so a retry redoes just that one.
        assert_eq!(enabled(&modes), vec![TerminalMode::AlternateScreen]);
    }

    #[test]
    fn cleanup_returns_the_first_error_in_cleanup_order() {
        let mut ops = MockOps::new();
        ops.fail_disable = vec![TerminalMode::BracketedPaste, TerminalMode::Raw];
        ops.fail_show_cursor = true;
        let mut modes = TerminalModes::new(ops);
        modes.enter().expect("enter");

        let err = modes.restore().expect_err("cleanup must report failure");
        assert!(
            err.to_string().contains("bracketed paste"),
            "expected the first failure, got: {err}"
        );
        assert_eq!(
            enabled(&modes),
            vec![TerminalMode::Raw, TerminalMode::BracketedPaste]
        );
    }

    #[test]
    fn a_retry_redoes_only_what_stayed_active() {
        let mut ops = MockOps::new();
        ops.fail_disable = vec![TerminalMode::AlternateScreen];
        let mut modes = TerminalModes::new(ops);
        modes.enter().expect("enter");
        modes.restore().expect_err("first cleanup fails");

        // The retry that `Drop` performs, with the terminal now cooperating.
        modes.ops.fail_disable.clear();
        modes.ops.calls.clear();
        modes.restore().expect("retry succeeds");
        assert_eq!(
            modes.ops.calls,
            vec![
                Call::Disable(TerminalMode::AlternateScreen),
                Call::ShowCursor
            ]
        );
        assert!(enabled(&modes).is_empty());
    }

    #[test]
    fn panic_restore_undoes_exactly_the_modes_recorded_as_active() {
        // What the panic hook does: rebuild the flags from the global mask (the
        // panicking thread does not own the session) and clean up in order.
        let mask = TerminalMode::Raw.bit() | TerminalMode::AlternateScreen.bit();
        let mut modes = TerminalModes::from_active_mask(MockOps::new(), mask);
        assert_eq!(
            enabled(&modes),
            vec![TerminalMode::Raw, TerminalMode::AlternateScreen]
        );
        modes.restore().expect("restore");
        assert_eq!(
            modes.ops.calls,
            vec![
                Call::Disable(TerminalMode::AlternateScreen),
                Call::ShowCursor,
                Call::Disable(TerminalMode::Raw),
            ],
            "modes that were never enabled must not be touched"
        );
        assert!(enabled(&modes).is_empty());
    }

    #[test]
    fn suspend_and_resume_are_balanced_and_idempotent() {
        let mut modes = TerminalModes::new(MockOps::new());
        modes.enter().expect("enter");
        modes.restore().expect("suspend");
        modes.enter().expect("resume");
        modes.restore().expect("final cleanup");

        let enables = modes
            .ops
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Enable(_)))
            .count();
        let disables = modes
            .ops
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Disable(_)))
            .count();
        assert_eq!(enables, 8, "4 modes enabled twice");
        assert_eq!(disables, enables, "every enable has a matching disable");

        // Re-entering an already-entered session issues no duplicate enables.
        modes.enter().expect("enter");
        modes.ops.calls.clear();
        modes.enter().expect("re-enter");
        assert!(modes.ops.calls.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn dir_positional_parses_path_forms() {
        let cli = parse(&["s7s", "."]).expect("parse");
        assert_eq!(cli.dir.as_deref(), Some("."));
        assert!(cli.command.is_none());

        let cli = parse(&["s7s", "../other-project"]).expect("parse");
        assert_eq!(cli.dir.as_deref(), Some("../other-project"));
    }

    #[test]
    fn subcommand_names_win_over_dir_positional() {
        // A folder literally named like a subcommand is unreachable as a bare word;
        // a path form or `--` is the documented escape hatch.
        assert!(matches!(
            parse(&["s7s", "demo"]).expect("parse").command,
            Some(CliCommand::Demo)
        ));
        assert_eq!(
            parse(&["s7s", "./demo"]).expect("parse").dir.as_deref(),
            Some("./demo")
        );
        assert_eq!(
            parse(&["s7s", "--", "demo"]).expect("parse").dir.as_deref(),
            Some("demo")
        );
    }

    #[test]
    fn dir_with_subcommand_is_rejected() {
        // clap accepts this shape, so the explicit check owns the rejection.
        let cli = parse(&["s7s", ".", "demo"]).expect("parse");
        assert!(cli.dir_conflicts_with_subcommand());
        assert!(!parse(&["s7s", "."])
            .expect("parse")
            .dir_conflicts_with_subcommand());
        assert!(!parse(&["s7s", "demo"])
            .expect("parse")
            .dir_conflicts_with_subcommand());
    }

    #[test]
    fn dir_conflicts_with_debug_only_flags_but_not_rebuild_cache() {
        assert!(parse(&["s7s", "--print", "."]).is_err());
        assert!(parse(&["s7s", "--usage-probe", "."]).is_err());
        assert!(parse(&["s7s", "--model-probe", "."]).is_err());
        // `--handoff-samples` takes an optional value, so a trailing `.` is its
        // output directory, not <DIR>; the conflict shows in the other order.
        assert_eq!(
            parse(&["s7s", "--handoff-samples", "."])
                .expect("parse")
                .handoff_samples,
            Some(Some(std::path::PathBuf::from(".")))
        );
        assert!(parse(&["s7s", ".", "--handoff-samples"]).is_err());
        // Rebuilding the index before the dialog opens is a valid combination.
        let cli = parse(&["s7s", "--rebuild-cache", "."]).expect("parse");
        assert!(cli.rebuild_cache);
        assert_eq!(cli.dir.as_deref(), Some("."));
    }

    #[test]
    fn resolve_startup_dir_accepts_existing_relative_dir() {
        // Tests run with the crate root as cwd.
        let resolved = resolve_startup_dir("src").expect("resolve");
        assert!(resolved.is_absolute());
        assert!(resolved.is_dir());
        assert!(resolved.ends_with("src"));
        let dot = resolve_startup_dir(".").expect("resolve");
        assert_eq!(dot, std::fs::canonicalize(".").expect("canonicalize"));
    }

    #[test]
    fn resolve_startup_dir_rejects_missing_path_file_and_empty() {
        assert!(resolve_startup_dir("no-such-folder-xyz").is_err());
        assert!(resolve_startup_dir("Cargo.toml")
            .expect_err("file must be rejected")
            .contains("not a directory"));
        assert!(resolve_startup_dir("   ").is_err());
    }
}
