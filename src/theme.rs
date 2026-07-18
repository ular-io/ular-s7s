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
}

impl Theme {
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
        key_hint: $keyc:literal, usage_high: $uh:literal, usage_low: $ul:literal,
        button_focus_bg: $bfb:literal, button_focus_fg: $bff:literal,
        button_bg: $bb:literal, button_fg: $bf:literal,
        success: $succ:literal, warning: $warn:literal, error: $err:literal,
        agent_claude: $ac:literal, agent_codex: $ax:literal, agent_antigravity: $aa:literal $(,)?
    }) => {
        Theme {
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
        }
    };
}

/// Built-in themes in dialog order: the original 10 (6 dark, 4 light), then 10
/// developer-popular palettes (7 dark, 3 light), then 3 additional dark and
/// 9 additional light palettes, then Ular Dark / Ular Light (the project's own
/// palettes), then 6 color-vision-deficiency (CVD) safe palettes (3 dark,
/// 3 light) closing each category list. 40 total (20 dark, 20 light). Standard
/// palettes come from the projects' official specs; the CVD themes are built on
/// the Okabe-Ito (CUD), IBM Carbon, and Paul Tol colorblind-safe color sets,
/// mapping success/error to blue vs orange/vermillion/magenta (never red vs green)
/// so severity stays distinguishable under red-green (and blue-yellow) deficiency.
pub fn builtin_themes() -> Vec<Theme> {
    vec![
        builtin_theme!("nord", "Nord", dark: true, {
            bg: 0x2E3440, fg: 0xD8DEE9, muted: 0x7B88A1, dim: 0x4C566A,
            accent: 0x88C0D0, on_accent: 0x2E3440,
            selection_bg: 0x88C0D0, selection_fg: 0x2E3440,
            selection_inactive_bg: 0x434C5E,
            key_hint: 0x81A1C1, usage_high: 0x81A1C1, usage_low: 0xBF616A,
            button_focus_bg: 0x5E81AC, button_focus_fg: 0xECEFF4,
            button_bg: 0x4C566A, button_fg: 0xD8DEE9,
            success: 0xA3BE8C, warning: 0xEBCB8B, error: 0xBF616A,
            agent_claude: 0xD08770, agent_codex: 0xA3BE8C, agent_antigravity: 0x81A1C1,
        }),
        builtin_theme!("tokyo-night", "Tokyo Night", dark: true, {
            bg: 0x1A1B26, fg: 0xC0CAF5, muted: 0x565F89, dim: 0x3B4261,
            accent: 0x7AA2F7, on_accent: 0x16161E,
            selection_bg: 0x7AA2F7, selection_fg: 0x16161E,
            selection_inactive_bg: 0x292E42,
            key_hint: 0x7DCFFF, usage_high: 0x7AA2F7, usage_low: 0xF7768E,
            button_focus_bg: 0x3D59A1, button_focus_fg: 0xC0CAF5,
            button_bg: 0x414868, button_fg: 0xA9B1D6,
            success: 0x9ECE6A, warning: 0xE0AF68, error: 0xF7768E,
            agent_claude: 0xFF9E64, agent_codex: 0x9ECE6A, agent_antigravity: 0x7DCFFF,
        }),
        builtin_theme!("dracula", "Dracula", dark: true, {
            bg: 0x282A36, fg: 0xF8F8F2, muted: 0x6272A4, dim: 0x44475A,
            accent: 0xBD93F9, on_accent: 0x282A36,
            selection_bg: 0xBD93F9, selection_fg: 0x282A36,
            selection_inactive_bg: 0x44475A,
            key_hint: 0x8BE9FD, usage_high: 0x8BE9FD, usage_low: 0xFF5555,
            button_focus_bg: 0xBD93F9, button_focus_fg: 0x282A36,
            button_bg: 0x44475A, button_fg: 0xF8F8F2,
            success: 0x50FA7B, warning: 0xF1FA8C, error: 0xFF5555,
            agent_claude: 0xFFB86C, agent_codex: 0x50FA7B, agent_antigravity: 0x8BE9FD,
        }),
        builtin_theme!("gruvbox-dark", "Gruvbox Dark", dark: true, {
            bg: 0x282828, fg: 0xEBDBB2, muted: 0xA89984, dim: 0x665C54,
            accent: 0xFE8019, on_accent: 0x282828,
            selection_bg: 0xFE8019, selection_fg: 0x282828,
            selection_inactive_bg: 0x3C3836,
            key_hint: 0x83A598, usage_high: 0x83A598, usage_low: 0xFB4934,
            button_focus_bg: 0x458588, button_focus_fg: 0xFBF1C7,
            button_bg: 0x504945, button_fg: 0xBDAE93,
            success: 0xB8BB26, warning: 0xFABD2F, error: 0xFB4934,
            agent_claude: 0xFE8019, agent_codex: 0xB8BB26, agent_antigravity: 0x83A598,
        }),
        builtin_theme!("solarized-dark", "Solarized Dark", dark: true, {
            bg: 0x002B36, fg: 0x839496, muted: 0x657B83, dim: 0x586E75,
            accent: 0x268BD2, on_accent: 0xFDF6E3,
            selection_bg: 0x268BD2, selection_fg: 0xFDF6E3,
            selection_inactive_bg: 0x073642,
            key_hint: 0x2AA198, usage_high: 0x268BD2, usage_low: 0xDC322F,
            button_focus_bg: 0x268BD2, button_focus_fg: 0xFDF6E3,
            button_bg: 0x073642, button_fg: 0x93A1A1,
            success: 0x859900, warning: 0xB58900, error: 0xDC322F,
            agent_claude: 0xCB4B16, agent_codex: 0x859900, agent_antigravity: 0x268BD2,
        }),
        builtin_theme!("catppuccin-mocha", "Catppuccin Mocha", dark: true, {
            bg: 0x1E1E2E, fg: 0xCDD6F4, muted: 0x7F849C, dim: 0x45475A,
            accent: 0xCBA6F7, on_accent: 0x1E1E2E,
            selection_bg: 0xCBA6F7, selection_fg: 0x1E1E2E,
            selection_inactive_bg: 0x313244,
            key_hint: 0x89B4FA, usage_high: 0x89B4FA, usage_low: 0xF38BA8,
            button_focus_bg: 0x89B4FA, button_focus_fg: 0x1E1E2E,
            button_bg: 0x45475A, button_fg: 0xBAC2DE,
            success: 0xA6E3A1, warning: 0xF9E2AF, error: 0xF38BA8,
            agent_claude: 0xFAB387, agent_codex: 0xA6E3A1, agent_antigravity: 0x89DCEB,
        }),
        builtin_theme!("github-light", "GitHub Light", dark: false, {
            bg: 0xFFFFFF, fg: 0x1F2328, muted: 0x59636E, dim: 0xD1D9E0,
            accent: 0x0969DA, on_accent: 0xFFFFFF,
            selection_bg: 0x0969DA, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xD0D7DE,
            key_hint: 0x8250DF, usage_high: 0x0969DA, usage_low: 0xCF222E,
            button_focus_bg: 0x0969DA, button_focus_fg: 0xFFFFFF,
            button_bg: 0xEAEEF2, button_fg: 0x59636E,
            success: 0x1A7F37, warning: 0x9A6700, error: 0xCF222E,
            agent_claude: 0xBC4C00, agent_codex: 0x1A7F37, agent_antigravity: 0x0969DA,
        }),
        builtin_theme!("solarized-light", "Solarized Light", dark: false, {
            bg: 0xFDF6E3, fg: 0x586E75, muted: 0x839496, dim: 0x93A1A1,
            accent: 0x268BD2, on_accent: 0xFDF6E3,
            selection_bg: 0x268BD2, selection_fg: 0xFDF6E3,
            selection_inactive_bg: 0xEEE8D5,
            key_hint: 0x2AA198, usage_high: 0x268BD2, usage_low: 0xDC322F,
            button_focus_bg: 0x268BD2, button_focus_fg: 0xFDF6E3,
            button_bg: 0xEEE8D5, button_fg: 0x839496,
            success: 0x859900, warning: 0xB58900, error: 0xDC322F,
            agent_claude: 0xCB4B16, agent_codex: 0x859900, agent_antigravity: 0x268BD2,
        }),
        builtin_theme!("gruvbox-light", "Gruvbox Light", dark: false, {
            bg: 0xFBF1C7, fg: 0x3C3836, muted: 0x7C6F64, dim: 0xBDAE93,
            accent: 0xAF3A03, on_accent: 0xFBF1C7,
            selection_bg: 0xD65D0E, selection_fg: 0xFBF1C7,
            selection_inactive_bg: 0xEBDBB2,
            key_hint: 0x076678, usage_high: 0x076678, usage_low: 0x9D0006,
            button_focus_bg: 0x076678, button_focus_fg: 0xFBF1C7,
            button_bg: 0xEBDBB2, button_fg: 0x7C6F64,
            success: 0x79740E, warning: 0xB57614, error: 0x9D0006,
            agent_claude: 0xAF3A03, agent_codex: 0x79740E, agent_antigravity: 0x076678,
        }),
        builtin_theme!("catppuccin-latte", "Catppuccin Latte", dark: false, {
            bg: 0xEFF1F5, fg: 0x4C4F69, muted: 0x8C8FA1, dim: 0xBCC0CC,
            accent: 0x8839EF, on_accent: 0xEFF1F5,
            selection_bg: 0x8839EF, selection_fg: 0xEFF1F5,
            selection_inactive_bg: 0xCCD0DA,
            key_hint: 0x1E66F5, usage_high: 0x1E66F5, usage_low: 0xD20F39,
            button_focus_bg: 0x1E66F5, button_focus_fg: 0xEFF1F5,
            button_bg: 0xCCD0DA, button_fg: 0x5C5F77,
            success: 0x40A02B, warning: 0xDF8E1D, error: 0xD20F39,
            agent_claude: 0xFE640B, agent_codex: 0x40A02B, agent_antigravity: 0x04A5E5,
        }),
        // --- Developer-popular themes (7 dark, 3 light) ---
        builtin_theme!("monokai", "Monokai", dark: true, {
            bg: 0x272822, fg: 0xF8F8F2, muted: 0x908E80, dim: 0x5B5A4E,
            accent: 0x66D9EF, on_accent: 0x272822,
            selection_bg: 0x66D9EF, selection_fg: 0x272822,
            selection_inactive_bg: 0x3E3D32,
            key_hint: 0xA6E22E, usage_high: 0x66D9EF, usage_low: 0xF92672,
            button_focus_bg: 0x66D9EF, button_focus_fg: 0x272822,
            button_bg: 0x3E3D32, button_fg: 0xF8F8F2,
            success: 0xA6E22E, warning: 0xE6DB74, error: 0xF92672,
            agent_claude: 0xFD971F, agent_codex: 0xA6E22E, agent_antigravity: 0x66D9EF,
        }),
        builtin_theme!("one-dark", "One Dark", dark: true, {
            bg: 0x282C34, fg: 0xABB2BF, muted: 0x828997, dim: 0x5C6370,
            accent: 0x61AFEF, on_accent: 0x282C34,
            selection_bg: 0x61AFEF, selection_fg: 0x282C34,
            selection_inactive_bg: 0x3E4451,
            key_hint: 0x56B6C2, usage_high: 0x61AFEF, usage_low: 0xE06C75,
            button_focus_bg: 0x61AFEF, button_focus_fg: 0x282C34,
            button_bg: 0x3E4451, button_fg: 0xABB2BF,
            success: 0x98C379, warning: 0xE5C07B, error: 0xE06C75,
            agent_claude: 0xD19A66, agent_codex: 0x98C379, agent_antigravity: 0x56B6C2,
        }),
        builtin_theme!("night-owl", "Night Owl", dark: true, {
            bg: 0x011627, fg: 0xD6DEEB, muted: 0x7E8C9A, dim: 0x5F7E97,
            accent: 0x82AAFF, on_accent: 0x011627,
            selection_bg: 0x82AAFF, selection_fg: 0x011627,
            selection_inactive_bg: 0x1D3B53,
            key_hint: 0x7FDBCA, usage_high: 0x82AAFF, usage_low: 0xFF5874,
            button_focus_bg: 0x82AAFF, button_focus_fg: 0x011627,
            button_bg: 0x1D3B53, button_fg: 0xD6DEEB,
            success: 0xADDB67, warning: 0xECC48D, error: 0xFF5874,
            agent_claude: 0xF78C6C, agent_codex: 0xADDB67, agent_antigravity: 0x7FDBCA,
        }),
        builtin_theme!("ayu-dark", "Ayu Dark", dark: true, {
            bg: 0x0D1017, fg: 0xBFBDB6, muted: 0x7A7F87, dim: 0x565B66,
            accent: 0xE6B450, on_accent: 0x0D1017,
            selection_bg: 0xE6B450, selection_fg: 0x0D1017,
            selection_inactive_bg: 0x1E232B,
            key_hint: 0x59C2FF, usage_high: 0x59C2FF, usage_low: 0xF07178,
            button_focus_bg: 0xE6B450, button_focus_fg: 0x0D1017,
            button_bg: 0x1E232B, button_fg: 0xBFBDB6,
            success: 0xAAD94C, warning: 0xFFB454, error: 0xF07178,
            agent_claude: 0xFF8F40, agent_codex: 0xAAD94C, agent_antigravity: 0x59C2FF,
        }),
        builtin_theme!("everforest-dark", "Everforest Dark", dark: true, {
            bg: 0x2D353B, fg: 0xD3C6AA, muted: 0x9DA9A0, dim: 0x7A8478,
            accent: 0xA7C080, on_accent: 0x2D353B,
            selection_bg: 0xA7C080, selection_fg: 0x2D353B,
            selection_inactive_bg: 0x3D484D,
            key_hint: 0x7FBBB3, usage_high: 0x7FBBB3, usage_low: 0xE67E80,
            button_focus_bg: 0x83C092, button_focus_fg: 0x2D353B,
            button_bg: 0x3D484D, button_fg: 0xD3C6AA,
            success: 0xA7C080, warning: 0xDBBC7F, error: 0xE67E80,
            agent_claude: 0xE69875, agent_codex: 0xA7C080, agent_antigravity: 0x7FBBB3,
        }),
        builtin_theme!("rose-pine", "Rosé Pine", dark: true, {
            bg: 0x191724, fg: 0xE0DEF4, muted: 0x908CAA, dim: 0x6E6A86,
            accent: 0xC4A7E7, on_accent: 0x191724,
            selection_bg: 0xC4A7E7, selection_fg: 0x191724,
            selection_inactive_bg: 0x26233A,
            key_hint: 0x9CCFD8, usage_high: 0x9CCFD8, usage_low: 0xEB6F92,
            button_focus_bg: 0x31748F, button_focus_fg: 0xE0DEF4,
            button_bg: 0x403D52, button_fg: 0xE0DEF4,
            success: 0x9CCFD8, warning: 0xF6C177, error: 0xEB6F92,
            agent_claude: 0xEBBCBA, agent_codex: 0x9CCFD8, agent_antigravity: 0xC4A7E7,
        }),
        builtin_theme!("kanagawa", "Kanagawa", dark: true, {
            bg: 0x1F1F28, fg: 0xDCD7BA, muted: 0xA6A28C, dim: 0x54546D,
            accent: 0x7E9CD8, on_accent: 0x1F1F28,
            selection_bg: 0x7E9CD8, selection_fg: 0x1F1F28,
            selection_inactive_bg: 0x2D4F67,
            key_hint: 0x7FB4CA, usage_high: 0x7E9CD8, usage_low: 0xE46876,
            button_focus_bg: 0x2D4F67, button_focus_fg: 0xDCD7BA,
            button_bg: 0x363646, button_fg: 0xC8C093,
            success: 0x98BB6C, warning: 0xE6C384, error: 0xE46876,
            agent_claude: 0xFFA066, agent_codex: 0x98BB6C, agent_antigravity: 0x7FB4CA,
        }),
        builtin_theme!("one-light", "One Light", dark: false, {
            bg: 0xFAFAFA, fg: 0x383A42, muted: 0x696C77, dim: 0xC9CACE,
            accent: 0x4078F2, on_accent: 0xFAFAFA,
            selection_bg: 0x4078F2, selection_fg: 0xFAFAFA,
            selection_inactive_bg: 0xDBDBDC,
            key_hint: 0xA626A4, usage_high: 0x4078F2, usage_low: 0xCA1243,
            button_focus_bg: 0x4078F2, button_focus_fg: 0xFAFAFA,
            button_bg: 0xE5E5E6, button_fg: 0x696C77,
            success: 0x50A14F, warning: 0x986801, error: 0xCA1243,
            agent_claude: 0xC18401, agent_codex: 0x50A14F, agent_antigravity: 0x0184BC,
        }),
        builtin_theme!("ayu-light", "Ayu Light", dark: false, {
            bg: 0xFCFCFC, fg: 0x5C6166, muted: 0x8A8F94, dim: 0xD5D6D8,
            accent: 0x399EE6, on_accent: 0xFCFCFC,
            selection_bg: 0x399EE6, selection_fg: 0xFCFCFC,
            selection_inactive_bg: 0xE7E8E9,
            key_hint: 0xA37ACC, usage_high: 0x399EE6, usage_low: 0xE65050,
            button_focus_bg: 0x399EE6, button_focus_fg: 0xFCFCFC,
            button_bg: 0xEDEEEF, button_fg: 0x8A8F94,
            success: 0x86B300, warning: 0xF2AE49, error: 0xE65050,
            agent_claude: 0xFA8D3E, agent_codex: 0x86B300, agent_antigravity: 0x399EE6,
        }),
        builtin_theme!("everforest-light", "Everforest Light", dark: false, {
            bg: 0xFDF6E3, fg: 0x5C6A72, muted: 0x829181, dim: 0xBEC5B2,
            accent: 0x8DA101, on_accent: 0xFDF6E3,
            selection_bg: 0x8DA101, selection_fg: 0xFDF6E3,
            selection_inactive_bg: 0xEAEDC8,
            key_hint: 0x3A94C5, usage_high: 0x3A94C5, usage_low: 0xF85552,
            button_focus_bg: 0x35A77C, button_focus_fg: 0xFDF6E3,
            button_bg: 0xEAEDC8, button_fg: 0x829181,
            success: 0x8DA101, warning: 0xDFA000, error: 0xF85552,
            agent_claude: 0xF57D26, agent_codex: 0x8DA101, agent_antigravity: 0x3A94C5,
        }),
        // --- Additional dark themes (3): official dark siblings of shipped
        // light themes (GitHub Dark, Flexoki Dark, Tomorrow Night). ---
        builtin_theme!("github-dark", "GitHub Dark", dark: true, {
            bg: 0x0D1117, fg: 0xE6EDF3, muted: 0x8B949E, dim: 0x30363D,
            accent: 0x58A6FF, on_accent: 0x0D1117,
            selection_bg: 0x58A6FF, selection_fg: 0x0D1117,
            selection_inactive_bg: 0x21262D,
            key_hint: 0xBC8CFF, usage_high: 0x58A6FF, usage_low: 0xF85149,
            button_focus_bg: 0x1F6FEB, button_focus_fg: 0xFFFFFF,
            button_bg: 0x21262D, button_fg: 0x8B949E,
            success: 0x3FB950, warning: 0xD29922, error: 0xF85149,
            agent_claude: 0xDB6D28, agent_codex: 0x3FB950, agent_antigravity: 0x58A6FF,
        }),
        builtin_theme!("flexoki-dark", "Flexoki Dark", dark: true, {
            bg: 0x100F0F, fg: 0xCECDC3, muted: 0x878580, dim: 0x403E3C,
            accent: 0x4385BE, on_accent: 0x100F0F,
            selection_bg: 0x4385BE, selection_fg: 0x100F0F,
            selection_inactive_bg: 0x282726,
            key_hint: 0x8B7EC8, usage_high: 0x4385BE, usage_low: 0xD14D41,
            button_focus_bg: 0x4385BE, button_focus_fg: 0x100F0F,
            button_bg: 0x343331, button_fg: 0x878580,
            success: 0x879A39, warning: 0xD0A215, error: 0xD14D41,
            agent_claude: 0xDA702C, agent_codex: 0x879A39, agent_antigravity: 0x3AA99F,
        }),
        builtin_theme!("tomorrow-night", "Tomorrow Night", dark: true, {
            bg: 0x1D1F21, fg: 0xC5C8C6, muted: 0x969896, dim: 0x373B41,
            accent: 0x81A2BE, on_accent: 0x1D1F21,
            selection_bg: 0x81A2BE, selection_fg: 0x1D1F21,
            selection_inactive_bg: 0x282A2E,
            key_hint: 0xB294BB, usage_high: 0x81A2BE, usage_low: 0xCC6666,
            button_focus_bg: 0x81A2BE, button_focus_fg: 0x1D1F21,
            button_bg: 0x373B41, button_fg: 0xC5C8C6,
            success: 0xB5BD68, warning: 0xF0C674, error: 0xCC6666,
            agent_claude: 0xDE935F, agent_codex: 0xB5BD68, agent_antigravity: 0x8ABEB7,
        }),
        // --- Additional light themes (9): official light siblings of shipped
        // dark themes (Tokyo Night Day, Rosé Pine Dawn, Kanagawa Lotus, Night
        // Owl Light) plus popular standalone light palettes. ---
        builtin_theme!("tokyo-night-day", "Tokyo Night Day", dark: false, {
            bg: 0xE1E2E7, fg: 0x3760BF, muted: 0x848CB5, dim: 0xA8AECB,
            accent: 0x2E7DE9, on_accent: 0xE1E2E7,
            selection_bg: 0x2E7DE9, selection_fg: 0xE1E2E7,
            selection_inactive_bg: 0xC4C8DA,
            key_hint: 0x007197, usage_high: 0x2E7DE9, usage_low: 0xF52A65,
            button_focus_bg: 0x2E7DE9, button_focus_fg: 0xE1E2E7,
            button_bg: 0xC4C8DA, button_fg: 0x6172B0,
            success: 0x587539, warning: 0x8C6C3E, error: 0xF52A65,
            agent_claude: 0xB15C00, agent_codex: 0x587539, agent_antigravity: 0x007197,
        }),
        builtin_theme!("rose-pine-dawn", "Rosé Pine Dawn", dark: false, {
            bg: 0xFAF4ED, fg: 0x575279, muted: 0x797593, dim: 0xCECACD,
            accent: 0x907AA9, on_accent: 0xFAF4ED,
            selection_bg: 0x907AA9, selection_fg: 0xFAF4ED,
            selection_inactive_bg: 0xDFDAD9,
            key_hint: 0x56949F, usage_high: 0x56949F, usage_low: 0xB4637A,
            button_focus_bg: 0x286983, button_focus_fg: 0xFAF4ED,
            button_bg: 0xF2E9E1, button_fg: 0x797593,
            success: 0x56949F, warning: 0xEA9D34, error: 0xB4637A,
            agent_claude: 0xD7827E, agent_codex: 0x56949F, agent_antigravity: 0x907AA9,
        }),
        builtin_theme!("kanagawa-lotus", "Kanagawa Lotus", dark: false, {
            bg: 0xF2ECBC, fg: 0x545464, muted: 0x8A8980, dim: 0xD5CEA3,
            accent: 0x4D699B, on_accent: 0xF2ECBC,
            selection_bg: 0x4D699B, selection_fg: 0xF2ECBC,
            selection_inactive_bg: 0xE7DBA0,
            key_hint: 0x4E8CA2, usage_high: 0x4D699B, usage_low: 0xC84053,
            button_focus_bg: 0x4D699B, button_focus_fg: 0xF2ECBC,
            button_bg: 0xE4D794, button_fg: 0x8A8980,
            success: 0x6F894E, warning: 0xCC6D00, error: 0xC84053,
            agent_claude: 0xE98A00, agent_codex: 0x6F894E, agent_antigravity: 0x4E8CA2,
        }),
        builtin_theme!("night-owl-light", "Night Owl Light", dark: false, {
            bg: 0xFBFBFB, fg: 0x403F53, muted: 0x989FB1, dim: 0xD0D0D0,
            accent: 0x288ED7, on_accent: 0xFBFBFB,
            selection_bg: 0x288ED7, selection_fg: 0xFBFBFB,
            selection_inactive_bg: 0xE0E0E0,
            key_hint: 0x2AA298, usage_high: 0x288ED7, usage_low: 0xDE3D3B,
            button_focus_bg: 0x288ED7, button_focus_fg: 0xFBFBFB,
            button_bg: 0xE8E8E8, button_fg: 0x989FB1,
            success: 0x08916A, warning: 0xE0AF02, error: 0xDE3D3B,
            agent_claude: 0xD6438A, agent_codex: 0x08916A, agent_antigravity: 0x2AA298,
        }),
        builtin_theme!("flexoki-light", "Flexoki Light", dark: false, {
            bg: 0xFFFCF0, fg: 0x100F0F, muted: 0x6F6E69, dim: 0xCECDC3,
            accent: 0x205EA6, on_accent: 0xFFFCF0,
            selection_bg: 0x205EA6, selection_fg: 0xFFFCF0,
            selection_inactive_bg: 0xE6E4D9,
            key_hint: 0x5E409D, usage_high: 0x205EA6, usage_low: 0xAF3029,
            button_focus_bg: 0x205EA6, button_focus_fg: 0xFFFCF0,
            button_bg: 0xE6E4D9, button_fg: 0x6F6E69,
            success: 0x66800B, warning: 0xAD8301, error: 0xAF3029,
            agent_claude: 0xBC5215, agent_codex: 0x66800B, agent_antigravity: 0x24837B,
        }),
        builtin_theme!("selenized-light", "Selenized Light", dark: false, {
            bg: 0xFBF3DB, fg: 0x53676D, muted: 0x909995, dim: 0xD5CDB6,
            accent: 0x0072D4, on_accent: 0xFBF3DB,
            selection_bg: 0x0072D4, selection_fg: 0xFBF3DB,
            selection_inactive_bg: 0xECE3CC,
            key_hint: 0x009C8F, usage_high: 0x0072D4, usage_low: 0xD2212D,
            button_focus_bg: 0x0072D4, button_focus_fg: 0xFBF3DB,
            button_bg: 0xECE3CC, button_fg: 0x909995,
            success: 0x489100, warning: 0xAD8900, error: 0xD2212D,
            agent_claude: 0xC25D1E, agent_codex: 0x489100, agent_antigravity: 0x0072D4,
        }),
        builtin_theme!("papercolor-light", "PaperColor Light", dark: false, {
            bg: 0xEEEEEE, fg: 0x444444, muted: 0x6C6C6C, dim: 0xD0D0D0,
            accent: 0x005F87, on_accent: 0xEEEEEE,
            selection_bg: 0x005F87, selection_fg: 0xEEEEEE,
            selection_inactive_bg: 0xDADADA,
            key_hint: 0x8700AF, usage_high: 0x005F87, usage_low: 0xAF0000,
            button_focus_bg: 0x005F87, button_focus_fg: 0xEEEEEE,
            button_bg: 0xDADADA, button_fg: 0x6C6C6C,
            success: 0x008700, warning: 0xD75F00, error: 0xAF0000,
            agent_claude: 0xD75F00, agent_codex: 0x008700, agent_antigravity: 0x0087AF,
        }),
        builtin_theme!("tomorrow", "Tomorrow", dark: false, {
            bg: 0xFFFFFF, fg: 0x4D4D4C, muted: 0x8E908C, dim: 0xD6D6D6,
            accent: 0x4271AE, on_accent: 0xFFFFFF,
            selection_bg: 0x4271AE, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xEFEFEF,
            key_hint: 0x8959A8, usage_high: 0x4271AE, usage_low: 0xC82829,
            button_focus_bg: 0x4271AE, button_focus_fg: 0xFFFFFF,
            button_bg: 0xEFEFEF, button_fg: 0x8E908C,
            success: 0x718C00, warning: 0xEAB700, error: 0xC82829,
            agent_claude: 0xF5871F, agent_codex: 0x718C00, agent_antigravity: 0x3E999F,
        }),
        builtin_theme!("modus-operandi", "Modus Operandi", dark: false, {
            bg: 0xFFFFFF, fg: 0x000000, muted: 0x595959, dim: 0x9F9F9F,
            accent: 0x0031A9, on_accent: 0xFFFFFF,
            selection_bg: 0x0031A9, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xC4C4C4,
            key_hint: 0x721045, usage_high: 0x0031A9, usage_low: 0xA60000,
            button_focus_bg: 0x0031A9, button_focus_fg: 0xFFFFFF,
            button_bg: 0xE0E0E0, button_fg: 0x595959,
            success: 0x006800, warning: 0x6F5500, error: 0xA60000,
            agent_claude: 0x972500, agent_codex: 0x006800, agent_antigravity: 0x005E8B,
        }),
        // Ular Dark: the project's own dark palette — a dark blue-gray base
        // carrying the brand colors of the pre-theme-system era (cyan accent,
        // #78AAFF key hints, the CLD/CDX/AGY badge colors).
        builtin_theme!("ular-dark", "Ular Dark", dark: true, {
            bg: 0x171A21, fg: 0xD5DBE5, muted: 0x8891A0, dim: 0x3A4150,
            accent: 0x40C4D4, on_accent: 0x171A21,
            selection_bg: 0x40C4D4, selection_fg: 0x171A21,
            selection_inactive_bg: 0x2A2F3A,
            key_hint: 0x78AAFF, usage_high: 0x5096FF, usage_low: 0xEB5A5A,
            button_focus_bg: 0x5096FF, button_focus_fg: 0xFFFFFF,
            button_bg: 0x2A2F3A, button_fg: 0xAAB3C0,
            success: 0x8CDCA0, warning: 0xE6C475, error: 0xEB5A5A,
            agent_claude: 0xD97757, agent_codex: 0x8CDCA0, agent_antigravity: 0x78AAFF,
        }),
        // Ular Light: warm cream background with the brand steel-blue family.
        // Fully hex-fixed — no terminal-delegated (Reset/ANSI-named) roles remain,
        // so it renders identically on every terminal (and the backdrop fade can
        // blend against a known background). Bright brand tones that washed out
        // on the cream background (key hints, mint/sky agent badges, ANSI
        // success/warning/error) are darkened same-hue variants.
        builtin_theme!("ular-light", "Ular Light", dark: false, {
            bg: 0xFDF6E3, fg: 0x39465A, muted: 0x7E93AE, dim: 0xA9BACE,
            accent: 0x5F82B0, on_accent: 0xFFFFFF,
            selection_bg: 0x5F82B0, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xCBD8E9,
            key_hint: 0x4E77C2, usage_high: 0x3D7FE6, usage_low: 0xD64545,
            button_focus_bg: 0x5096FF, button_focus_fg: 0xFFFFFF,
            button_bg: 0xEEE8D5, button_fg: 0x7E93AE,
            success: 0x2E8F5B, warning: 0xB58900, error: 0xD14343,
            agent_claude: 0xC2603F, agent_codex: 0x3F9668, agent_antigravity: 0x4E77C2,
        }),
        // --- Colorblind-safe (CVD) themes: 3 dark, 3 light. success=blue-ish,
        // error=orange/vermillion/magenta, never red-vs-green semantics. ---
        builtin_theme!("cb-okabe-ito-dark", "(CVD) Okabe-Ito Dark", dark: true, {
            bg: 0x1B1B1B, fg: 0xE8E8E8, muted: 0x9A9A9A, dim: 0x5A5A5A,
            accent: 0x56B4E9, on_accent: 0x1B1B1B,
            selection_bg: 0x56B4E9, selection_fg: 0x1B1B1B,
            selection_inactive_bg: 0x333333,
            key_hint: 0xF0E442, usage_high: 0x56B4E9, usage_low: 0xE69F00,
            button_focus_bg: 0x0072B2, button_focus_fg: 0xFFFFFF,
            button_bg: 0x3A3A3A, button_fg: 0xE8E8E8,
            success: 0x009E73, warning: 0xE69F00, error: 0xD55E00,
            agent_claude: 0xE69F00, agent_codex: 0x009E73, agent_antigravity: 0xCC79A7,
        }),
        builtin_theme!("cb-ibm-dark", "(CVD) IBM Carbon Dark", dark: true, {
            bg: 0x161616, fg: 0xF4F4F4, muted: 0xA8A8A8, dim: 0x525252,
            accent: 0x785EF0, on_accent: 0xF4F4F4,
            selection_bg: 0x785EF0, selection_fg: 0xF4F4F4,
            selection_inactive_bg: 0x393939,
            key_hint: 0xFFB000, usage_high: 0x648FFF, usage_low: 0xFE6100,
            button_focus_bg: 0x648FFF, button_focus_fg: 0x161616,
            button_bg: 0x393939, button_fg: 0xF4F4F4,
            success: 0x648FFF, warning: 0xFFB000, error: 0xDC267F,
            agent_claude: 0xFE6100, agent_codex: 0x33B1FF, agent_antigravity: 0x785EF0,
        }),
        builtin_theme!("cb-tol-dark", "(CVD) Paul Tol Dark", dark: true, {
            bg: 0x1A1A1A, fg: 0xE6E6E6, muted: 0xBBBBBB, dim: 0x555555,
            accent: 0x33BBEE, on_accent: 0x1A1A1A,
            selection_bg: 0x33BBEE, selection_fg: 0x1A1A1A,
            selection_inactive_bg: 0x333333,
            key_hint: 0xCCBB44, usage_high: 0x0077BB, usage_low: 0xEE7733,
            button_focus_bg: 0x0077BB, button_focus_fg: 0xFFFFFF,
            button_bg: 0x333333, button_fg: 0xE6E6E6,
            success: 0x009988, warning: 0xCCBB44, error: 0xCC3311,
            agent_claude: 0xEE7733, agent_codex: 0x009988, agent_antigravity: 0xEE3377,
        }),
        builtin_theme!("cb-okabe-ito-light", "(CVD) Okabe-Ito Light", dark: false, {
            bg: 0xFBFBFB, fg: 0x1A1A1A, muted: 0x555555, dim: 0xC8C8C8,
            accent: 0x0072B2, on_accent: 0xFFFFFF,
            selection_bg: 0x0072B2, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xDCDCDC,
            key_hint: 0xCC79A7, usage_high: 0x0072B2, usage_low: 0xD55E00,
            button_focus_bg: 0x0072B2, button_focus_fg: 0xFFFFFF,
            button_bg: 0xE8E8E8, button_fg: 0x555555,
            success: 0x009E73, warning: 0xC77E00, error: 0xD55E00,
            agent_claude: 0xD55E00, agent_codex: 0x009E73, agent_antigravity: 0xCC79A7,
        }),
        builtin_theme!("cb-ibm-light", "(CVD) IBM Carbon Light", dark: false, {
            bg: 0xFFFFFF, fg: 0x161616, muted: 0x525252, dim: 0xC6C6C6,
            accent: 0x0043CE, on_accent: 0xFFFFFF,
            selection_bg: 0x0043CE, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xDDE1E6,
            key_hint: 0x6929C4, usage_high: 0x0043CE, usage_low: 0xC64600,
            button_focus_bg: 0x0043CE, button_focus_fg: 0xFFFFFF,
            button_bg: 0xE0E0E0, button_fg: 0x525252,
            success: 0x0043CE, warning: 0xB28600, error: 0xC22463,
            agent_claude: 0xC64600, agent_codex: 0x0F62FE, agent_antigravity: 0x6929C4,
        }),
        builtin_theme!("cb-tol-light", "(CVD) Paul Tol Light", dark: false, {
            bg: 0xFCFCFC, fg: 0x222222, muted: 0x555555, dim: 0xCCCCCC,
            accent: 0x004488, on_accent: 0xFFFFFF,
            selection_bg: 0x004488, selection_fg: 0xFFFFFF,
            selection_inactive_bg: 0xDEDEDE,
            key_hint: 0xAA3377, usage_high: 0x004488, usage_low: 0xCC5500,
            button_focus_bg: 0x004488, button_focus_fg: 0xFFFFFF,
            button_bg: 0xE6E6E6, button_fg: 0x555555,
            success: 0x228833, warning: 0xDDAA33, error: 0xBB5566,
            agent_claude: 0xCC5500, agent_codex: 0x228833, agent_antigravity: 0xAA3377,
        }),
    ]
}

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
fn theme_from_file(key: &str, data: &str) -> Option<Theme> {
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
fn load_custom_themes_from(dir: &std::path::Path) -> Vec<Theme> {
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
mod tests {
    use super::*;

    #[test]
    fn builtins_have_unique_keys_and_expected_split() {
        let themes = builtin_themes();
        assert_eq!(themes.len(), 40);
        assert_eq!(themes.iter().filter(|t| t.dark).count(), 20);
        assert_eq!(themes.iter().filter(|t| !t.dark).count(), 20);
        let mut keys: Vec<&str> = themes.iter().map(|t| t.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 40);
        assert!(themes.iter().all(|t| !t.custom));
    }

    #[test]
    fn ular_light_is_fully_hex_fixed_on_cream() {
        let t = builtin_themes()
            .into_iter()
            .find(|t| t.key == "ular-light")
            .expect("ular-light exists");
        assert_eq!(t.bg, Color::Rgb(0xFD, 0xF6, 0xE3));
        // No terminal-delegated (Reset / ANSI-named) roles remain: every role must
        // be a concrete Rgb so rendering matches on any terminal and the backdrop
        // fade can blend against known colors.
        let roles = [
            t.bg,
            t.fg,
            t.muted,
            t.dim,
            t.accent,
            t.on_accent,
            t.selection_bg,
            t.selection_fg,
            t.selection_inactive_bg,
            t.key_hint,
            t.usage_high,
            t.usage_low,
            t.button_focus_bg,
            t.button_focus_fg,
            t.button_bg,
            t.button_fg,
            t.success,
            t.warning,
            t.error,
            t.agent_claude,
            t.agent_codex,
            t.agent_antigravity,
        ];
        assert!(roles.iter().all(|c| matches!(c, Color::Rgb(..))));
    }

    #[test]
    fn default_theme_is_nord() {
        let theme = default_theme();
        assert_eq!(theme.key, "nord");
        assert_eq!(theme.bg, Color::Rgb(0x2E, 0x34, 0x40));
    }

    #[test]
    fn resolve_unknown_key_falls_back_to_default() {
        assert_eq!(resolve("no-such-theme").key, DEFAULT_THEME_KEY);
    }

    #[test]
    fn parse_color_accepts_hex_and_names() {
        assert_eq!(parse_color("#FF8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(parse_color("ff8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(parse_color(" DarkGray "), Some(Color::DarkGray));
        assert_eq!(parse_color("default"), Some(Color::Reset));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("not-a-color"), None);
    }

    #[test]
    fn theme_file_overrides_partial_roles_over_base() {
        let data = r##"
name = "My Theme"
dark = false
base = "dracula"

[colors]
bg = "#101010"
error = "red"
bogus_role = "#FFFFFF"
warning = "not-a-color"
"##;
        let theme = theme_from_file("my-theme", data).expect("parses");
        assert_eq!(theme.key, "my-theme");
        assert_eq!(theme.name, "My Theme");
        assert!(!theme.dark);
        assert!(theme.custom);
        assert_eq!(theme.bg, Color::Rgb(0x10, 0x10, 0x10));
        assert_eq!(theme.error, Color::Red);
        // Unparsable value keeps the base (Dracula) color.
        let dracula = builtin_themes()
            .into_iter()
            .find(|t| t.key == "dracula")
            .unwrap();
        assert_eq!(theme.warning, dracula.warning);
        assert_eq!(theme.fg, dracula.fg);
    }

    #[test]
    fn theme_file_defaults_name_and_base() {
        let theme = theme_from_file("bare", "").expect("empty file parses");
        assert_eq!(theme.name, "bare");
        assert!(theme.custom);
        let nord = default_theme();
        assert_eq!(theme.bg, nord.bg);
        assert_eq!(theme.accent, nord.accent);
    }

    #[test]
    fn custom_themes_load_from_dir_sorted() {
        let dir = std::env::temp_dir().join(format!("s7s-theme-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zeta.toml"), "name = \"Zeta\"\n").unwrap();
        std::fs::write(dir.join("alpha.toml"), "name = \"Alpha\"\n").unwrap();
        std::fs::write(dir.join("ignored.txt"), "not a theme").unwrap();
        std::fs::write(dir.join("broken.toml"), "name = [not toml").unwrap();
        let themes = load_custom_themes_from(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            themes.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "Zeta"]
        );
        assert!(themes.iter().all(|t| t.custom));
    }

    #[test]
    fn themes_sorting_order() {
        let themes = all_themes();

        // 1. All non-CVD themes must be placed before CVD themes
        let cvd_start_idx = themes
            .iter()
            .position(|t| t.name.starts_with("(CVD) "))
            .unwrap();

        for theme in &themes[0..cvd_start_idx] {
            assert!(!theme.name.starts_with("(CVD) "));
        }

        for theme in &themes[cvd_start_idx..] {
            assert!(theme.name.starts_with("(CVD) "));
        }

        // 2. Non-CVD themes must be sorted alphabetically (case-insensitive)
        for i in 0..cvd_start_idx - 1 {
            let name_a = themes[i].name.to_lowercase();
            let name_b = themes[i + 1].name.to_lowercase();
            assert!(
                name_a <= name_b,
                "Non-CVD order mismatch: {} vs {}",
                themes[i].name,
                themes[i + 1].name
            );
        }

        // 3. CVD themes must be sorted alphabetically (case-insensitive)
        for i in cvd_start_idx..themes.len() - 1 {
            let name_a = themes[i].name.to_lowercase();
            let name_b = themes[i + 1].name.to_lowercase();
            assert!(
                name_a <= name_b,
                "CVD order mismatch: {} vs {}",
                themes[i].name,
                themes[i + 1].name
            );
        }
    }
}
