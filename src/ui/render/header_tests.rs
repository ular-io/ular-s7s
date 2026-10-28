//! Header shortcut columns, logo, and usage span rendering tests.

use super::header::{
    SHORTCUTS_COMMON, SHORTCUTS_DETAIL, SHORTCUTS_PROFILE, SHORTCUTS_SESSION,
    SHORTCUTS_WORKSPACE_LIST,
};
use super::test_support::*;
use super::{age_label, usage_tail_span, usage_value_spans};
use crate::ui::components::text::{pad_w, truncate_w};
use crate::usage::{ResetCountdown, UsageEntry, UsagePhase, UsageSnapshot, UsageWindow};
use ratatui::style::{Color, Modifier};
use ratatui::{backend::TestBackend, Terminal};

/// Fixed render time; `ready_usage` values are read exactly then.
const NOW: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;

fn plain_usage(entry: UsageEntry) -> String {
    plain_usage_at(entry, NOW)
}

fn plain_usage_at(entry: UsageEntry, now: i64) -> String {
    usage_value_spans(entry, now, &crate::theme::default_theme())
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect()
}

fn usage_style_attrs(entry: UsageEntry) -> Vec<(Option<Color>, Modifier)> {
    usage_value_spans(entry, NOW, &crate::theme::default_theme())
        .into_iter()
        .map(|span| (span.style.fg, span.style.add_modifier))
        .collect()
}

fn tail(entry: UsageEntry, now: i64) -> (String, Option<Color>) {
    let span = usage_tail_span(entry, now, &crate::theme::default_theme());
    (span.content.into_owned(), span.style.fg)
}

/// Muted (soft-dim) fg of the default theme — what grayed-out usage spans use.
fn muted() -> Color {
    crate::theme::default_theme().muted
}

fn ready_usage(current: UsageWindow, weekly: UsageWindow) -> UsageEntry {
    UsageEntry {
        phase: UsagePhase::Ready,
        last: Some(UsageSnapshot {
            current: Some(current),
            weekly: Some(weekly),
        }),
        last_at: Some(NOW),
    }
}

fn with_phase(entry: UsageEntry, phase: UsagePhase) -> UsageEntry {
    UsageEntry { phase, ..entry }
}

fn without_value(phase: UsagePhase) -> UsageEntry {
    UsageEntry {
        phase,
        last: None,
        last_at: None,
    }
}

fn usage_window(pct_left: u8, days: u16, hours: u8, minutes: u8) -> UsageWindow {
    UsageWindow {
        pct_left,
        reset: Some(ResetCountdown {
            days,
            hours,
            minutes,
        }),
    }
}

fn usage_window_without_reset(pct_left: u8) -> UsageWindow {
    UsageWindow {
        pct_left,
        reset: None,
    }
}

fn header_usage_line(n: usize, name: &str, entry: UsageEntry) -> String {
    let name = pad_w(&truncate_w(name, 12), 12);
    format!("<{n}> {name} {}", plain_usage(entry))
}

/// The header is a fixed five rows and each hotkey column renders as one
/// top-aligned `Paragraph`, so a sixth entry would be silently clipped.
#[test]
fn header_shortcut_columns_fit_the_five_row_header() {
    const HEADER_ROWS: usize = 5;
    for col in SHORTCUTS_SESSION
        .iter()
        .chain(SHORTCUTS_DETAIL.iter())
        .chain(SHORTCUTS_PROFILE.iter())
        .chain(SHORTCUTS_WORKSPACE_LIST.iter())
        .chain(std::iter::once(&SHORTCUTS_COMMON))
    {
        assert!(
            col.len() <= HEADER_ROWS,
            "column overflows the header: {col:?}"
        );
    }
}

#[test]
fn header_shows_the_bookmark_key() {
    let app = session_app();
    let mut terminal = Terminal::new(TestBackend::new(160, 30)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");

    // Fifth row of the session-operations column, still inside the header.
    let (_, y) = find_cell(&terminal, "Bookmark");
    assert!(y < 5, "shortcut row {y} is clipped out of the header");
}

#[test]
fn usage_spans_use_compact_current_reset_width() {
    let entry = ready_usage(
        UsageWindow {
            pct_left: 15,
            reset: Some(ResetCountdown {
                days: 0,
                hours: 1,
                minutes: 50,
            }),
        },
        UsageWindow {
            pct_left: 38,
            reset: Some(ResetCountdown {
                days: 1,
                hours: 22,
                minutes: 0,
            }),
        },
    );

    assert_eq!(plain_usage(entry), " 15%(1h 50m)  38%(1d 22h) left");
}

#[test]
fn usage_spans_show_weekly_reset_in_minutes_under_one_hour() {
    let entry = ready_usage(usage_window(15, 0, 0, 12), usage_window(3, 0, 0, 45));

    assert_eq!(plain_usage(entry), " 15%(   12m)   3%(   45m) left");
}

#[test]
fn usage_spans_keep_fixed_width_when_current_reset_is_empty() {
    let entry = ready_usage(usage_window_without_reset(100), usage_window(0, 0, 22, 0));

    assert_eq!(plain_usage(entry), "100%           0%(   22h) left");
}

#[test]
fn usage_spans_dim_gray_all_usage_when_any_window_is_zero() {
    let current_zero = ready_usage(usage_window(0, 0, 4, 30), usage_window(52, 2, 16, 0));
    let weekly_zero = ready_usage(usage_window(72, 0, 4, 30), usage_window(0, 2, 16, 0));

    assert_eq!(
        usage_style_attrs(current_zero),
        vec![
            (Some(muted()), Modifier::empty()),
            (None, Modifier::empty()),
            (Some(muted()), Modifier::empty()),
            (Some(muted()), Modifier::empty())
        ]
    );
    assert_eq!(
        usage_style_attrs(weekly_zero),
        vec![
            (Some(muted()), Modifier::empty()),
            (None, Modifier::empty()),
            (Some(muted()), Modifier::empty()),
            (Some(muted()), Modifier::empty())
        ]
    );
}

#[test]
fn header_usage_lines_match_documented_spacing() {
    let rows = [
        header_usage_line(
            1,
            "Claude",
            ready_usage(usage_window(15, 0, 1, 50), usage_window(38, 1, 22, 0)),
        ),
        header_usage_line(
            2,
            "Antigravity",
            ready_usage(
                usage_window_without_reset(100),
                usage_window_without_reset(100),
            ),
        ),
        header_usage_line(
            3,
            "Codex",
            ready_usage(usage_window(69, 0, 4, 0), usage_window(92, 0, 22, 0)),
        ),
        header_usage_line(
            4,
            "CLD-Share",
            ready_usage(usage_window_without_reset(100), usage_window(0, 0, 22, 0)),
        ),
    ];

    assert_eq!(
        rows,
        [
            "<1> Claude        15%(1h 50m)  38%(1d 22h) left",
            "<2> Antigravity  100%         100%         left",
            "<3> Codex         69%(4h  0m)  92%(   22h) left",
            "<4> CLD-Share    100%           0%(   22h) left",
        ]
    );
}

#[test]
fn header_hides_logo_before_shortcut_columns() {
    let app = session_app();
    // This fixture's complete left side needs 148 cells; 165 leaves room for
    // the 16-cell logo but not the required 2-cell gap.
    let text = rendered_text(&app, 165, 20);

    assert!(text.contains("Quick Command"));
    assert!(text.contains("Help"));
    assert!(!text.contains("██████"));
}

#[test]
fn header_shows_logo_when_shortcuts_logo_and_gap_fit() {
    let app = session_app();
    let text = rendered_text(&app, 166, 20);

    assert!(text.contains("Quick Command"));
    assert!(text.contains("Help"));
    assert!(text.contains("██████"));
}

#[test]
fn header_drops_rightmost_shortcut_column_after_logo_is_hidden() {
    let app = session_app();
    let text = rendered_text(&app, 147, 20);

    assert!(text.contains("Resume Session"));
    assert!(!text.contains("Quick Command"));
    assert!(!text.contains("██████"));
}

#[test]
fn age_labels_floor_to_one_unit() {
    assert_eq!(age_label(NOW, NOW + 59_999), "just now");
    assert_eq!(age_label(NOW, NOW + 5 * MINUTE), "5m ago");
    assert_eq!(age_label(NOW, NOW + 90 * MINUTE), "1h ago");
    assert_eq!(age_label(NOW, NOW + (23 * 60 + 59) * MINUTE), "23h ago");
    assert_eq!(age_label(NOW, NOW + 3 * 1440 * MINUTE), "3d ago");
    // A clock that moved backwards never shows a negative age.
    assert_eq!(age_label(NOW, NOW - MINUTE), "just now");
}

#[test]
fn reset_countdowns_keep_counting_and_mark_a_passed_reset() {
    let entry = ready_usage(usage_window(72, 0, 4, 30), usage_window(52, 0, 1, 0));
    assert_eq!(
        plain_usage_at(entry, NOW + 90 * MINUTE),
        " 72%(3h  0m)  52%(reset)  left"
    );
    // The window whose reset passed is grayed on its own.
    let attrs: Vec<_> = usage_value_spans(entry, NOW + 90 * MINUTE, &crate::theme::default_theme())
        .into_iter()
        .map(|span| span.style.fg)
        .collect();
    let th = crate::theme::default_theme();
    assert_eq!(attrs[0], Some(th.usage_high));
    assert_eq!(attrs[2], Some(muted()));
    // Within the first minute the value reads exactly as captured.
    assert_eq!(
        plain_usage_at(entry, NOW + 30_000),
        " 72%(4h 30m)  52%(    1h) left"
    );
}

#[test]
fn tail_shows_the_age_of_a_value_and_pulses_nothing_else_while_loading() {
    let entry = ready_usage(usage_window(72, 0, 4, 30), usage_window(52, 2, 16, 0));
    assert_eq!(tail(entry, NOW).0, "just now");
    assert_eq!(tail(entry, NOW + 125 * MINUTE).0, "2h ago");

    // Rechecking keeps the values on screen; only the age is the loading cue.
    let rechecking = with_phase(entry, UsagePhase::Loading);
    assert_eq!(plain_usage(rechecking), plain_usage(entry));
    assert_eq!(tail(rechecking, NOW + 125 * MINUTE).0, "2h ago");
    assert_eq!(
        tail(without_value(UsagePhase::Loading), NOW).0,
        "Loading..."
    );
    assert_eq!(
        plain_usage(without_value(UsagePhase::Loading)),
        " --%          --%         left"
    );
}

#[test]
fn failures_and_logouts_keep_the_last_values_grayed() {
    let th = crate::theme::default_theme();
    let entry = ready_usage(usage_window(41, 0, 1, 12), usage_window(38, 2, 14, 0));
    for (phase, label, color) in [
        (UsagePhase::Failed, "Error", th.error),
        (UsagePhase::NotLoggedIn, "Not logged in", th.warning),
    ] {
        let stale = with_phase(entry, phase);
        assert_eq!(plain_usage(stale), " 41%(1h 12m)  38%(2d 14h) left");
        assert!(usage_style_attrs(stale)
            .iter()
            .all(|(fg, _)| fg.is_none() || *fg == Some(muted())));
        assert_eq!(tail(stale, NOW), (label.to_string(), Some(color)));
    }
}

#[test]
fn states_without_values_show_their_reason_in_the_tail() {
    let th = crate::theme::default_theme();
    for (phase, label, color) in [
        (UsagePhase::Failed, "Error", Some(th.error)),
        (UsagePhase::MissingDir, "Error", Some(th.error)),
        (UsagePhase::NotLoggedIn, "Not logged in", Some(th.warning)),
        (UsagePhase::NotInstalled, "Not installed", Some(muted())),
        (UsagePhase::Unavailable, "Unavailable", Some(muted())),
    ] {
        let entry = without_value(phase);
        assert_eq!(plain_usage(entry), " --%          --%         left");
        assert_eq!(tail(entry, NOW), (label.to_string(), color), "{phase:?}");
    }
    // A missing folder or uninstalled CLI never shows a stale value.
    let entry = ready_usage(usage_window(41, 0, 1, 12), usage_window(38, 2, 14, 0));
    assert_eq!(
        plain_usage(with_phase(entry, UsagePhase::MissingDir)),
        " --%          --%         left"
    );
}
