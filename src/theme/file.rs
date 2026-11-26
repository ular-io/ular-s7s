//! Custom theme files (`~/.config/s7s/themes/*.toml`) and color parsing.

use super::{builtin_themes, rgb, Theme, DEFAULT_THEME_KEY};
use ratatui::style::Color;
use serde::Deserialize;
use std::path::PathBuf;

/// Parses a color value from a theme file: `#RRGGBB` / `RRGGBB` hex, ANSI names,
/// or `default`/`reset` for the terminal's own color.
pub fn parse_color(value: &str) -> Option<Color> {
    let v = value.trim().to_ascii_lowercase();
    match v.as_str() {
        "default" | "reset" | "none" => return Some(Color::Reset),
        "black" => return Some(Color::Black),
        "red" => return Some(Color::Red),
        "green" => return Some(Color::Green),
        "yellow" => return Some(Color::Yellow),
        "blue" => return Some(Color::Blue),
        "magenta" => return Some(Color::Magenta),
        "cyan" => return Some(Color::Cyan),
        "gray" | "grey" => return Some(Color::Gray),
        "darkgray" | "darkgrey" => return Some(Color::DarkGray),
        "white" => return Some(Color::White),
        "lightred" => return Some(Color::LightRed),
        "lightgreen" => return Some(Color::LightGreen),
        "lightyellow" => return Some(Color::LightYellow),
        "lightblue" => return Some(Color::LightBlue),
        "lightmagenta" => return Some(Color::LightMagenta),
        "lightcyan" => return Some(Color::LightCyan),
        _ => {}
    }
    let hex = v.strip_prefix('#').unwrap_or(&v);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(rgb(n))
}

/// Custom theme file schema (`~/.config/s7s/themes/<key>.toml`). Every color is
/// optional; missing roles inherit from `base` (a built-in key, default "nord").
#[derive(Debug, Default, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    dark: Option<bool>,
    base: Option<String>,
    #[serde(default)]
    colors: std::collections::HashMap<String, String>,
}

/// Builds a custom theme from file content. `key` is the file stem.
/// Unknown role names and unparsable colors are ignored (inherited from base).
pub(super) fn theme_from_file(key: &str, data: &str) -> Option<Theme> {
    let file: ThemeFile = toml::from_str(data).ok()?;
    let base_key = file.base.as_deref().unwrap_or(DEFAULT_THEME_KEY);
    let mut theme = builtin_themes()
        .into_iter()
        .find(|t| t.key == base_key)
        .unwrap_or_else(|| builtin_themes().remove(0));
    theme.key = key.to_string();
    theme.name = file.name.unwrap_or_else(|| key.to_string());
    theme.custom = true;
    if let Some(dark) = file.dark {
        theme.dark = dark;
    }
    for (role, value) in &file.colors {
        let Some(color) = parse_color(value) else {
            continue;
        };
        let slot = match role.as_str() {
            "bg" => &mut theme.bg,
            "fg" => &mut theme.fg,
            "muted" => &mut theme.muted,
            "dim" => &mut theme.dim,
            "accent" => &mut theme.accent,
            "on_accent" => &mut theme.on_accent,
            "selection_bg" => &mut theme.selection_bg,
            "selection_fg" => &mut theme.selection_fg,
            "selection_inactive_bg" => &mut theme.selection_inactive_bg,
            "bookmark_bg" => &mut theme.bookmark_bg,
            "key_hint" => &mut theme.key_hint,
            "usage_high" => &mut theme.usage_high,
            "usage_low" => &mut theme.usage_low,
            "button_focus_bg" => &mut theme.button_focus_bg,
            "button_focus_fg" => &mut theme.button_focus_fg,
            "button_bg" => &mut theme.button_bg,
            "button_fg" => &mut theme.button_fg,
            "success" => &mut theme.success,
            "warning" => &mut theme.warning,
            "error" => &mut theme.error,
            "agent_claude" => &mut theme.agent_claude,
            "agent_codex" => &mut theme.agent_codex,
            "agent_antigravity" => &mut theme.agent_antigravity,
            _ => continue,
        };
        *slot = color;
    }
    Some(theme)
}

/// Custom theme directory: `~/.config/s7s/themes`.
pub fn themes_dir() -> PathBuf {
    crate::config::config_base_dir().join("themes")
}

/// Loads custom themes from a directory, sorted by display name.
pub(super) fn load_custom_themes_from(dir: &std::path::Path) -> Vec<Theme> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut themes: Vec<Theme> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                return None;
            }
            let key = path.file_stem()?.to_str()?.to_string();
            let data = std::fs::read_to_string(&path).ok()?;
            theme_from_file(&key, &data)
        })
        .collect();
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    themes
}
