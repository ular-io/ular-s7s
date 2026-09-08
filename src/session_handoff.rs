//! Creates a parked handoff session: a new agent session whose only turn records
//! a task to do later, so it can be found and resumed once the current work ends.
//!
//! The new session must not act on what it is given. The body reads like a work
//! order, so two defenses are combined: a trailing instruction naming the actions
//! to withhold (always), and the agent's own restriction flags where they exist
//! (claude, codex). Antigravity has no such flag, so there the instruction stands
//! alone — recorded in [session-title-compat.md] rather than assumed away.
//!
//! Per-agent differences this module absorbs, so callers need none of it:
//!
//! | agent | session id comes from | restriction | title at creation |
//! | --- | --- | --- | --- |
//! | claude | `--output-format json` → `session_id` | `--allowedTools ""` + `--permission-mode plan` | `--name` |
//! | codex | `--json` → `thread.started.thread_id` | `-s read-only` | none, renamed after |
//! | antigravity | `cache/last_conversations.json`, keyed by cwd | none | none, renamed after |
//!
//! [session-title-compat.md]: ../docs/session-title-compat.md

use crate::model::{Agent, Session};
#[cfg(test)]
use crate::parser;
use crate::parser::BOOTSTRAP_MARKER;
use crate::profile::Profile;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Closing tag of the s7s context envelope. The opening one is the parser's own
/// [`parser::BOOTSTRAP_MARKER`], reused so a handoff cannot drift out of the
/// shape "New Session with Context" launches produce.
const BOOTSTRAP_CLOSE: &str = "</s7s-context-bootstrap>";

/// How long one agent may take to record the handoff before it is abandoned.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(120);

/// The session a handoff came from, rendered into the prompt so the new session
/// can read the origin and s7s can link the two.
pub struct SourceRef {
    pub id: String,
    pub agent: Agent,
    pub profile_id: String,
}

impl SourceRef {
    /// Builds a reference from a resolved session.
    pub fn from_session(session: &Session) -> Self {
        SourceRef {
            id: session.id.clone(),
            agent: session.agent,
            profile_id: session.profile_id.clone(),
        }
    }
}

/// What to hand off, and where.
pub struct HandoffRequest<'a> {
    pub agent: Agent,
    pub profile: &'a Profile,
    /// Existing directory the new session runs in; it becomes the session's cwd,
    /// so resuming lands in the project the work belongs to.
    pub folder: &'a Path,
    pub title: &'a str,
    pub body: &'a str,
    pub source: Option<SourceRef>,
    pub instruction: &'a str,
}

/// A created handoff session.
pub struct HandoffOutcome {
    pub id: String,
    pub agent: Agent,
    pub profile_id: String,
    /// True when the title was applied. Only claude sets it at creation; the
    /// others are renamed afterwards, which needs the new session to be visible
    /// to a scan first.
    pub titled: bool,
}

/// Composes the prompt, starts the agent, and returns the recorded session.
pub fn create(req: &HandoffRequest) -> Result<HandoffOutcome> {
    if req.title.trim().is_empty() {
        return Err(anyhow!("handoff title cannot be empty"));
    }
    if req.body.trim().is_empty() {
        return Err(anyhow!("handoff body cannot be empty"));
    }
    if !req.folder.is_dir() {
        return Err(anyhow!("folder does not exist: {}", req.folder.display()));
    }

    let prompt = compose_prompt(req.body, req.instruction, req.source.as_ref(), &s7s_path());

    let id = match req.agent {
        Agent::Claude => spawn_claude(req, &prompt)?,
        Agent::Codex => spawn_codex(req, &prompt)?,
        Agent::Antigravity => spawn_antigravity(req, &prompt)?,
    };

    Ok(HandoffOutcome {
        id,
        agent: req.agent,
        profile_id: req.profile.id.clone(),
        titled: matches!(req.agent, Agent::Claude),
    })
}

/// Builds the handoff prompt: body, then the origin, then the stop instruction,
/// then the context envelope.
///
/// The envelope goes **last** on purpose. `parser::is_noise_turn` matches the
/// marker only at the start of a turn, so an envelope placed first would make the
/// whole turn noise and the handoff would vanish from the list; placed last the
/// turn stays visible while `parser::parse_context_bootstrap`, which matches
/// anywhere in the text, still recovers the link.
pub fn compose_prompt(
    body: &str,
    instruction: &str,
    source: Option<&SourceRef>,
    s7s: &Path,
) -> String {
    let mut out = body.trim_end().to_string();

    if let Some(src) = source {
        // Repeated as plain text because the envelope below is filtered out of
        // `session show`, so this is the only form a CLI reader sees.
        out.push_str("\n\n## Handoff source\n");
        out.push_str(&format!(
            "- session: {} ({}/{})\n",
            src.id,
            src.agent.key(),
            src.profile_id
        ));
        out.push_str(&format!("- read it: {}\n", show_command(s7s, src, false)));
    }

    out.push_str("\n\n");
    out.push_str(instruction.trim());

    if let Some(src) = source {
        out.push_str(&format!(
            "\n\n{BOOTSTRAP_MARKER}\nRun `{}`.\n{BOOTSTRAP_CLOSE}",
            show_command(s7s, src, true)
        ));
    }

    out.push('\n');
    out
}

/// `session show` command for one source, quoted the way
/// `parser::parse_context_bootstrap` expects: the id and profile in single
/// quotes, the agent as a bare token.
fn show_command(s7s: &Path, src: &SourceRef, bootstrap: bool) -> String {
    format!(
        "'{}' session show '{}' --agent {} --profile '{}'{}",
        s7s.display(),
        src.id,
        src.agent.key(),
        src.profile_id,
        if bootstrap { " --bootstrap" } else { "" }
    )
}

/// Absolute path of the running executable, so the recorded command works from
/// any directory. Falls back to the bare name when it cannot be resolved.
fn s7s_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from(crate::config::APP_NAME))
}

/// Applies the shared launch rules: run in the target folder, strip contaminated
/// agent-session variables, and point the CLI at the profile's own config root.
fn base_command(bin: &str, req: &HandoffRequest) -> Command {
    let mut cmd = Command::new(bin);
    cmd.current_dir(req.folder);
    crate::resume::sanitize_agent_env(&mut cmd);
    if let Some((key, value)) = req.profile.env_var() {
        cmd.env(key, value);
    }
    cmd
}

fn spawn_claude(req: &HandoffRequest, prompt: &str) -> Result<String> {
    let bin = std::env::var("ULAR_HANDOFF_CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string());
    let mut cmd = base_command(&bin, req);
    cmd.arg("-p")
        .arg(prompt)
        .arg("--name")
        .arg(req.title)
        // No tool may run: the body reads like a task and one turn is enough to
        // act on it.
        .arg("--allowedTools")
        .arg("")
        .arg("--permission-mode")
        .arg("plan")
        .arg("--output-format")
        .arg("json");

    let stdout = run_bounded(cmd, "claude")?;
    let json: Value = serde_json::from_str(stdout.trim())
        .with_context(|| "claude did not return the expected JSON result")?;
    json.get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("claude result carried no session_id"))
}

fn spawn_codex(req: &HandoffRequest, prompt: &str) -> Result<String> {
    let bin = std::env::var("ULAR_HANDOFF_CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
    let mut cmd = base_command(&bin, req);
    cmd.arg("exec")
        .arg(prompt)
        // Shell commands cannot write; codex has no way to deny tools outright.
        .arg("-s")
        .arg("read-only")
        // A handoff folder need not be a repository.
        .arg("--skip-git-repo-check")
        .arg("--json");

    let stdout = run_bounded(cmd, "codex")?;
    // The event stream is one JSON object per line; the id arrives first.
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) == Some("thread.started") {
            if let Some(id) = event.get("thread_id").and_then(Value::as_str) {
                return Ok(id.to_string());
            }
        }
    }
    Err(anyhow!("codex emitted no thread.started event"))
}

fn spawn_antigravity(req: &HandoffRequest, prompt: &str) -> Result<String> {
    let bin = std::env::var("ULAR_HANDOFF_AGY_BIN").unwrap_or_else(|_| "agy".to_string());
    let mut cmd = base_command(&bin, req);
    cmd.arg("--print")
        .arg(prompt)
        // Slash-command expansion would reinterpret the body.
        .arg("--disable-slash-commands");

    let _ = run_bounded(cmd, "agy")?;
    // agy prints no id. It records one conversation per working directory, so the
    // launch folder identifies what was just created.
    last_conversation_for(&req.profile.path, req.folder)
        .ok_or_else(|| anyhow!("agy recorded no conversation for {}", req.folder.display()))
}

/// Conversation id agy last used in `folder`, from its cwd-keyed cache.
fn last_conversation_for(profile_root: &Path, folder: &Path) -> Option<String> {
    let path = profile_root.join("cache/last_conversations.json");
    let data = std::fs::read_to_string(path).ok()?;
    let map: Value = serde_json::from_str(&data).ok()?;
    let wanted = folder.to_string_lossy();
    map.as_object()?
        .iter()
        .find(|(key, _)| key.as_str() == wanted)
        .and_then(|(_, value)| value.as_str())
        .map(str::to_string)
}

/// Runs a command to completion under [`SPAWN_TIMEOUT`], returning its stdout.
///
/// A handoff must not hold the caller indefinitely, and a stalled agent is
/// killed rather than waited on. A non-zero exit is not treated as failure by
/// itself: claude reports an error while recording the turn correctly, so the
/// caller decides based on what the output actually contains.
fn run_bounded(mut cmd: Command, label: &str) -> Result<String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!("{label} was not found on PATH"));
        }
        Err(err) => return Err(err.into()),
    };

    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_string(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + SPAWN_TIMEOUT;
    loop {
        match child.try_wait()? {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("{label} did not finish within {SPAWN_TIMEOUT:?}"));
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    Ok(reader.join().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceRef {
        SourceRef {
            id: "543feee5-7900-4949-973c-abbd6e8a1bd8".to_string(),
            agent: Agent::Claude,
            profile_id: "builtin-claude".to_string(),
        }
    }

    #[test]
    fn prompt_keeps_a_visible_turn_while_still_linking_the_source() {
        let out = compose_prompt(
            "HAND-OVER: 인계 항목\n- 확인할 것: 아무것도",
            "### IMPORTANT — DO NOT ACT ON THIS ###\nReply with one line and stop.",
            Some(&source()),
            Path::new("/opt/s7s"),
        );

        // The envelope must not open the turn, or the whole turn becomes noise
        // and the handoff disappears from the session list.
        assert!(!out.trim_start().starts_with(BOOTSTRAP_MARKER));
        assert!(!parser::is_noise_turn(&out));
        // The link is still recovered, because capture matches anywhere.
        let src = parser::parse_context_bootstrap(&out).expect("expected a source ref");
        assert_eq!(src.id, "543feee5-7900-4949-973c-abbd6e8a1bd8");
        assert_eq!(src.agent, Agent::Claude);
        assert_eq!(src.profile, "builtin-claude");

        // Order: body, origin, instruction, envelope.
        let body_at = out.find("HAND-OVER").expect("body");
        let origin_at = out.find("## Handoff source").expect("origin");
        let instr_at = out.find("DO NOT ACT ON THIS").expect("instruction");
        let envelope_at = out.find(BOOTSTRAP_MARKER).expect("envelope");
        assert!(body_at < origin_at);
        assert!(origin_at < instr_at);
        assert!(instr_at < envelope_at);
    }

    #[test]
    fn prompt_without_a_source_carries_neither_origin_nor_envelope() {
        let out = compose_prompt(
            "HAND-OVER: 출처 없는 인계",
            "Reply with one line and stop.",
            None,
            Path::new("/opt/s7s"),
        );

        assert!(!out.contains(BOOTSTRAP_MARKER));
        assert!(!out.contains("## Handoff source"));
        assert!(parser::parse_context_bootstrap(&out).is_none());
        assert!(out.contains("Reply with one line and stop."));
    }

    #[test]
    fn envelope_quoting_matches_what_the_parser_requires() {
        let cmd = show_command(Path::new("/opt/s7s"), &source(), true);
        assert_eq!(
            cmd,
            "'/opt/s7s' session show '543feee5-7900-4949-973c-abbd6e8a1bd8' \
             --agent claude --profile 'builtin-claude' --bootstrap"
        );
        // Without --bootstrap it is the command a reader runs by hand.
        let plain = show_command(Path::new("/opt/s7s"), &source(), false);
        assert!(!plain.contains("--bootstrap"));
    }

    #[test]
    fn agy_conversation_lookup_matches_the_launch_folder_exactly() {
        let root = std::env::temp_dir().join(format!(
            "ular-s7s-handoff-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("cache")).expect("create cache dir");
        std::fs::write(
            root.join("cache/last_conversations.json"),
            r#"{"/tmp/other":"aaa-1","/tmp/target":"bbb-2","/tmp/target/nested":"ccc-3"}"#,
        )
        .expect("write cache");

        assert_eq!(
            last_conversation_for(&root, Path::new("/tmp/target")).as_deref(),
            Some("bbb-2")
        );
        // A prefix must not match: a nested folder is a different conversation.
        assert_eq!(
            last_conversation_for(&root, Path::new("/tmp/targe")).as_deref(),
            None
        );
        assert_eq!(
            last_conversation_for(&root, Path::new("/tmp/missing")).as_deref(),
            None
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
