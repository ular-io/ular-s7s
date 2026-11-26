//! Color themes: built-in palettes, custom theme files, and selection persistence.
//!
//! Every color in the UI is a semantic role on [`Theme`]; `render.rs` never uses
//! literal colors. Built-ins ship 40 palettes (20 dark, 20 light) — see `builtin_themes`.
//! Custom themes are user-edited TOML files in `~/.config/s7s/themes/*.toml`
//! (user-edited file = TOML, app-owned state = JSON — same split as config.toml).
//! The selected theme key is app-owned state persisted to `~/.config/s7s/theme.json`
//! by the theme dialog (Enter), so config.toml stays untouched by the app.

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::path::PathBuf;

/// Key of the theme used when nothing is persisted yet (first run).
pub const DEFAULT_THEME_KEY: &str = "nord";

/// A complete palette of semantic color roles used by the renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Stable identifier persisted in theme.json (kebab-case; file stem for customs).
    pub key: String,
    /// Display name shown in the theme dialog.
    pub name: String,
    /// Dark background palette (drives the dialog tag and pulse fade fallback).
    pub dark: bool,
    /// Loaded from a themes/*.toml file (shown as "Custom" in the dialog).
    pub custom: bool,

    /// App background. `Color::Reset` keeps the terminal's own background.
    pub bg: Color,
    /// Default text.
    pub fg: Color,
    /// Secondary text (labels, hints, footer descriptions).
    pub muted: Color,
    /// Faint text, separators, and unfocused input borders.
    pub dim: Color,
    /// Focus borders, panel titles, bullets, and the filter/status chips.
    pub accent: Color,
    /// Text placed on an accent-colored background.
    pub on_accent: Color,
    /// Focused table row highlight.
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// Row highlight when the table is visible but not focused.
    pub selection_inactive_bg: Color,
    /// Subtle full-row fill for bookmarked sessions; selection takes precedence.
    pub bookmark_bg: Color,
    /// Hotkey labels (`<enter>`, `<ctrl+n>`, ...).
    pub key_hint: Color,
    /// Usage percentage >= 50% (also the logo `7`).
    pub usage_high: Color,
    /// Usage percentage < 50% and "Limit reached".
    pub usage_low: Color,
    /// Focused dialog button.
    pub button_focus_bg: Color,
    pub button_focus_fg: Color,
    /// Unfocused dialog button.
    pub button_bg: Color,
    pub button_fg: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    /// Agent badge colors (CLD / CDX / AGY).
    pub agent_claude: Color,
    pub agent_codex: Color,
    pub agent_antigravity: Color,
    /// Rendered Markdown roles (`md_*`): derived, except Ular Light's tuned set.
    pub md: MarkdownColors,
}

impl Theme {
    /// Replaces the derived Markdown roles with a fixed set.
    fn with_markdown(mut self, md: MarkdownColors) -> Self {
        self.md = md;
        self
    }

    /// Secondary-text style (previous `soft_dim_style`).
    pub fn soft_dim(&self) -> Style {
        Style::default().fg(self.muted)
    }

    /// Hotkey label style.
    pub fn key_style(&self) -> Style {
        Style::default()
            .fg(self.key_hint)
            .add_modifier(Modifier::BOLD)
    }

    /// Base fill style painted under the whole frame (and re-painted under modals
    /// after `Clear`, which resets cells to the terminal default).
    pub fn base_style(&self) -> Style {
        Style::default().bg(self.bg).fg(self.fg)
    }

    /// RGB the background fades toward for pulse animations. `Color::Reset`
    /// backgrounds fall back on the dark flag (terminal default assumption).
    pub fn bg_rgb(&self) -> (u8, u8, u8) {
        match self.bg {
            Color::Reset => {
                if self.dark {
                    (0, 0, 0)
                } else {
                    (255, 255, 255)
                }
            }
            c => color_rgb(c),
        }
    }
}

/// Approximate RGB for named ANSI colors (used by pulse fading only).
pub fn color_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::DarkGray => (100, 100, 100),
        Color::Gray => (170, 170, 170),
        Color::White => (235, 235, 235),
        Color::Cyan => (0, 190, 190),
        Color::Red => (220, 70, 70),
        Color::Green => (80, 200, 120),
        Color::Yellow => (220, 200, 90),
        Color::Blue => (70, 110, 220),
        Color::Magenta => (200, 90, 200),
        _ => (170, 170, 170),
    }
}

/// Black or white, whichever contrasts more against the given background color
/// (used for text on severity-colored fills, which vary per theme).
pub fn contrast_fg(bg: Color) -> Color {
    let (r, g, b) = color_rgb(bg);
    let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    if luma > 140.0 {
        Color::Black
    } else {
        Color::White
    }
}

/// Shorthand used by the built-in palette tables.
const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

macro_rules! builtin_theme {
    ($key:literal, $name:literal, dark: $dark:literal, {
        bg: $bg:literal, fg: $fg:literal, muted: $muted:literal, dim: $dim:literal,
        accent: $accent:literal, on_accent: $on_accent:literal,
        selection_bg: $sel_bg:literal, selection_fg: $sel_fg:literal,
        selection_inactive_bg: $sel_in:literal,
        bookmark_bg: $bookmark:literal,
        key_hint: $keyc:literal, usage_high: $uh:literal, usage_low: $ul:literal,
        button_focus_bg: $bfb:literal, button_focus_fg: $bff:literal,
        button_bg: $bb:literal, button_fg: $bf:literal,
        success: $succ:literal, warning: $warn:literal, error: $err:literal,
        agent_claude: $ac:literal, agent_codex: $ax:literal, agent_antigravity: $aa:literal $(,)?
    }) => {{
        let mut theme = Theme {
            key: String::from($key),
            name: String::from($name),
            dark: $dark,
            custom: false,
            bg: rgb($bg),
            fg: rgb($fg),
            muted: rgb($muted),
            dim: rgb($dim),
            accent: rgb($accent),
            on_accent: rgb($on_accent),
            selection_bg: rgb($sel_bg),
            selection_fg: rgb($sel_fg),
            selection_inactive_bg: rgb($sel_in),
            bookmark_bg: rgb($bookmark),
            key_hint: rgb($keyc),
            usage_high: rgb($uh),
            usage_low: rgb($ul),
            button_focus_bg: rgb($bfb),
            button_focus_fg: rgb($bff),
            button_bg: rgb($bb),
            button_fg: rgb($bf),
            success: rgb($succ),
            warning: rgb($warn),
            error: rgb($err),
            agent_claude: rgb($ac),
            agent_codex: rgb($ax),
            agent_antigravity: rgb($aa),
            md: markdown::UNSET,
        };
        theme.md = MarkdownColors::derive(&theme);
        theme
    }};
}

mod builtin;
mod file;
mod markdown;
mod project;

pub use builtin::builtin_themes;
use file::load_custom_themes_from;
pub use file::{parse_color, themes_dir};
pub use markdown::MarkdownColors;

/// Full theme list for the dialog: built-ins (dark then light) followed by
/// custom themes. Unit tests skip disk access for determinism.
pub fn all_themes() -> Vec<Theme> {
    let mut themes = builtin_themes();
    if !cfg!(test) {
        themes.extend(load_custom_themes_from(&themes_dir()));
    }
    themes.sort_by(|a, b| {
        let a_is_cvd = a.name.starts_with("(CVD) ");
        let b_is_cvd = b.name.starts_with("(CVD) ");
        match (a_is_cvd, b_is_cvd) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });
    themes
}

/// Resolves a theme by key, falling back to the default built-in.
pub fn resolve(key: &str) -> Theme {
    all_themes()
        .into_iter()
        .find(|t| t.key == key)
        .unwrap_or_else(default_theme)
}

pub fn default_theme() -> Theme {
    builtin_themes()
        .into_iter()
        .find(|t| t.key == DEFAULT_THEME_KEY)
        .expect("default theme exists")
}

/// Selection persistence: `~/.config/s7s/theme.json` (`{"name": "<key>"}`).
fn pref_path() -> PathBuf {
    crate::config::config_base_dir().join("theme.json")
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct ThemePref {
    name: String,
}

/// Loads the persisted selection and resolves it (default theme on first run,
/// unknown key, or in unit tests).
pub fn current() -> Theme {
    if cfg!(test) {
        return default_theme();
    }
    let key = std::fs::read_to_string(pref_path())
        .ok()
        .and_then(|data| serde_json::from_str::<ThemePref>(&data).ok())
        .map(|p| p.name);
    match key {
        Some(key) => resolve(&key),
        None => default_theme(),
    }
}

/// Persists the selected theme key (best-effort; no-op in unit tests).
pub fn save_selected(key: &str) {
    if cfg!(test) {
        return;
    }
    let path = pref_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(&ThemePref {
        name: key.to_string(),
    }) {
        let _ = std::fs::write(path, data);
    }
}

#[cfg(test)]
mod tests;
