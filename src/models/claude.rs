//! Claude model enumeration: scraping the `/model` picker through the shared PTY driver.

use super::ModelEntry;
use crate::probe::pty::{drive_screen, DriveOutcome, CAPTURE_SEPARATOR};
use crate::probe::{claude_logged_in, CLAUDE_READY_MARKERS};
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

/// claude: Spawns the `/model` screen via PTY to parse the list and the current default (✔).
pub(super) fn fetch_claude(envs: &[(&str, &Path)]) -> Result<(Vec<ModelEntry>, Option<String>)> {
    if claude_logged_in(envs) == Some(false) {
        return Err(anyhow!("claude: not logged in"));
    }
    // The dump env name is shared with the usage probe (see docs/models.md).
    match drive_screen(
        "claude",
        "/model",
        CLAUDE_READY_MARKERS,
        &["Select model"],
        &[],
        Duration::from_secs(2),
        Duration::from_millis(800),
        envs,
        Some("ULAR_USAGE_DUMP"),
        Some(claude_picker_has_more_rows),
    )? {
        DriveOutcome::Screen(text) => parse_claude_model_screen(&text)
            .ok_or_else(|| anyhow!("claude: failed to parse /model screen")),
        DriveOutcome::NotLoggedIn => Err(anyhow!("claude: not logged in")),
    }
}

/// Claude's picker reports how many rows do not fit its ten-row viewport. Count distinct row
/// numbers across captured screens, because the overflow indicator remains visible at the end.
pub(super) fn claude_picker_has_more_rows(initial: &str, captured: &str) -> bool {
    let Some(hidden) = initial.lines().find_map(|line| {
        line.split_once("… +")
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .and_then(|count| count.parse::<usize>().ok())
    }) else {
        return false;
    };
    let visible = claude_picker_row_numbers(initial).len();
    let seen = claude_picker_row_numbers(captured).len();
    visible > 0 && seen < visible + hidden
}

fn claude_picker_row_numbers(screen: &str) -> HashSet<usize> {
    screen
        .split_once("Select model")
        .map(|(_, picker)| {
            picker
                .lines()
                .filter_map(|line| claude_model_row(line).map(|row| row.0))
                .collect()
        })
        .unwrap_or_default()
}

fn claude_model_row(line: &str) -> Option<(usize, &str)> {
    let row = line.trim();
    let row = row
        .strip_prefix('❯')
        .or_else(|| row.strip_prefix('↓'))
        .unwrap_or(row)
        .trim_start();
    let (number, rest) = row.split_once(". ")?;
    Some((number.parse().ok()?, rest.trim_start()))
}

/// Parser for the claude `/model` screen. Reads `N. Name  Description` lines after "Select model".
///
/// - `✔` following the name denotes the current default model.
/// - The `Default (recommended)` line is excluded from the list as it overlaps with s7s's own Default entry.
///   If ✔ is on this line, default_model is None (CLI Default).
/// - Value is the `--model` ID derived from the name by `claude_model_value`. An unknown picker
///   row fails the query rather than caching a partial catalog. A stale custom-model row is
///   skipped because it is not one of the CLI's offered models.
/// - Rows collapsing to an already-seen value are folded into the first one (the screen can list
///   a stale `Custom model` row alongside the canonical row), keeping the ✔ if it sits on the
///   duplicate.
pub(super) fn parse_claude_model_screen(text: &str) -> Option<(Vec<ModelEntry>, Option<String>)> {
    let mut models = Vec::new();
    let mut default_model = None;
    let mut seen_header = false;
    let mut unknown_row = false;
    for line in text.lines() {
        let t = line.trim();
        if !seen_header {
            seen_header = t.starts_with("Select model");
            continue;
        }
        let Some((_, rest)) = claude_model_row(t) else {
            continue;
        };
        // Name and description are separated by 2 or more spaces.
        let (name_part, note) = match rest.find("  ") {
            Some(i) => (&rest[..i], rest[i..].trim()),
            None => (rest, ""),
        };
        let is_default = name_part.contains('✔');
        let name = name_part.replace('✔', "").trim().to_string();
        if name.is_empty() {
            continue;
        }
        if name.to_ascii_lowercase().starts_with("default") {
            continue; // CLI's own Default line - keep default_model as None even if checked.
        }
        let Some(value) = claude_model_value(&name) else {
            if note != "Custom model" {
                unknown_row = true;
            }
            continue;
        };
        if is_default {
            default_model = Some(value.clone());
        }
        if models.iter().any(|m: &ModelEntry| m.value == value) {
            continue;
        }
        models.push(ModelEntry {
            value,
            label: name,
            note: note.to_string(),
        });
    }
    let first_screen = text
        .split_once(CAPTURE_SEPARATOR)
        .map_or(text, |(first, _)| first);
    (!models.is_empty() && !unknown_row && !claude_picker_has_more_rows(first_screen, text))
        .then_some((models, default_model))
}

/// `/model` screen row name -> `--model` value.
///
/// Through claude 2.1.207 the row name *was* the alias (`Opus` -> `opus`), so lowercasing was
/// enough. 2.1.220 renders the long-context variant as `Opus (1M context)`, whose lowercase form
/// is not a valid alias — the CLI spells it `opus[1m]` (verified against the 2.1.220 binary and
/// the `model` key `/model` writes to settings.json).
///
/// Versioned picker names must use pinned IDs. A family alias such as `opus` could resolve to a
/// newer version while an older `Opus 5.5` row still appears in the picker. An unknown name
/// makes the query fail rather than caching a partial list (see docs/models.md).
pub(super) fn claude_model_value(name: &str) -> Option<String> {
    const LONG_CONTEXT: &str = "(1m context)";
    let lower = name.to_ascii_lowercase();
    let (base, long_context) = match lower.strip_suffix(LONG_CONTEXT) {
        Some(head) => (head.trim_end(), true),
        None => (lower.as_str(), false),
    };
    let value = match base {
        "opus 5.5" => "claude-opus-5-5",
        "fable 5.1" => "claude-fable-5-1",
        "sonnet 5.5" => "claude-sonnet-5-5",
        "sonnet 5" => "claude-sonnet-5",
        "haiku 4.5" => "claude-haiku-4-5-20251001",
        "opus 5" => "claude-opus-5",
        "fable 5" => "claude-fable-5",
        "opus 4.8" => "claude-opus-4-8",
        "opus 4.7" => "claude-opus-4-7",
        "opus 4.6" => "claude-opus-4-6",
        "sonnet 4.6" => "claude-sonnet-4-6",
        _ if !base.is_empty()
            && base.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
            }) =>
        {
            base
        }
        _ => return None,
    };
    Some(if long_context {
        // The qualifier has a documented alias for the current Opus model. Do not invent
        // `[1m]` forms for pinned IDs or unrelated families.
        if base == "opus" {
            "opus[1m]".to_string()
        } else {
            return None;
        }
    } else {
        value.to_string()
    })
}
