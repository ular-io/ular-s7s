//! Antigravity model enumeration (`agy models`) and its configured default model.

use super::ModelEntry;
use anyhow::{anyhow, Result};
use std::path::Path;

/// agy: Tab-separated `slug<TAB>display name` rows from `agy models`. Older CLI versions
/// emitted one display name per line, which remains supported. Default model is the top-level
/// `model` key in settings.json.
pub(super) fn fetch_agy(config_root: &Path) -> Result<(Vec<ModelEntry>, Option<String>)> {
    let out = std::process::Command::new("agy").arg("models").output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "agy models failed (exit {})",
            out.status.code().unwrap_or(-1)
        ));
    }
    let models = parse_agy_models(&String::from_utf8_lossy(&out.stdout));
    let default_model = normalize_agy_model_value(
        agy_default_model(&config_root.join("settings.json")),
        &models,
    );
    Ok((models, default_model))
}

/// Parses current `agy models` output (`slug<TAB>display name`) while retaining
/// compatibility with legacy display-name-only output.
pub(super) fn parse_agy_models(text: &str) -> Vec<ModelEntry> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let Some((value, display)) = line.split_once('\t') else {
                let value = line.split_whitespace().collect::<Vec<_>>().join(" ");
                return (!value.is_empty()).then(|| ModelEntry {
                    label: value.clone(),
                    value,
                    note: String::new(),
                });
            };
            let value = value.trim();
            if value.is_empty() {
                return None;
            }
            let display = display.split_whitespace().collect::<Vec<_>>().join(" ");
            let note = if display == value {
                String::new()
            } else {
                display
            };
            Some(ModelEntry {
                value: value.to_string(),
                label: value.to_string(),
                note,
            })
        })
        .collect()
}

/// Read top-level `model` key from agy settings.json.
pub(super) fn agy_default_model(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("model")?.as_str().map(str::to_string)
}

/// Maps a legacy display-name setting to the slug emitted alongside it by current agy.
/// Unknown values stay unchanged so the dialog can block them as missing.
pub(super) fn normalize_agy_model_value(
    configured: Option<String>,
    models: &[ModelEntry],
) -> Option<String> {
    let configured = configured?;
    models
        .iter()
        .find(|model| {
            model.value == configured || (!model.note.is_empty() && model.note == configured)
        })
        .map(|model| model.value.clone())
        .or(Some(configured))
}
