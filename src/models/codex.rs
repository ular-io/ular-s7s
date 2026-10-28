//! Codex model enumeration (`codex debug models`) and its configured default model.

use super::ModelEntry;
use anyhow::{anyhow, Result};
use std::path::Path;

/// codex: Filters visibility=="list" entries from `codex debug models` JSON catalog.
/// Default model is the top-level `model` key in `<CODEX_HOME>/config.toml`.
pub(super) fn fetch_codex(
    envs: &[(&str, &Path)],
    config_root: &Path,
) -> Result<(Vec<ModelEntry>, Option<String>)> {
    let mut cmd = std::process::Command::new("codex");
    cmd.args(["debug", "models"]);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "codex debug models failed (exit {})",
            out.status.code().unwrap_or(-1)
        ));
    }
    let models = parse_codex_models(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| anyhow!("codex: failed to parse debug models JSON"))?;
    let default_model = codex_default_model(&config_root.join("config.toml"));
    Ok((models, default_model))
}

/// JSON parser for codex model catalog (`{"models":[{slug, display_name, ...}]}`).
pub(super) fn parse_codex_models(json: &str) -> Option<Vec<ModelEntry>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let models = v.get("models")?.as_array()?;
    Some(
        models
            .iter()
            .filter(|m| m.get("visibility").and_then(|s| s.as_str()) == Some("list"))
            .filter_map(|m| {
                let slug = m.get("slug")?.as_str()?;
                let label = m
                    .get("display_name")
                    .and_then(|s| s.as_str())
                    .unwrap_or(slug);
                let note = m
                    .get("description")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default();
                Some(ModelEntry {
                    value: slug.to_string(),
                    label: label.to_string(),
                    note: note.to_string(),
                })
            })
            .collect(),
    )
}

/// Read top-level `model = "..."` value in config.toml. Parses lines before section `[`
/// entries without a toml dependency (excludes prefix-similar keys like `model_reasoning_effort` via `=` validation).
pub(super) fn codex_default_model(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            break;
        }
        if let Some(rest) = t.strip_prefix("model") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                let value = value.trim().trim_matches('"').to_string();
                return (!value.is_empty()).then_some(value);
            }
        }
    }
    None
}
