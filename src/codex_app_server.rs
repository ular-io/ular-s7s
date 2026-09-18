//! Minimal `codex app-server` JSON-RPC client, shared by every write s7s makes
//! to a Codex session.
//!
//! Codex keeps one session in several stores at once — the rollout JSONL, the
//! thread row in `state_*.sqlite`, the paginated item history in
//! `thread_history_*.sqlite`, and the daemon catalog under `sqlite/` — and that
//! set grows between CLI versions. A request through the app server is the only
//! path that keeps all of them in step, so s7s asks the app server first and
//! writes the stores itself only as a fallback.
//!
//! A JSON-RPC result is never taken as proof: the caller re-reads the store
//! afterwards, for the same reason a successful exit code is not trusted.

use crate::profile::Profile;
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One budget for the whole exchange: the app server starts a runtime and opens
/// its databases, and no s7s action may hold the UI for longer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// Sends one request to a freshly spawned `codex app-server` and waits for its
/// reply. `profile` decides which config root the server opens (`CODEX_HOME`).
///
/// An error means the request did not complete — no binary, a handshake that
/// stalled, or an error reply — and the caller must fall back to writing the
/// stores directly.
pub(crate) fn call(profile: &Profile, method: &str, params: Value) -> Result<()> {
    #[cfg(test)]
    if std::env::var_os("ULAR_TEST_ENABLE_CODEX_APP_SERVER").is_none() {
        return Err(anyhow!("codex app server is disabled in unit tests"));
    }

    let bin = std::env::var_os("ULAR_CODEX_BIN").unwrap_or_else(|| "codex".into());
    let mut cmd = Command::new(bin);
    cmd.arg("app-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::resume::sanitize_agent_env(&mut cmd);
    if let Some((key, value)) = profile.env_var() {
        cmd.env(key, value);
    }

    let mut child = cmd.spawn().context("spawn codex app-server")?;
    let result = exchange(&mut child, method, params, REPLY_TIMEOUT);
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// Drives the two-request exchange on an already spawned app server:
/// `initialize`, then the caller's method. Replies arrive interleaved with
/// notifications, so the reader keys on the request id.
fn exchange(
    child: &mut std::process::Child,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<()> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("app server stdout unavailable"))?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                return;
            }
        }
    });

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("app server stdin unavailable"))?;

    let await_reply = |id: i64, deadline: Instant| -> Result<()> {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(anyhow!("app server did not answer request {id}"));
            }
            let line = rx
                .recv_timeout(left)
                .map_err(|_| anyhow!("app server did not answer request {id}"))?;
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if msg.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(err) = msg.get("error") {
                return Err(anyhow!("app server rejected request {id}: {err}"));
            }
            return Ok(());
        }
    };

    let deadline = Instant::now() + timeout;

    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": { "name": "s7s", "title": null, "version": env!("CARGO_PKG_VERSION") },
            "capabilities": null,
        }
    });
    writeln!(stdin, "{init}").context("write initialize")?;
    stdin.flush().context("flush initialize")?;
    await_reply(1, deadline)?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": method,
        "params": params,
    });
    writeln!(stdin, "{request}").with_context(|| format!("write {method}"))?;
    stdin.flush().with_context(|| format!("flush {method}"))?;
    await_reply(2, deadline)
}
