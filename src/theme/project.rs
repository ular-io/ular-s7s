//! The project's own palettes (Ular Dark / Ular Light) and the colorblind-safe
//! (CVD) palettes, appended after the standard built-ins.

use super::{rgb, Theme};

pub(super) fn project_themes() -> Vec<Theme> {
    vec![
        // Ular Dark: the project's own dark palette — a dark blue-gray base
        // carrying the brand colors of the pre-theme-system era (cyan accent,
        // #78AAFF key hints, the CLD/CDX/AGY badge colors).
        builtin_theme!("ular-dark", "Ular Dark", dark: true, {
            bg: 0x171A21, fg: 0xD5DBE5, muted: 0x8891A0, dim: 0x3A4150,
            accent: 0x40C4D4, on_accent: 0x171A21,
            selection_bg: 0x40C4D4, selection_fg: 0x171A21,
            selection_inactive_bg: 0x2A2F3A,
            bookmark_bg: 0x1C2E36,
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
            bookmark_bg: 0xEAE8DD,
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
            bookmark_bg: 0x222D34,
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
            bookmark_bg: 0x221F30,
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
            bookmark_bg: 0x1D2D33,
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
            bookmark_bg: 0xDDEBF2,
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
            bookmark_bg: 0xE0E8F9,
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
            bookmark_bg: 0xDEE6EE,
            key_hint: 0xAA3377, usage_high: 0x004488, usage_low: 0xCC5500,
            button_focus_bg: 0x004488, button_focus_fg: 0xFFFFFF,
            button_bg: 0xE6E6E6, button_fg: 0x555555,
            success: 0x228833, warning: 0xDDAA33, error: 0xBB5566,
            agent_claude: 0xCC5500, agent_codex: 0x228833, agent_antigravity: 0xAA3377,
        }),
    ]
}
