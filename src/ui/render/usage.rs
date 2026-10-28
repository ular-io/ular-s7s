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
pub(crate) const UNAVAILABLE_LABEL: &str = "Unavailable";

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
/// Shown in place of a countdown whose reset time has passed since the value was read.
const RESET_PASSED_LABEL: &str = "(reset)";
/// Width of the header's trailing slot: the widest label, `Not logged in` / `Not installed`.
pub(crate) const TAIL_W: usize = 13;

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

/// Age of a value read at `at_ms`, floored to one unit: `just now` (under a
/// minute), `5m ago`, `1h ago` (1h 30m included), `3d ago`.
pub(crate) fn age_label(at_ms: i64, now_ms: i64) -> String {
    let minutes = now_ms.saturating_sub(at_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".to_string(),
        1..=59 => format!("{minutes}m ago"),
        60..=1439 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

/// Fixed-width `{:>3}%` + countdown text of one window, the countdown advanced
/// from `taken_at` to `now`. The flag is true once the reset time has passed:
/// the countdown reads `(reset)` and the percentage no longer applies.
pub(crate) fn window_text(
    window: crate::usage::UsageWindow,
    weekly: bool,
    taken_at: Option<i64>,
    now: i64,
) -> (String, bool) {
    let reset_w = if weekly {
        RESET_W_WEEKLY
    } else {
        RESET_W_CURRENT
    };
    let (reset, passed) = match (window.reset, taken_at) {
        (Some(r), Some(at)) => match r.remaining(at, now) {
            Some(left) => (Some(left), false),
            None => (None, true),
        },
        (reset, _) => (reset, false),
    };
    let label = if passed {
        RESET_PASSED_LABEL.to_string()
    } else if weekly {
        reset_label_weekly(reset)
    } else {
        reset_label_current(reset)
    };
    (
        format!("{:>3}%{:<w$}", window.pct_left, label, w = reset_w),
        passed,
    )
}

fn has_zero_window(snapshot: crate::usage::UsageSnapshot) -> bool {
    snapshot.current.is_some_and(|window| window.pct_left == 0)
        || snapshot.weekly.is_some_and(|window| window.pct_left == 0)
}

/// The two usage windows plus ` left`, in fixed-width segments so rows align:
/// ```text
///  72%(4h 30m)  52%(2d  6h) left
///   0%(17h  6m)   0%(   17h) left
/// 100%(4h 15m)  51%(reset)  left
///  --%           --%         left
/// ```
/// Values come from the last success whenever one exists (also while
/// `Loading`). They are gray after a failure or logout, and all gray when any
/// window is at 0%; a window whose reset has passed is gray on its own.
pub(crate) fn usage_value_spans(
    entry: crate::usage::UsageEntry,
    now: i64,
    th: &Theme,
) -> Vec<Span<'static>> {
    use crate::usage::UsagePhase;
    let placeholder = |weekly: bool| {
        let reset_w = if weekly {
            RESET_W_WEEKLY
        } else {
            RESET_W_CURRENT
        };
        (
            format!("{:>3}%{}", "--", " ".repeat(reset_w)),
            Style::default().fg(th.dim),
        )
    };
    let shows_last = matches!(
        entry.phase,
        UsagePhase::Ready | UsagePhase::Loading | UsagePhase::Failed | UsagePhase::NotLoggedIn
    );
    let (current, weekly) = match entry.last.filter(|_| shows_last) {
        Some(snapshot) => {
            let all_gray = matches!(entry.phase, UsagePhase::Failed | UsagePhase::NotLoggedIn)
                || has_zero_window(snapshot);
            let segment = |window: Option<crate::usage::UsageWindow>, weekly: bool| match window {
                Some(window) => {
                    let (text, passed) = window_text(window, weekly, entry.last_at, now);
                    let style = if all_gray || passed {
                        th.soft_dim()
                    } else if window.pct_left >= 50 {
                        Style::default().fg(th.usage_high)
                    } else {
                        Style::default().fg(th.usage_low)
                    };
                    (text, style)
                }
                None => placeholder(weekly),
            };
            (
                segment(snapshot.current, false),
                segment(snapshot.weekly, true),
            )
        }
        None => (placeholder(false), placeholder(true)),
    };
    vec![
        Span::styled(current.0, current.1),
        Span::raw(" "),
        Span::styled(weekly.0, weekly.1),
        Span::styled(" left", th.soft_dim()),
    ]
}

/// The header's trailing slot: how old the value is (`2h ago`), why there is
/// none or it is stale (`Error`, `Not logged in`, ...), or `Loading...` for a
/// first query. While `Loading` with a value, the caller pulses the age.
pub(crate) fn usage_tail_span(
    entry: crate::usage::UsageEntry,
    now: i64,
    th: &Theme,
) -> Span<'static> {
    use crate::usage::UsagePhase;
    let age = || {
        entry
            .last_at
            .map(|at| age_label(at, now))
            .unwrap_or_default()
    };
    match entry.phase {
        UsagePhase::Loading if entry.last_at.is_some() => Span::styled(age(), th.soft_dim()),
        UsagePhase::Loading => Span::styled(LOADING_LABEL, th.soft_dim()),
        UsagePhase::Ready => Span::styled(age(), th.soft_dim()),
        UsagePhase::Failed | UsagePhase::MissingDir => {
            Span::styled("Error", Style::default().fg(th.error))
        }
        UsagePhase::NotLoggedIn => {
            Span::styled(NOT_LOGGED_IN_LABEL, Style::default().fg(th.warning))
        }
        UsagePhase::NotInstalled => Span::styled(NOT_INSTALLED_LABEL, th.soft_dim()),
        UsagePhase::Unavailable => Span::styled(UNAVAILABLE_LABEL, th.soft_dim()),
        UsagePhase::Idle => Span::raw(""),
    }
}

/// Compact usage for the New Session profile combo and list: the value spans,
/// or a single label where no value exists (`Loading...`, `Not logged in`,
/// `Not installed`, `Config folder not found`). No age is shown there.
pub(crate) fn usage_spans(entry: crate::usage::UsageEntry, th: &Theme) -> Vec<Span<'static>> {
    use crate::usage::UsagePhase;
    let label = match (entry.phase, entry.last) {
        (UsagePhase::Loading, None) => Some(Span::styled(LOADING_LABEL, th.soft_dim())),
        (UsagePhase::NotLoggedIn, None) => Some(Span::styled(NOT_LOGGED_IN_LABEL, th.soft_dim())),
        (UsagePhase::NotInstalled, _) => Some(Span::styled(NOT_INSTALLED_LABEL, th.soft_dim())),
        (UsagePhase::MissingDir, _) => Some(Span::styled(
            MISSING_DIR_LABEL,
            Style::default().fg(th.error),
        )),
        _ => None,
    };
    match label {
        Some(span) => vec![span],
        None => usage_value_spans(entry, crate::usage::now_ms(), th),
    }
}
