//! Usage labels, loading pulse animation, and per-profile usage spans shared by
//! the header and the Profile table.

use crate::theme::Theme;
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};
use unicode_width::UnicodeWidthStr;

/// Label shown during usage queries. Shared by header usage section and STATUS column;
/// fade pulse animations are applied solely to this label.
pub(crate) const LOADING_LABEL: &str = "Loading...";

/// Error label for profiles with missing config directories. Rendered in place of usage stats
/// in header section and profile table USAGE cells (width 30); STATUS preserves "Error" state.
pub(crate) const MISSING_DIR_LABEL: &str = "Config folder not found";

/// Logged out and installation error labels. Shared by header usage section and STATUS column.
pub(crate) const NOT_LOGGED_IN_LABEL: &str = "Not logged in";
pub(crate) const NOT_INSTALLED_LABEL: &str = "Not installed";

/// Loading pulse animation sequence: normal -> light -> lighter -> invisible -> lighter -> light -> normal ...
const PULSE_SEQ: [u8; 6] = [0, 1, 2, 3, 2, 1];
/// Pulse step duration. Set to double the main loop refresh rate (100ms in main.rs) to avoid
/// aliasing, guaranteeing that each animation frame renders for at least two frames (1.2s period).
const PULSE_STEP_MS: u128 = 200;

pub(crate) fn pulse_level_now() -> u8 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    PULSE_SEQ[((ms / PULSE_STEP_MS) as usize) % PULSE_SEQ.len()]
}

pub(super) fn is_eye_closed_now() -> bool {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let cycle = ms % 800;
    cycle >= 400
}

/// Applies pulse animation level to style. Level 1 (light) blends fg 40% toward the
/// theme background (fades on dark and light themes alike); Level 2 (lighter) applies
/// soft dimming (muted+DIM); Level 3 (invisible) is handled by content space
/// replacement in `pulse_span` rather than styling.
fn pulse_style(style: Style, level: u8, th: &Theme) -> Style {
    match level {
        1 => {
            let (r, g, b) = crate::theme::color_rgb(style.fg.unwrap_or(th.muted));
            let (br, bg, bb) = th.bg_rgb();
            let blend = |f: u8, b: u8| (f as f32 * 0.6 + b as f32 * 0.4) as u8;
            style.fg(Color::Rgb(blend(r, br), blend(g, bg), blend(b, bb)))
        }
        2 => style.fg(th.muted).add_modifier(Modifier::DIM),
        _ => style,
    }
}

/// Applies pulse animation step to a Span (0 preserves original). Level 3 replaces content
/// with equivalent width spaces to guarantee invisibility regardless of terminal conceal support.
pub(crate) fn pulse_span(span: Span<'static>, level: u8, th: &Theme) -> Span<'static> {
    match level {
        0 => span,
        3 => Span::styled(" ".repeat(span.content.as_ref().width()), span.style),
        l => Span::styled(span.content, pulse_style(span.style, l, th)),
    }
}

/// Width of current (5h) reset label: `(4h 30m)` = 8.
const RESET_W_CURRENT: usize = 8;
/// Width of weekly reset label: `(2d  6h)` = 8.
const RESET_W_WEEKLY: usize = 8;

// Current (5h) countdown format: `(4h 30m)`, `(17h  6m)`, `(   45m)` — minutes are right-aligned to width 2.
pub(crate) fn reset_label_current(reset: Option<crate::usage::ResetCountdown>) -> String {
    let Some(r) = reset else {
        return String::new();
    };
    let hours = r.days as u32 * 24 + r.hours as u32;
    if hours > 0 {
        format!("({}h {:>2}m)", hours, r.minutes)
    } else {
        format!("(   {:>2}m)", r.minutes)
    }
}

// Weekly countdown tracks down to hours, dropping minutes: `(2d  6h)`, `(   17h)`, `(    2h)`.
// Under an hour hours would read `(    0h)`, so it falls back to minutes: `(   45m)`.
pub(crate) fn reset_label_weekly(reset: Option<crate::usage::ResetCountdown>) -> String {
    let Some(r) = reset else {
        return String::new();
    };
    if r.days > 0 {
        format!("({}d {:>2}h)", r.days, r.hours)
    } else if r.hours > 0 {
        format!("(   {:>2}h)", r.hours)
    } else {
        format!("(   {:>2}m)", r.minutes)
    }
}

/// Spans representing agent usage stats (`current weekly left`).
/// Under `Loading` states, only renders `Loading...` placeholder.
/// Otherwise, preserves fixed width segments to keep vertical alignment across rows.
/// 1 space padding separating current and weekly segments:
/// ```text
///  72%(4h 30m)  52%(2d  6h) left
///   0%(17h  6m)   0%(   17h) left
/// 100%(4h 15m)  51%(    2h) left
///  --%           --%         left
/// ```
/// Percent values are right-aligned to width 3; "left" label is rendered in soft dim gray.
pub(crate) fn usage_spans(entry: crate::usage::UsageEntry, th: &Theme) -> Vec<Span<'static>> {
    use crate::usage::{UsagePhase, UsageSnapshot, UsageWindow};

    // `{:>3}% + reset (fixed width)` segment. Pads with whitespace if reset is absent.
    let render_window = |window: Option<UsageWindow>, weekly: bool| -> (String, Style) {
        let reset_w = if weekly {
            RESET_W_WEEKLY
        } else {
            RESET_W_CURRENT
        };
        match window {
            Some(window) => {
                let color = if window.pct_left >= 50 {
                    th.usage_high
                } else {
                    th.usage_low
                };
                let reset = if weekly {
                    reset_label_weekly(window.reset)
                } else {
                    reset_label_current(window.reset)
                };
                (
                    format!("{:>3}%{:<w$}", window.pct_left, reset, w = reset_w),
                    Style::default().fg(color),
                )
            }
            None => (
                format!("{:>3}%{}", "--", " ".repeat(reset_w)),
                Style::default().fg(th.dim),
            ),
        }
    };

    let placeholder = |txt: &str, weekly: bool| -> String {
        let reset_w = if weekly {
            RESET_W_WEEKLY
        } else {
            RESET_W_CURRENT
        };
        format!("{:>3}%{}", txt, " ".repeat(reset_w))
    };

    // Gray out segments to indicate pending or failed states while preserving values.
    let gray = |seg: (String, Style)| -> (String, Style) { (seg.0, th.soft_dim()) };

    fn has_zero_window(snapshot: UsageSnapshot) -> bool {
        snapshot.current.is_some_and(|window| window.pct_left == 0)
            || snapshot.weekly.is_some_and(|window| window.pct_left == 0)
    }

    // Under query states (startup or Ctrl+U refreshes): only show "Loading..." to consistently indicate active checking.
    if entry.phase == UsagePhase::Loading {
        return vec![Span::styled(LOADING_LABEL, th.soft_dim())];
    }

    // Label-only states without numeric values (fixed width padding not required here; trailing columns handled separately).
    match entry.phase {
        UsagePhase::NotLoggedIn => {
            return vec![Span::styled(NOT_LOGGED_IN_LABEL, th.soft_dim())];
        }
        UsagePhase::NotInstalled => {
            return vec![Span::styled(NOT_INSTALLED_LABEL, th.soft_dim())];
        }
        UsagePhase::MissingDir => {
            return vec![Span::styled(
                MISSING_DIR_LABEL,
                Style::default().fg(th.error),
            )];
        }
        _ => {}
    }

    let (current, weekly): ((String, Style), (String, Style)) = match (entry.phase, entry.last) {
        // Failed state: falls back to dim gray rendering of the last cached snapshot if available.
        (UsagePhase::Failed, Some(snapshot)) => (
            gray(render_window(snapshot.current, false)),
            gray(render_window(snapshot.weekly, true)),
        ),
        (UsagePhase::Ready, Some(snapshot)) if has_zero_window(snapshot) => (
            gray(render_window(snapshot.current, false)),
            gray(render_window(snapshot.weekly, true)),
        ),
        (UsagePhase::Ready, Some(snapshot)) => (
            render_window(snapshot.current, false),
            render_window(snapshot.weekly, true),
        ),
        _ => (
            (placeholder("--", false), Style::default().fg(th.dim)),
            (placeholder("--", true), Style::default().fg(th.dim)),
        ),
    };
    vec![
        Span::styled(current.0, current.1),
        Span::raw(" "),
        Span::styled(weekly.0, weekly.1),
        Span::styled(" left", th.soft_dim()),
    ]
}
