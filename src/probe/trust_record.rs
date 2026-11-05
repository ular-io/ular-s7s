//! Pre-records folder trust for the dedicated probe folder in each agent CLI's own
//! config, so a probe boots without the trust dialog `trust.rs` otherwise has to
//! answer on screen.
//!
//! Each CLI keeps trust in a file the user also edits by hand: claude
//! `<config dir>/.claude.json` (`projects["<dir>"].hasTrustDialogAccepted`), codex
//! `<CODEX_HOME>/config.toml` (`[projects."<dir>"] trust_level = "trusted"`), agy
//! `~/.gemini/antigravity-cli/settings.json` (`trustedWorkspaces`). Measured on
//! claude 2.1.318, codex 0.166.0 and agy 1.5.1: a record written from outside is
//! honored, and a CLI that is running while it is written keeps it when the CLI
//! saves its own config (each re-reads the file before saving).
//!
//! The record is added only when missing, so a config is written at most once.
//! The file is never re-serialized: the record is spliced in as text, and the
//! result is kept only when it parses to exactly the original plus that record, so
//! key order, formatting, and every other value stay byte-for-byte. Anything
//! unexpected — no config file, an existing record for the folder (even an
//! untrusted one), an unrecognized layout — leaves the file alone and falls back to
//! the on-screen dialog handling.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How often the write restarts when the file changed between reading it and
/// replacing it (the CLI saved its config in that window).
const MAX_ATTEMPTS: usize = 3;

/// Adds the probe folder's trust record to `cmd`'s config when it is missing.
/// `envs` are the profile's injected config-root variables. Best-effort: a failure
/// only means the probe may meet the dialog.
pub(super) fn ensure_probe_trusted(cmd: &str, envs: &[(&str, &Path)], probe_dir: &Path) {
    let Some((config, edit)) = target(cmd, envs) else {
        return;
    };
    let Ok(dir) = fs::canonicalize(probe_dir) else {
        return;
    };
    let Some(dir) = dir.to_str() else {
        return;
    };
    // One s7s-owned lock for every config: probes of several profiles, and of
    // several running s7s instances, start at once and may target the same file.
    let lock = crate::config::config_base_dir().join("trust-records");
    let _ = crate::store_lock::with_store_lock(&lock, || add_record(&config, dir, edit));
}

/// Text edit that returns the config with the trust record added, or `None` when
/// nothing should be written.
type Edit = fn(&str, &str) -> Option<String>;

fn target(cmd: &str, envs: &[(&str, &Path)]) -> Option<(PathBuf, Edit)> {
    let root = |var: &str| {
        envs.iter()
            .find(|(key, _)| *key == var)
            .map(|(_, value)| value.to_path_buf())
            .or_else(|| std::env::var_os(var).map(PathBuf::from))
    };
    let home = dirs::home_dir()?;
    match cmd {
        "claude" => Some((
            root("CLAUDE_CONFIG_DIR")
                .map(|dir| dir.join(".claude.json"))
                .unwrap_or_else(|| home.join(".claude.json")),
            claude_with_trust,
        )),
        "codex" => Some((
            root("CODEX_HOME")
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml"),
            codex_with_trust,
        )),
        "agy" => Some((
            home.join(".gemini/antigravity-cli/settings.json"),
            agy_with_trust,
        )),
        _ => None,
    }
}

fn add_record(config: &Path, dir: &str, edit: Edit) -> Result<()> {
    // Write through a symlinked config (dotfile setups) instead of replacing the link.
    let config = fs::canonicalize(config).context("config file missing")?;
    for _ in 0..MAX_ATTEMPTS {
        let before = stamp(&config)?;
        let text = fs::read_to_string(&config)?;
        let Some(updated) = edit(&text, dir) else {
            return Ok(());
        };
        let mode = fs::metadata(&config)?.permissions();
        let temp = config.with_file_name(format!(
            ".{}.s7s-{}",
            config.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        write_private(&temp, updated.as_bytes())?;
        fs::set_permissions(&temp, mode)?;
        // The CLI saved in the meantime: start over from its version.
        if stamp(&config)? != before {
            let _ = fs::remove_file(&temp);
            continue;
        }
        return fs::rename(&temp, &config).map_err(Into::into);
    }
    Ok(())
}

/// Creates `path` readable only by the owner until the caller copies the config's
/// own mode onto it: `.claude.json` is `0600` and must not be exposed even briefly.
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let _ = fs::remove_file(path);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

fn stamp(path: &Path) -> Result<(SystemTime, u64)> {
    let meta = fs::metadata(path)?;
    Ok((meta.modified()?, meta.len()))
}

/// claude: `projects["<dir>"] = {"hasTrustDialogAccepted": true}`.
fn claude_with_trust(text: &str, dir: &str) -> Option<String> {
    let mut expected: serde_json::Value = serde_json::from_str(text).ok()?;
    let projects = expected.get_mut("projects")?.as_object_mut()?;
    if projects.contains_key(dir) {
        return None;
    }
    projects.insert(
        dir.to_string(),
        serde_json::json!({ "hasTrustDialogAccepted": true }),
    );
    let key = serde_json::to_string(dir).ok()?;
    splice_json(text, "\"projects\"", '{', &expected, |indent| {
        format!("{indent}{key}: {{\n{indent}  \"hasTrustDialogAccepted\": true\n{indent}}}")
    })
}

/// agy: `"<dir>"` added to `trustedWorkspaces`.
fn agy_with_trust(text: &str, dir: &str) -> Option<String> {
    let mut expected: serde_json::Value = serde_json::from_str(text).ok()?;
    let list = expected.get_mut("trustedWorkspaces")?.as_array_mut()?;
    if list.iter().any(|entry| entry.as_str() == Some(dir)) {
        return None;
    }
    list.insert(0, serde_json::Value::String(dir.to_string()));
    let value = serde_json::to_string(dir).ok()?;
    splice_json(text, "\"trustedWorkspaces\"", '[', &expected, |indent| {
        format!("{indent}{value}")
    })
}

/// Inserts `entry(indent)` as the first member of the container that follows
/// `key` and returns the text only if it parses to `expected`. Each occurrence of
/// `key` is tried, so a same-named key nested deeper cannot take the edit.
fn splice_json(
    text: &str,
    key: &str,
    open: char,
    expected: &serde_json::Value,
    entry: impl Fn(&str) -> String,
) -> Option<String> {
    let close = if open == '{' { '}' } else { ']' };
    for (at, _) in text.match_indices(key) {
        let rest = &text[at + key.len()..];
        let Some(after_colon) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(inner) = after_colon.trim_start().strip_prefix(open) else {
            continue;
        };
        let insert_at = text.len() - inner.len();
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = text[line_start..at]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        let member_indent = format!("{indent}  ");
        let candidate = if inner.trim_start().starts_with(close) {
            let close_at = text.len() - inner.trim_start().len();
            format!(
                "{}\n{}\n{indent}{}",
                &text[..insert_at],
                entry(&member_indent),
                &text[close_at..]
            )
        } else {
            format!(
                "{}\n{},{}",
                &text[..insert_at],
                entry(&member_indent),
                &text[insert_at..]
            )
        };
        let parsed: Option<serde_json::Value> = serde_json::from_str(&candidate).ok();
        if parsed.as_ref() == Some(expected) {
            return Some(candidate);
        }
    }
    None
}

/// codex: a `[projects."<dir>"]` table with `trust_level = "trusted"` appended.
fn codex_with_trust(text: &str, dir: &str) -> Option<String> {
    let original: toml::Table = text.parse().ok()?;
    if let Some(projects) = original.get("projects") {
        if projects.as_table()?.contains_key(dir) {
            return None;
        }
    }
    let mut expected = original;
    let mut record = toml::Table::new();
    record.insert("trust_level".into(), toml::Value::String("trusted".into()));
    expected
        .entry("projects")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()?
        .insert(dir.to_string(), toml::Value::Table(record));
    let key = toml::Value::String(dir.to_string()).to_string();
    let separator = match text {
        "" => "",
        t if t.ends_with('\n') => "\n",
        _ => "\n\n",
    };
    let candidate = format!("{text}{separator}[projects.{key}]\ntrust_level = \"trusted\"\n");
    let parsed: toml::Table = candidate.parse().ok()?;
    (parsed == expected).then_some(candidate)
}

#[cfg(test)]
#[path = "trust_record_tests.rs"]
mod tests;
