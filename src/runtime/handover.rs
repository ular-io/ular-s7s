//! Handing the terminal over to an agent CLI (resume, attach, new session, login, terminal command) and back.

use super::*;

/// Hands over TUI control to the agent CLI, then returns. Filter state remains intact in App.
pub(super) fn handover(
    tui: &mut TerminalSession,
    app: &mut App,
    session: &model::Session,
) -> Result<()> {
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

/// Hands the terminal to `claude attach` for a session Claude Code's daemon
/// holds, then restores the TUI and rescans like a resume.
pub(super) fn handover_attach(
    tui: &mut TerminalSession,
    app: &mut App,
    session: &model::Session,
    job: &str,
) -> Result<()> {
    tui.suspend()?;
    let profile = app.profiles.find(&session.profile_id).cloned();
    let preview = resume::preview_attach(job, profile.as_ref());
    print_handover_screen(
        &format!(
            "[{}] attach: {}",
            session.agent.label(),
            session.cwd.to_string_lossy()
        ),
        &preview,
    );
    match resume::run_attach(session, job, profile.as_ref()) {
        Ok(status) => {
            print_returning_notice();
            if !status.success() && !resume::interrupted_by_user(&status) {
                eprintln!(
                    "\n⚠ Attach exited abnormally (exit code: {}).",
                    status.code().unwrap_or(-1)
                );
                eprintln!("  command: {preview}");
                pause_before_return();
            }
        }
        Err(e) => {
            print_returning_notice();
            eprintln!("\n⚠ failed to run attach: {e}");
            eprintln!("  command: {preview}");
            pause_before_return();
        }
    }
    tui.resume()?;
    app.refresh_sessions();
    drain_pending_input();
    app.begin_quit_grace();
    app.status_msg = Some(format!("Returned from attach: {}", session.folder));
    Ok(())
}

/// Hands over TUI control to the agent CLI to start a new session in the specified folder.
pub(super) fn handover_new_session(
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

    // Scratch launch: empty the shared workspace and rewrite its policy files
    // immediately before handing over, so the agent starts on a folder holding
    // nothing but the policy. A failure here would leave an earlier session's
    // files in place, so the launch is abandoned rather than run on stale state.
    if scratch::is_scratch(&req.cwd) {
        if let Err(e) = scratch::prepare(&req.cwd) {
            tui.resume()?;
            app.status_msg = Some(format!("Failed to prepare scratch folder: {e}"));
            return Ok(());
        }
    }

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
pub(super) fn handover_login(
    tui: &mut TerminalSession,
    app: &mut App,
    profile_id: String,
) -> Result<()> {
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
pub(super) fn handover_terminal(
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
pub(super) fn after_terminal_failure(kind: ui::TerminalKind) {
    match kind {
        ui::TerminalKind::Command => pause_before_return(),
        ui::TerminalKind::EditConfig => offer_vim_retry(),
    }
}

/// Asks whether to reopen config.toml with vim after the configured editor failed,
/// and runs it on confirmation. vim is a deliberate fixed fallback: it is present on
/// virtually every system and independent of the (possibly broken) `editor` value.
pub(super) fn offer_vim_retry() {
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
pub(super) fn print_handover_screen(header: &str, command: &str) {
    let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
    println!("▶ {header}\n  {command}\n");
    println!("⏳ Agent is starting or closing — s7s will return automatically. Please wait…\n");
}

/// Clears screen and prints return-in-progress notice immediately after child process exits (visible until TUI re-entry).
pub(super) fn print_returning_notice() {
    let _ = execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0));
    println!("⏳ Agent exited — returning to s7s…");
}

/// Discards pending keyboard inputs in the buffer immediately after returning from the agent.
/// Prevents keys pressed during the wait (such as rapid Ctrl+C) from flooding the TUI as event queue triggers.
pub(super) fn drain_pending_input() {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }
}

/// Wait for any keypress so the user has time to read the last screen (agent execution
/// failures and terminal command completion). Flushes buffered keys first so leftover
/// keystrokes (e.g. rapid Ctrl+C spam) do not satisfy the wait instantly.
pub(super) fn pause_before_return() {
    eprintln!("\nPress any key to return to the TUI...");
    if let Some(_raw) = RawModeGuard::enter() {
        drain_pending_input();
        let _ = event::read();
    } else {
        let mut buf = String::new();
        io::stdin().read_line(&mut buf).ok();
    }
}
