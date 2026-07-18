//! s7s: TUI for unified searching and resuming of Claude Code / Antigravity CLI / Codex sessions.

mod cache;
mod config;
mod filter;
mod handoff;
mod model;
mod models;
mod normalize;
mod parser;
mod profile;
mod rename;
mod resume;
mod scan;
mod session_cli;
mod session_context;
mod theme;
mod title;
mod ui;
mod usage;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::{
    cursor::MoveTo,
    event::{
        self, Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
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
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use ui::{App, Screen, UiMode};

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// s7s — Search, inspect, and resume AI CLI sessions.
#[derive(Parser)]
#[command(
    name = "s7s",
    version,
    about = "s7s — Search, inspect, and resume AI CLI sessions (TUI when run without a command)",
    after_help = "\
PROFILES: ~/.config/s7s/profiles.json (builtin Claude/Antigravity/Codex + user-defined)
  Claude/Codex profiles support multiple subscriptions via CLAUDE_CONFIG_DIR/CODEX_HOME
CONFIG:   ~/.config/s7s/config.toml overrides command templates ({prompt} token supported
          in new_* templates for contextual launches)
CACHE:    <OS cache dir>/s7s/index.bin — macOS ~/Library/Caches/s7s
          (mtime incremental; rebuild with --rebuild-cache)

Run `s7s session --help` for session query examples."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
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

#[derive(Subcommand)]
enum CliCommand {
    /// Read context from a previous session
    Session(session_cli::SessionArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

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
    // Query agent usage in the background at app startup (shown in the header).
    app.start_usage_fetch();
    // Also update model lists in the background (version gate - keeps cache if CLI version is unchanged).
    app.start_models_fetch(false);

    let mut terminal = init_terminal()?;
    let res = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    res
}

/// Main event loop.
/// Coalesces input events: waits blocks for the first event, but drains any pending
/// remaining events to update the state, then redraws **exactly once**.
/// Rapidly pressing or holding navigation keys won't redraw on every frame, ensuring immediate cursor movement.
fn run_loop(terminal: &mut Tui, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render::draw(f, app))?;

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
                break;
            }
            let updated = app.poll_background();
            if updated || app.background_in_flight() {
                break; // Background update or loading animation frame -> redraw
            }
        }

        // 2) Drain remaining queued events immediately (reflecting state without redrawing).
        while !app.should_quit
            && app.resume_request.is_none()
            && app.new_session_request.is_none()
            && app.login_request.is_none()
            && app.terminal_request.is_none()
            && event::poll(std::time::Duration::from_millis(0))?
        {
            dispatch_event(app, event::read()?);
        }

        // Process resume request: exit TUI -> execute agent -> return to TUI.
        if let Some(idx) = app.resume_request.take() {
            let session = app.sessions[idx].clone();
            handover(terminal, app, &session)?;
        }
        if let Some(req) = app.new_session_request.take() {
            handover_new_session(terminal, app, req)?;
        }
        if let Some(profile_id) = app.login_request.take() {
            handover_login(terminal, app, profile_id)?;
        }
        if let Some(req) = app.terminal_request.take() {
            handover_terminal(terminal, app, req)?;
        }

        if app.should_quit {
            break;
        }
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
                UiMode::QuickCommand => app.on_key_quick(key),
                UiMode::ThemeSelect => app.on_key_theme_select(key),
                UiMode::Help => app.on_key_help(key),
                UiMode::Message => app.on_key_message(key),
            }
        }
        Event::Resize(_, _) => { /* Reflect in next redraw */ }
        _ => {}
    }
}

/// Hands over TUI control to the agent CLI, then returns. Filter state remains intact in App.
fn handover(terminal: &mut Tui, app: &mut App, session: &model::Session) -> Result<()> {
    // 1) Temporarily disable TUI (exit raw and alternate screens).
    restore_terminal(terminal)?;

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

    // 3) Re-initialize TUI and force redraw.
    *terminal = init_terminal()?;
    terminal.clear()?;
    // Reflect new conversations continued during resume: perform incremental rescan to update
    // the target session and re-sort it to the top based on mtime (selection cursor tracks the session).
    app.refresh_sessions();
    drain_pending_input();
    app.begin_quit_grace();
    app.status_msg = Some(format!("Returned from resume: {}", session.folder));
    Ok(())
}

/// Hands over TUI control to the agent CLI to start a new session in the specified folder.
fn handover_new_session(
    terminal: &mut Tui,
    app: &mut App,
    req: ui::NewSessionRequest,
) -> Result<()> {
    restore_terminal(terminal)?;

    let profile = app.profiles.find(&req.profile_id).cloned();
    let Some(profile) = profile else {
        *terminal = init_terminal()?;
        terminal.clear()?;
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

    *terminal = init_terminal()?;
    terminal.clear()?;
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
fn handover_login(terminal: &mut Tui, app: &mut App, profile_id: String) -> Result<()> {
    restore_terminal(terminal)?;

    let profile = app.profiles.find(&profile_id).cloned();
    let Some(profile) = profile else {
        *terminal = init_terminal()?;
        terminal.clear()?;
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

    *terminal = init_terminal()?;
    terminal.clear()?;
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
fn handover_terminal(terminal: &mut Tui, app: &mut App, req: ui::TerminalRequest) -> Result<()> {
    restore_terminal(terminal)?;

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

    *terminal = init_terminal()?;
    terminal.clear()?;
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
    let yes = if enable_raw_mode().is_ok() {
        drain_pending_input();
        let yes = matches!(
            event::read(),
            Ok(Event::Key(k))
                if k.kind == KeyEventKind::Press
                    && matches!(k.code, event::KeyCode::Char('y' | 'Y'))
        );
        let _ = disable_raw_mode();
        yes
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
    if enable_raw_mode().is_ok() {
        drain_pending_input();
        let _ = event::read();
        let _ = disable_raw_mode();
    } else {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).ok();
    }
}

/// Whether keyboard enhancement flags are currently pushed (must be popped before
/// every terminal restoration / agent handover).
static KEYBOARD_ENHANCED: AtomicBool = AtomicBool::new(false);

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

/// Enters Raw/Alt terminal modes and constructs a Terminal handle.
fn init_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // App does not handle mouse events. Capturing mouse events intercepts default terminal text selection,
    // so we only activate the alternate screen.
    execute!(stdout, EnterAlternateScreen)?;
    // Enhanced keyboard protocol (where supported): lets the terminal report
    // Ctrl+Shift+N distinctly from Ctrl+N (legacy encoding sends the same control
    // byte for both). DISAMBIGUATE_ESCAPE_CODES is sufficient and keeps plain
    // printable-key handling unchanged. Unsupported terminals keep legacy input;
    // the Quick Command palette is the functional fallback there.
    if keyboard_enhancement_supported()
        && execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
    {
        KEYBOARD_ENHANCED.store(true, Ordering::Relaxed);
    }
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Disables Raw/Alt modes and restores the normal terminal state.
/// Pops keyboard enhancement flags first so no enhancement leaks into agent
/// handovers or the parent shell after exit.
fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    if KEYBOARD_ENHANCED.swap(false, Ordering::Relaxed) {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
