//! Markdown body colors (`md_*` roles) for rendered assistant answers.
//!
//! Ular Light carries values the user tuned by eye on its cream background and
//! is never derived. Every other palette derives the roles from its own colors:
//! a base role is nudged toward black (light palettes) or white (dark palettes)
//! until text reaches 4.5:1 and list markers 3:1 against the background, which
//! keeps the hue instead of collapsing accents into the body text color.

use super::{color_rgb, rgb, Theme};
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarkdownColors {
    /// Full-width heading bar and its text.
    pub heading_bg: Color,
    pub heading_fg: Color,
    /// Inline-code chip: body text on a pale highlighter tint.
    pub code_bg: Color,
    pub code_fg: Color,
    /// Fenced/indented code block text (the frame uses `border`).
    pub code_block_fg: Color,
    pub table_head: Color,
    /// List bullets, ordered numbers, and task boxes.
    pub marker: Color,
    pub link: Color,
    pub quote: Color,
    /// Code-block and table frames, quote bar, horizontal rule.
    pub border: Color,
    /// GFM alert quotes (`> [!NOTE]` ...): label text, then the fill per kind
    /// shared by the label chip and the quote bar.
    pub alert_fg: Color,
    pub alert_note: Color,
    pub alert_tip: Color,
    pub alert_important: Color,
    pub alert_warning: Color,
    pub alert_caution: Color,
}

/// Placeholder the palette macro replaces with derived values right away.
pub(super) const UNSET: MarkdownColors = MarkdownColors {
    heading_bg: Color::Reset,
    heading_fg: Color::Reset,
    code_bg: Color::Reset,
    code_fg: Color::Reset,
    code_block_fg: Color::Reset,
    table_head: Color::Reset,
    marker: Color::Reset,
    link: Color::Reset,
    quote: Color::Reset,
    border: Color::Reset,
    alert_fg: Color::Reset,
    alert_note: Color::Reset,
    alert_tip: Color::Reset,
    alert_important: Color::Reset,
    alert_warning: Color::Reset,
    alert_caution: Color::Reset,
};

/// User-decided values for Ular Light (cream `#FDF6E3`). Some sit below the
/// derived palettes' contrast targets by the user's choice; keep them as is.
pub(super) const ULAR_LIGHT: MarkdownColors = MarkdownColors {
    heading_bg: rgb(0xD0D0D0),
    heading_fg: rgb(0x073642),
    code_bg: rgb(0xF3E6A8),
    code_fg: rgb(0x073642),
    code_block_fg: rgb(0x005FAF),
    table_head: rgb(0x005F87),
    marker: rgb(0x839496),
    link: rgb(0x268BD2),
    quote: rgb(0x859900),
    border: rgb(0x002B36),
    alert_fg: rgb(0xFFFFFF),
    alert_note: rgb(0x073642),
    alert_tip: rgb(0x1A7F37),
    alert_important: rgb(0x8250DF),
    alert_warning: rgb(0x9A6700),
    alert_caution: rgb(0xCF222E),
};

const TEXT_CONTRAST: f64 = 4.5;
const MARKER_CONTRAST: f64 = 3.0;

impl MarkdownColors {
    pub(super) fn derive(t: &Theme) -> Self {
        let bg = t.bg_rgb();
        let toward = if t.dark { (255, 255, 255) } else { (0, 0, 0) };
        let fit =
            |c: Color, against, need| as_color(reach_contrast(color_rgb(c), against, toward, need));
        let heading_bg = mix(bg, color_rgb(t.fg), 0.12);
        let code_bg = mix(bg, color_rgb(t.warning), 0.25);
        Self {
            heading_bg: as_color(heading_bg),
            heading_fg: fit(t.fg, heading_bg, TEXT_CONTRAST),
            code_bg: as_color(code_bg),
            code_fg: fit(t.fg, code_bg, TEXT_CONTRAST),
            code_block_fg: fit(t.key_hint, bg, TEXT_CONTRAST),
            table_head: fit(t.accent, bg, TEXT_CONTRAST),
            marker: fit(t.accent, bg, MARKER_CONTRAST),
            link: fit(t.key_hint, bg, TEXT_CONTRAST),
            quote: fit(t.success, bg, TEXT_CONTRAST),
            border: t.dim,
            // Labels are written in the page background color on each fill.
            alert_fg: as_color(bg),
            alert_note: fit(t.fg, bg, TEXT_CONTRAST),
            alert_tip: fit(t.success, bg, TEXT_CONTRAST),
            alert_important: fit(t.accent, bg, TEXT_CONTRAST),
            alert_warning: fit(t.warning, bg, TEXT_CONTRAST),
            alert_caution: fit(t.error, bg, TEXT_CONTRAST),
        }
    }

    /// Custom theme file key (`md_heading_bg`, ...) to its slot.
    pub(super) fn slot(&mut self, role: &str) -> Option<&mut Color> {
        Some(match role {
            "md_heading_bg" => &mut self.heading_bg,
            "md_heading_fg" => &mut self.heading_fg,
            "md_code_bg" => &mut self.code_bg,
            "md_code_fg" => &mut self.code_fg,
            "md_code_block_fg" => &mut self.code_block_fg,
            "md_table_head" => &mut self.table_head,
            "md_marker" => &mut self.marker,
            "md_link" => &mut self.link,
            "md_quote" => &mut self.quote,
            "md_border" => &mut self.border,
            "md_alert_fg" => &mut self.alert_fg,
            "md_alert_note" => &mut self.alert_note,
            "md_alert_tip" => &mut self.alert_tip,
            "md_alert_important" => &mut self.alert_important,
            "md_alert_warning" => &mut self.alert_warning,
            "md_alert_caution" => &mut self.alert_caution,
            _ => return None,
        })
    }
}

type Rgb = (u8, u8, u8);

fn as_color((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let ch = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (ch(a.0, b.0), ch(a.1, b.1), ch(a.2, b.2))
}

/// First 1/40 step from `c` toward `toward` that meets `need` against `bg`.
fn reach_contrast(c: Rgb, bg: Rgb, toward: Rgb, need: f64) -> Rgb {
    (0..=40)
        .map(|i| mix(c, toward, i as f64 / 40.0))
        .find(|&m| contrast(m, bg) >= need)
        .unwrap_or(toward)
}

/// WCAG relative-luminance contrast ratio.
pub(super) fn contrast(a: Rgb, b: Rgb) -> f64 {
    let lum = |(r, g, b): Rgb| {
        let ch = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
