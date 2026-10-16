//! Header shortcut columns, logo, and usage span rendering tests.

use super::header::{
    SHORTCUTS_COMMON, SHORTCUTS_DETAIL, SHORTCUTS_PROFILE, SHORTCUTS_SESSION,
    SHORTCUTS_WORKSPACE_LIST,
};
use super::test_support::*;
use super::usage_spans;
use crate::ui::components::text::{pad_w, truncate_w};
use crate::usage::{ResetCountdown, UsageEntry, UsagePhase, UsageSnapshot, UsageWindow};
use ratatui::style::{Color, Modifier};
use ratatui::{backend::TestBackend, Terminal};

fn plain_usage(entry: UsageEntry) -> String {
    usage_spans(entry, &crate::theme::default_theme())
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect()
}

fn usage_style_attrs(entry: UsageEntry) -> Vec<(Option<Color>, Modifier)> {
    usage_spans(entry, &crate::theme::default_theme())
        .into_iter()
        .map(|span| (span.style.fg, span.style.add_modifier))
        .collect()
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
    // This fixture's complete left side needs 137 cells; 154 leaves room for
    // the 16-cell logo but not the required 2-cell gap.
    let text = rendered_text(&app, 154, 20);

    assert!(text.contains("Quick Command"));
    assert!(text.contains("Help"));
    assert!(!text.contains("██████"));
}

#[test]
fn header_shows_logo_when_shortcuts_logo_and_gap_fit() {
    let app = session_app();
    let text = rendered_text(&app, 155, 20);

    assert!(text.contains("Quick Command"));
    assert!(text.contains("Help"));
    assert!(text.contains("██████"));
}

#[test]
fn header_drops_rightmost_shortcut_column_after_logo_is_hidden() {
    let app = session_app();
    let text = rendered_text(&app, 136, 20);

    assert!(text.contains("Resume Session"));
    assert!(!text.contains("Quick Command"));
    assert!(!text.contains("██████"));
}

#[test]
fn usage_spans_show_not_logged_in() {
    let entry = UsageEntry {
        phase: UsagePhase::NotLoggedIn,
        last: None,
    };
    assert_eq!(plain_usage(entry), "Not logged in");
    assert_eq!(
        usage_style_attrs(entry),
        vec![(Some(muted()), Modifier::empty())]
    );
}

#[test]
fn usage_spans_show_not_installed() {
    let entry = UsageEntry {
        phase: UsagePhase::NotInstalled,
        last: None,
    };
    assert_eq!(plain_usage(entry), "Not installed");
}

#[test]
fn usage_spans_show_missing_dir() {
    let entry = UsageEntry {
        phase: UsagePhase::MissingDir,
        last: None,
    };
    assert_eq!(plain_usage(entry), "Config folder not found");
    assert_eq!(
        usage_style_attrs(entry),
        vec![(Some(crate::theme::default_theme().error), Modifier::empty())]
    );
}

#[test]
fn usage_spans_show_loading_message() {
    // Under Loading queries, always renders "Loading..." placeholder, bypassing cached results.
    let fresh = UsageEntry {
        phase: UsagePhase::Loading,
        last: None,
    };
    let rechecking = UsageEntry {
        phase: UsagePhase::Loading,
        last: Some(UsageSnapshot {
            current: Some(usage_window(72, 0, 4, 30)),
            weekly: Some(usage_window(52, 2, 16, 0)),
        }),
    };
    assert_eq!(plain_usage(fresh), "Loading...");
    assert_eq!(plain_usage(rechecking), "Loading...");
    assert_eq!(
        usage_style_attrs(fresh),
        vec![(Some(muted()), Modifier::empty())]
    );
}
