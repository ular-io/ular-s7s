//! Process handover: navigate to the selected session's folder and synchronously execute the agent CLI.
//!
//! Temporary suspension/restoration of the TUI (Raw mode) is handled by the caller (main loop).
//! This module purely executes the child process and blocks until it terminates.
//!
//! If a profile is provided, the config root environment variable (`CLAUDE_CONFIG_DIR`/`CODEX_HOME`)
//! is injected as a command prefix to resume under that subscription/account.
//! We prepend it to the command string because `$SHELL -lc` runs login profile scripts first,
//! meaning process env arguments could be overwritten during shell initialization and would not appear in previews.

use crate::config::Config;
use crate::model::{Agent, Session};
use crate::profile::Profile;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Resumes the session. Blocks until the child terminates and returns the exit status.
pub fn run(
    session: &Session,
    cfg: &Config,
    profile: Option<&Profile>,
) -> std::io::Result<ExitStatus> {
    let cwd = session.cwd.to_string_lossy().to_string();
    let template = cfg.resume_template(session.agent);

    // Substitute template tokens (values are wrapped in single quotes to prevent shell injection).
    let cmd = template
        .replace("{id}", &shell_quote(&session.id))
        .replace("{cwd}", &shell_quote(&cwd));
    let cmd = prefix_env(profile, &cmd, true);

    // Execute after navigating to target directory. If cwd is valid, prepend a cd command.
    let full = if cwd.is_empty() {
        cmd
    } else {
        format!("cd {} && {}", shell_quote(&cwd), cmd)
    };

    // Execute via login shell to ensure PATH to claude/codex/etc. is available.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut child = Command::new(shell);
    child.arg("-lc").arg(&full);
    sanitize_agent_env(&mut child);
    run_status(&mut child)
}

/// Starts a new session in the specified directory. Blocks until the child terminates and returns the exit status.
///
/// If `model` is provided, appends `--model '<value>'` to the end of the command (None = Default,
/// falling back to the CLI's own default model). An append approach is used to ensure existing templates
/// in the user's config.toml remain functional.
///
/// If `initial_prompt` is provided (contextual New Session), it is injected after the
/// model flag: a `{prompt}` token in the template is replaced, otherwise the
/// shell-quoted prompt is appended as the final positional argument. `None` keeps
/// the command byte-for-byte identical to the previous behavior.
pub fn run_new(
    agent: Agent,
    cwd: &Path,
    cfg: &Config,
    profile: Option<&Profile>,
    model: Option<&str>,
    initial_prompt: Option<&str>,
) -> std::io::Result<ExitStatus> {
    let cmd = with_model_flag(cfg.new_session_template(agent), model);
    let cmd = with_initial_prompt(agent, &cmd, initial_prompt);
    let cmd = prefix_env(profile, &cmd, true);
    let full = format!("cd {} && {}", shell_quote(&cwd.to_string_lossy()), cmd);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut child = Command::new(shell);
    child.arg("-lc").arg(&full);
    sanitize_agent_env(&mut child);
    run_status(&mut child)
}

/// Runs a user shell command in the given folder (`!` terminal mode). Blocks until the
/// child terminates and returns the exit status.
///
/// Executed via login shell (`$SHELL -lc`) for PATH parity with agent handovers. Aliases and
/// shell functions from interactive rc files are not available (non-interactive shell).
/// Agent env sanitizing is not applied: the command runs with s7s's environment as-is.
/// If `editor` is configured, EDITOR/VISUAL are exported into the shell so editor-spawning
/// commands (e.g. `git commit`) use the configured editor.
pub fn run_terminal(
    cwd: &Path,
    command: &str,
    editor: Option<&str>,
) -> std::io::Result<ExitStatus> {
    let full = format!("cd {} && {}", shell_quote(&cwd.to_string_lossy()), command);
    let full = with_editor_env(&full, editor);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut child = Command::new(shell);
    child.arg("-lc").arg(&full);
    run_status(&mut child)
}

/// Shell command string to be executed for a terminal command (for preview/verification).
pub fn preview_terminal_command(cwd: &Path, command: &str, editor: Option<&str>) -> String {
    with_editor_env(
        &format!("cd {} && {}", cwd.to_string_lossy(), command),
        editor,
    )
}

/// Prepends an `export EDITOR/VISUAL` statement for the configured editor.
///
/// An `export` statement is used instead of a `KEY=v cmd` prefix (the profile-env approach)
/// because a prefix would only apply to the first command of a compound user command, and
/// login profile scripts could overwrite a plain inherited env var (see module doc).
/// Values are always single-quoted (like `with_model_flag`) so previews match execution.
fn with_editor_env(cmd: &str, editor: Option<&str>) -> String {
    match editor.map(str::trim) {
        Some(e) if !e.is_empty() => {
            let value = shell_quote(e);
            format!("export EDITOR={value} VISUAL={value}; {cmd}")
        }
        _ => cmd.to_string(),
    }
}

/// Executes the agent for initial setup (login) in a new config folder.
/// Blocks until the child terminates and returns the exit status.
///
/// Unlike resume/new sessions, this executes the base flagless command from s7s's current folder without `cd`.
/// Since usage queries run from s7s's execution folder, trusting this folder during login allows subsequent
/// usage queries to pass successfully (see docs/profiles.md).
pub fn run_login(profile: &Profile) -> std::io::Result<ExitStatus> {
    let cmd = prefix_env(Some(profile), login_command(profile.agent), true);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut child = Command::new(shell);
    child.arg("-lc").arg(&cmd);
    sanitize_agent_env(&mut child);
    run_status(&mut child)
}

/// Disables SIGINT/SIGQUIT handling in s7s while waiting for the child process (system(3) convention).
///
/// A Ctrl+C SIGINT is delivered to the entire foreground process group (including s7s).
/// While the agent TUI is in raw mode, signals are not generated; however, if a user repeatedly presses Ctrl+C
/// during the 1-3 second window when the agent restores the terminal to cooked mode and cleans up (or between
/// process termination and TUI restoration), s7s could terminate before returning. Since signal ignore statuses
/// are inherited across exec, we restore SIGINT/SIGQUIT default actions in the child via pre_exec.
fn run_status(cmd: &mut Command) -> std::io::Result<ExitStatus> {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            Ok(())
        });
        let old_int = libc::signal(libc::SIGINT, libc::SIG_IGN);
        let old_quit = libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        let status = cmd.status();
        libc::signal(libc::SIGINT, old_int);
        libc::signal(libc::SIGQUIT, old_quit);
        status
    }
}

/// Identifies if termination was caused by a user interrupt (e.g. repeated Ctrl+C).
///
/// If the shell terminates via SIGINT/SIGQUIT, it is flagged as signal termination. If only the agent terminates,
/// the shell returns conventional 128+N exit codes (130/131). In both cases, s7s should return to the TUI
/// quietly without abnormal termination warnings.
pub fn interrupted_by_user(status: &ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    matches!(status.signal(), Some(libc::SIGINT) | Some(libc::SIGQUIT))
        || matches!(status.code(), Some(130) | Some(131))
}

/// Shell command string to be executed for login (for preview/verification).
pub fn preview_login_command(profile: &Profile) -> String {
    prefix_env(Some(profile), login_command(profile.agent), false)
}

/// Base command used for login execution. Templates (new/resume) contain full access flags
/// which are inappropriate for initial setup, so we use raw CLI commands.
fn login_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "claude",
        Agent::Codex => "codex",
        Agent::Antigravity => "agy",
    }
}

/// Strips environment variables injected by Claude Code into child processes and forces transcript persistence.
///
/// When s7s is run inside a Claude Code session (e.g., via `!` or Bash), it inherits `CLAUDE_CODE_CHILD_SESSION=1`.
/// A nested Claude instance inheriting this env variable (verified in 2026-07) assumes it is an automated child
/// session and skips writing the transcript altogether. As a result, the session disappears from the s7s list
/// and `/resume` options. The list of variables to remove aligns with Claude Code's own internal env cleanup routine.
pub(crate) fn sanitize_agent_env(cmd: &mut Command) {
    for key in [
        "CLAUDECODE",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_BRIDGE_SESSION_ID",
        "CLAUDE_BG_AUTH_SNAPSHOT_PATH",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("CLAUDE_CODE_FORCE_SESSION_PERSISTENCE", "1");
}

/// Shell command string to be executed for resume (for preview/verification).
pub fn preview_command(session: &Session, cfg: &Config, profile: Option<&Profile>) -> String {
    let cwd = session.cwd.to_string_lossy().to_string();
    let template = cfg.resume_template(session.agent);
    let cmd = template.replace("{id}", &session.id).replace("{cwd}", &cwd);
    let cmd = prefix_env(profile, &cmd, false);
    if cwd.is_empty() {
        cmd
    } else {
        format!("cd {} && {}", cwd, cmd)
    }
}

/// Shell command string to be executed for starting a new session (for preview/verification).
pub fn preview_new_command(
    agent: Agent,
    cwd: &Path,
    cfg: &Config,
    profile: Option<&Profile>,
    model: Option<&str>,
    initial_prompt: Option<&str>,
) -> String {
    let cmd = with_model_flag(cfg.new_session_template(agent), model);
    let cmd = with_initial_prompt(agent, &cmd, initial_prompt);
    let cmd = prefix_env(profile, &cmd, false);
    format!("cd {} && {}", cwd.to_string_lossy(), cmd)
}

/// Injects the initial prompt into a new-session command.
///
/// Templates may declare an explicit `{prompt}` token (documented in config.toml);
/// it is replaced with the shell-quoted prompt, or with an empty string when no
/// prompt exists so ordinary New Session behavior is preserved. With a `{prompt}`
/// token the template author controls flag placement, so no agent-specific flag
/// is added around the replacement.
///
/// Templates without the token get an agent-specific injection appended after the
/// model flag (verified against installed CLIs 2026-07):
/// - claude (`claude [options] [prompt]`) and codex (`codex [OPTIONS] [PROMPT]`)
///   accept an interactive initial positional prompt;
/// - agy has NO positional prompt — `--prompt-interactive '<value>'` runs the
///   initial prompt interactively and continues the session.
///
/// Quoting is applied identically in previews so preview == execution.
fn with_initial_prompt(agent: Agent, cmd: &str, prompt: Option<&str>) -> String {
    let quoted = prompt.map(shell_quote);
    if cmd.contains("{prompt}") {
        cmd.replace("{prompt}", quoted.as_deref().unwrap_or(""))
            .trim_end()
            .to_string()
    } else {
        match quoted {
            Some(q) => match agent {
                Agent::Claude | Agent::Codex => format!("{cmd} {q}"),
                Agent::Antigravity => format!("{cmd} --prompt-interactive {q}"),
            },
            None => cmd.to_string(),
        }
    }
}

/// Appends `--model '<value>'` to the end of the command if a model is selected.
///
/// Verified that all three CLIs support the `--model` flag (claude=alias/full name,
/// codex=slug, agy=display name). To handle spaces and parentheses safely (e.g., agy display names like
/// "Gemini 3.1 Pro (Low)"), values are always enclosed in single quotes. This is applied identically to previews
/// to match the actual executed command.
fn with_model_flag(cmd: &str, model: Option<&str>) -> String {
    match model.map(str::trim) {
        Some(m) if !m.is_empty() => format!("{cmd} --model {}", shell_quote(m)),
        _ => cmd.to_string(),
    }
}

/// Prepends environmental variable prefixes for the profile's config root (`KEY='<path>' cmd`).
/// Returns the command unmodified if no mapping exists (e.g., Antigravity).
fn prefix_env(profile: Option<&Profile>, cmd: &str, quote: bool) -> String {
    let Some((key, path)) = profile.and_then(|p| p.env_var()) else {
        return cmd.to_string();
    };
    let value = path.to_string_lossy();
    let value = if quote {
        shell_quote(&value)
    } else {
        value.into_owned()
    };
    format!("{}={} {}", key, value, cmd)
}

/// POSIX shell single-quote escaping.
pub(crate) fn shell_quote(s: &str) -> String {
    // Inside single quotes, only ' is special. Escaped via '\'' pattern.
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Agent;
    use std::path::PathBuf;

    fn profile(agent: Agent, path: &str) -> Profile {
        Profile {
            id: "p1".into(),
            agent,
            name: "Test".into(),
            path: PathBuf::from(path),
            oauth_token: None,
            active: true,
            shortcut: None,
            builtin: false,
        }
    }

    fn session(agent: Agent) -> Session {
        Session {
            agent,
            profile_id: "p1".into(),
            id: "abc".into(),
            source_path: None,
            cwd: PathBuf::from("/tmp/demo"),
            folder: "demo".into(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: Vec::new(),
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        }
    }

    #[test]
    fn preview_prefixes_claude_config_dir() {
        let cfg = Config::load();
        let p = profile(Agent::Claude, "/home/u/.claude-team");
        let preview = preview_command(&session(Agent::Claude), &cfg, Some(&p));
        assert!(
            preview.contains("CLAUDE_CONFIG_DIR=/home/u/.claude-team claude"),
            "unexpected preview: {preview}"
        );
        assert!(preview.starts_with("cd /tmp/demo && "));
    }

    #[test]
    fn preview_without_profile_or_env_unchanged() {
        let cfg = Config::load();
        let no_profile = preview_command(&session(Agent::Claude), &cfg, None);
        assert!(!no_profile.contains("CLAUDE_CONFIG_DIR"));

        // Antigravity does not support config directory environment variables; no prefixes added.
        let agy = profile(Agent::Antigravity, "/home/u/agy2");
        let preview = preview_command(&session(Agent::Antigravity), &cfg, Some(&agy));
        assert!(!preview.contains("/home/u/agy2"));
    }

    #[test]
    fn sanitize_agent_env_strips_child_session_and_forces_persistence() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("printf '%s|%s' \"${CLAUDE_CODE_CHILD_SESSION:-unset}\" \"${CLAUDE_CODE_FORCE_SESSION_PERSISTENCE:-unset}\"");
        // Replicate case where s7s is executed inside a Claude session.
        cmd.env("CLAUDE_CODE_CHILD_SESSION", "1");
        sanitize_agent_env(&mut cmd);
        let out = cmd.output().expect("spawn /bin/sh");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "unset|1");
    }

    #[test]
    fn interrupted_by_user_detects_sigint_termination_and_shell_130() {
        use std::os::unix::process::ExitStatusExt;
        // Case where the shell itself exited due to SIGINT/SIGQUIT (wait status lower byte = signal).
        assert!(interrupted_by_user(&ExitStatus::from_raw(libc::SIGINT)));
        assert!(interrupted_by_user(&ExitStatus::from_raw(libc::SIGQUIT)));
        // Case where only the agent exited and the shell returned conventional 128+N status.
        assert!(interrupted_by_user(&ExitStatus::from_raw(130 << 8)));
        assert!(interrupted_by_user(&ExitStatus::from_raw(131 << 8)));
        // Normal exit and general errors are not user interrupts.
        assert!(!interrupted_by_user(&ExitStatus::from_raw(0)));
        assert!(!interrupted_by_user(&ExitStatus::from_raw(1 << 8)));
    }

    #[test]
    fn preview_terminal_command_prepends_cd() {
        assert_eq!(
            preview_terminal_command(Path::new("/tmp/demo"), "git status", None),
            "cd /tmp/demo && git status"
        );
    }

    #[test]
    fn terminal_command_exports_configured_editor() {
        // Always quoted so editor values with spaces stay a single assignment.
        assert_eq!(
            preview_terminal_command(Path::new("/tmp/demo"), "git commit", Some("code -w")),
            "export EDITOR='code -w' VISUAL='code -w'; cd /tmp/demo && git commit"
        );
        // Blank editor values leave the command untouched.
        assert_eq!(with_editor_env("git status", Some("  ")), "git status");
        assert_eq!(with_editor_env("git status", None), "git status");
    }

    #[test]
    fn preview_new_session_prefixes_profile_env() {
        let cfg = Config::load();
        let p = profile(Agent::Codex, "/home/u/.codex-team");
        let preview = preview_new_command(
            Agent::Codex,
            Path::new("/tmp/demo"),
            &cfg,
            Some(&p),
            None,
            None,
        );

        assert_eq!(
            preview,
            "cd /tmp/demo && CODEX_HOME=/home/u/.codex-team codex --yolo"
        );
    }

    #[test]
    fn preview_new_session_appends_model_flag() {
        let cfg = Config::load();
        // claude alias: appended to template tail, appearing after the env prefix.
        let preview = preview_new_command(
            Agent::Claude,
            Path::new("/tmp/demo"),
            &cfg,
            None,
            Some("fable"),
            None,
        );
        assert_eq!(
            preview,
            "cd /tmp/demo && claude --dangerously-skip-permissions --model 'fable'"
        );

        // agy display name: safely wrapped in single quotes despite spaces and parentheses.
        let preview = preview_new_command(
            Agent::Antigravity,
            Path::new("/tmp/demo"),
            &cfg,
            None,
            Some("Gemini 3.1 Pro (Low)"),
            None,
        );
        assert_eq!(
            preview,
            "cd /tmp/demo && agy --dangerously-skip-permissions --model 'Gemini 3.1 Pro (Low)'"
        );

        // Default (None) or empty strings do not append the flag.
        let preview = preview_new_command(
            Agent::Codex,
            Path::new("/tmp/demo"),
            &cfg,
            None,
            Some("  "),
            None,
        );
        assert_eq!(preview, "cd /tmp/demo && codex --yolo");
    }

    #[test]
    fn initial_prompt_appends_shell_quoted_once() {
        let cfg = Config::load();
        let prompt = "<s7s-context-bootstrap>\nRun `s7s session show 'abc' --bootstrap`.\n</s7s-context-bootstrap>";
        let preview = preview_new_command(
            Agent::Claude,
            Path::new("/tmp/demo"),
            &cfg,
            None,
            None,
            Some(prompt),
        );
        // Quoted exactly once, appended after the template tail.
        assert!(preview.starts_with("cd /tmp/demo && claude --dangerously-skip-permissions '"));
        assert!(preview.contains("'\\''abc'\\''"));
        assert!(preview.ends_with("</s7s-context-bootstrap>'"));
    }

    #[test]
    fn initial_prompt_orders_model_flag_before_prompt() {
        let cfg = Config::load();
        let preview = preview_new_command(
            Agent::Codex,
            Path::new("/tmp/demo"),
            &cfg,
            None,
            Some("gpt-5.3-codex"),
            Some("hello"),
        );
        assert_eq!(
            preview,
            "cd /tmp/demo && codex --yolo --model 'gpt-5.3-codex' 'hello'"
        );
    }

    #[test]
    fn prompt_token_in_template_is_replaced_or_cleared() {
        // Explicit token: replaced with the shell-quoted prompt in place
        // (no agent-specific flag — the template author controls placement).
        assert_eq!(
            with_initial_prompt(Agent::Claude, "claude {prompt} --flag", Some("hi there")),
            "claude 'hi there' --flag"
        );
        // Token with no prompt collapses to an empty string (ordinary New Session).
        assert_eq!(
            with_initial_prompt(Agent::Claude, "claude --flag {prompt}", None),
            "claude --flag"
        );
        // No token, no prompt: byte-for-byte identical.
        assert_eq!(
            with_initial_prompt(Agent::Codex, "codex --yolo", None),
            "codex --yolo"
        );
        // Paths and flags in the template survive replacement.
        assert_eq!(
            with_initial_prompt(
                Agent::Antigravity,
                "agy --config '/tmp/a b' {prompt}",
                Some("q")
            ),
            "agy --config '/tmp/a b' 'q'"
        );
    }

    #[test]
    fn antigravity_prompt_uses_prompt_interactive_flag() {
        // agy has no positional prompt: the initial prompt must go through
        // --prompt-interactive (verified against agy --help).
        let cfg = Config::load();
        let preview = preview_new_command(
            Agent::Antigravity,
            Path::new("/tmp/demo"),
            &cfg,
            None,
            None,
            Some("hello"),
        );
        assert_eq!(
            preview,
            "cd /tmp/demo && agy --dangerously-skip-permissions --prompt-interactive 'hello'"
        );
    }
}
