//! Profile screen rendering: the profile list table (with the merged usage
//! cell and status column) and the add/edit form, deletion, and
//! config-directory confirmation modals.

pub(crate) use super::dialogs::{
    draw_profile_delete_confirm, draw_profile_dir_confirm, draw_profile_form,
};
use crate::theme::Theme;
use crate::ui::components::modal::titled_block_nav;
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::truncate_w;
use crate::ui::render::{
    age_label, agent_tag, display_path, pulse_level_now, pulse_span, window_text, LOADING_LABEL,
    MISSING_DIR_LABEL, NOT_INSTALLED_LABEL, NOT_LOGGED_IN_LABEL,
};
use crate::ui::{App, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

/// Profile list table (profiles view only, full width).
pub(crate) fn draw_profile_table(f: &mut Frame, app: &App, area: Rect) {
    use std::collections::HashMap;

    let th = &app.theme;

    // Session count per profile.
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for s in &app.sessions {
        *counts.entry(s.profile_id.as_str()).or_insert(0) += 1;
    }
    // Numbered profiles mapped to header index numbers (<1>..<5>).
    let mut numbers: HashMap<&str, usize> = HashMap::new();
    for (i, p) in app.profiles.numbered_profiles().iter().enumerate() {
        numbers.insert(p.id.as_str(), i + 1);
    }

    let header_style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    // USAGE stays one table cell so long status messages fit, while its two
    // fixed-width windows read as separate columns.
    let header = Row::new(vec![
        aligned_cell("#", Alignment::Center),
        aligned_cell("A", Alignment::Center),
        Cell::from("NAME"),
        Cell::from("STATUS"),
        Cell::from(format!("{:^12}  {:^12}", "5H(LEFT)", "1W(LEFT)")),
        Cell::from("UPDATED"),
        aligned_cell("SESSION", Alignment::Right),
        Cell::from("CONFIG DIR"),
    ])
    .style(header_style);

    // Truncates CONFIG DIR column to fill the remaining width.
    // Fixed columns (4 + 4 + 20 + 13 + 26 + 8 + 7 = 82) + gaps (7 * 2) + borders (2) + highlight (1) + margin (1).
    let config_dir_w = (area.width as usize).saturating_sub(82 + 14 + 2 + 1 + 1);
    let now = crate::usage::now_ms();

    let rows: Vec<Row> = app
        .profiles
        .profiles
        .iter()
        .map(|p| {
            let entry = app.usage.entry(&p.id);
            let subdued = !p.active
                || matches!(
                    entry.phase,
                    crate::usage::UsagePhase::NotLoggedIn | crate::usage::UsagePhase::NotInstalled
                )
                || profile_usage_unavailable(entry);
            let text_style = if subdued {
                th.soft_dim()
            } else {
                Style::default()
            };
            let num = numbers
                .get(p.id.as_str())
                .filter(|&&n| n <= crate::profile::MAX_PROFILE_SHORTCUTS)
                .map(|n| format!("<{n}>"))
                .unwrap_or_default();
            let (tag, tag_color) = agent_tag(p.agent, th);
            let tag_style = if subdued {
                th.soft_dim()
            } else {
                Style::default().fg(tag_color)
            };
            let name_style = if subdued {
                th.soft_dim()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            };
            let (st, st_color) = profile_status(entry, th);
            let status_style = if subdued {
                th.soft_dim()
            } else {
                Style::default().fg(st_color)
            };
            // Under Loading states, only the "Loading..." text in the STATUS cell flashes via fade pulse.
            let status_span = if st == LOADING_LABEL {
                pulse_span(Span::styled(st, status_style), pulse_level_now(), th)
            } else {
                Span::styled(st, status_style)
            };
            let key_style = if subdued {
                th.soft_dim()
            } else {
                Style::default().fg(th.key_hint)
            };
            // If config directory is missing, render error label inside USAGE cell (width 26)
            // instead of usage stats (reads side-by-side with STATUS "Error" to clearly expose cause).
            let usage_cell = if entry.phase == crate::usage::UsagePhase::MissingDir {
                let style = if subdued {
                    th.soft_dim()
                } else {
                    Style::default().fg(th.error)
                };
                Cell::from(Span::styled(MISSING_DIR_LABEL, style))
            } else {
                profile_usage_cell(entry, subdued, now, th)
            };
            // When the shown values were read; `-` before any success.
            let updated = entry
                .last_at
                .map(|at| age_label(at, now))
                .unwrap_or_else(|| "-".to_string());
            let mut row = Row::new(vec![
                aligned_cell(Line::from(Span::styled(num, key_style)), Alignment::Center),
                aligned_cell(Line::from(Span::styled(tag, tag_style)), Alignment::Center),
                Cell::from(Span::styled(truncate_w(&p.name, 20), name_style)),
                Cell::from(status_span),
                usage_cell,
                Cell::from(Span::styled(updated, th.soft_dim())),
                aligned_cell(
                    Line::from(Span::styled(
                        format!("{}", counts.get(p.id.as_str()).copied().unwrap_or(0)),
                        text_style,
                    )),
                    Alignment::Right,
                ),
                Cell::from(Span::styled(
                    truncate_w(&display_path(&p.path), config_dir_w),
                    text_style,
                )),
            ]);
            if subdued {
                row = row.style(th.soft_dim());
            }
            row
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(4),
        Constraint::Length(20),
        Constraint::Length(13), // STATUS column (max "Not logged in")
        Constraint::Length(26), // USAGE column (5H left + 1W left)
        Constraint::Length(8),  // UPDATED column (max "just now")
        Constraint::Length(7),
        Constraint::Min(10),
    ];

    let title = format!(" Profile[{}] ", app.profiles.profiles.len());
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(2)
        .block(titled_block_nav(
            &title,
            app.mode == UiMode::Table,
            false,
            false,
            true,
            th,
        ))
        .row_highlight_style(
            Style::default()
                .bg(th.selection_bg)
                .fg(th.selection_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(" ");

    let mut state: std::cell::RefMut<TableState> = app.profile_table_state.borrow_mut();
    if app.profiles.profiles.is_empty() {
        state.select(None);
    } else {
        state.select(Some(app.profile_selected));
    }
    f.render_stateful_widget(table, area, &mut state);
    let offset = state.offset();
    drop(state);
    let viewport = (area.height as usize).saturating_sub(3);
    draw_vscrollbar(
        f,
        area,
        app.mode == UiMode::Table,
        offset,
        app.profiles.profiles.len(),
        viewport,
        th,
    );
}

fn aligned_cell<'a, T>(content: T, alignment: Alignment) -> Cell<'a>
where
    T: Into<Text<'a>>,
{
    Cell::from(content.into().alignment(alignment))
}

/// Status column label values: missing path/failed queries `Error` (Red),
/// logged out `Not logged in` (Yellow), uninstalled CLI `Not installed` (dim),
/// usage limit hit `Limit reached` (Red), healthy `OK` (Green), actively querying `Loading...`,
/// unsupported profile type `Unavailable` (dim), directory absent `Error` (Red), not queried `-`.
/// During updates, `Loading...` overrides other status phases.
fn profile_status(entry: crate::usage::UsageEntry, th: &Theme) -> (String, Color) {
    use crate::usage::UsagePhase;
    if entry.phase == UsagePhase::Loading {
        return (LOADING_LABEL.to_string(), th.muted);
    }
    if profile_usage_unavailable(entry) {
        return ("Limit reached".to_string(), th.usage_low);
    }
    match entry.phase {
        UsagePhase::Loading => (LOADING_LABEL.to_string(), th.muted),
        UsagePhase::Ready => ("OK".to_string(), th.success),
        UsagePhase::Failed => ("Error".to_string(), th.error),
        UsagePhase::NotLoggedIn => (NOT_LOGGED_IN_LABEL.to_string(), th.warning),
        UsagePhase::NotInstalled => (NOT_INSTALLED_LABEL.to_string(), th.dim),
        UsagePhase::MissingDir => ("Error".to_string(), th.error),
        UsagePhase::Unavailable => ("Unavailable".to_string(), th.dim),
        UsagePhase::Idle => ("-".to_string(), th.dim),
    }
}

fn profile_usage_unavailable(entry: crate::usage::UsageEntry) -> bool {
    entry.last.is_some_and(|snapshot| {
        snapshot.current.is_some_and(|window| window.pct_left == 0)
            || snapshot.weekly.is_some_and(|window| window.pct_left == 0)
    })
}

fn profile_usage_parts(
    entry: crate::usage::UsageEntry,
    subdued: bool,
    now: i64,
    th: &Theme,
) -> [(String, Style); 2] {
    use crate::usage::{UsagePhase, UsageSnapshot, UsageWindow};

    let pct_style = |window: UsageWindow| -> Style {
        if window.pct_left >= 50 {
            Style::default().fg(th.usage_high)
        } else {
            Style::default().fg(th.usage_low)
        }
    };

    let window_part = |window: Option<UsageWindow>,
                       weekly: bool,
                       style_override: Option<Style>|
     -> (String, Style) {
        let fallback_style = style_override.unwrap_or_else(|| Style::default().fg(th.dim));
        match window {
            Some(window) => {
                // A window whose reset has passed since the read is grayed.
                let (text, passed) = window_text(window, weekly, entry.last_at, now);
                let style = match style_override {
                    Some(style) => style,
                    None if passed => th.soft_dim(),
                    None => pct_style(window),
                };
                (text, style)
            }
            None => (format!("{:>3}%        ", "--"), fallback_style),
        }
    };

    let snapshot_parts =
        |snapshot: UsageSnapshot, style_override: Option<Style>| -> [(String, Style); 2] {
            [
                window_part(snapshot.current, false, style_override),
                window_part(snapshot.weekly, true, style_override),
            ]
        };
    let empty_parts = |style: Style| {
        [
            (format!("{:>3}%        ", "--"), style),
            (format!("{:>3}%        ", "--"), style),
        ]
    };

    let subdued_style = subdued.then(|| th.soft_dim());
    if let Some(style) = subdued_style {
        if let Some(snapshot) = entry.last {
            return snapshot_parts(snapshot, Some(style));
        }
        return empty_parts(style);
    }

    match (entry.phase, entry.last) {
        (UsagePhase::Loading, Some(snapshot)) | (UsagePhase::Failed, Some(snapshot)) => {
            snapshot_parts(snapshot, Some(th.soft_dim()))
        }
        (UsagePhase::Loading, None) => empty_parts(th.soft_dim()),
        (UsagePhase::Ready, Some(snapshot)) => snapshot_parts(snapshot, None),
        _ => empty_parts(Style::default().fg(th.dim)),
    }
}

/// Merged USAGE cell (width 26) for profile tables. Its two 12-cell windows use
/// the header's `72%(4h 30m)` format and are separated by two spaces.
fn profile_usage_cell(
    entry: crate::usage::UsageEntry,
    subdued: bool,
    now: i64,
    th: &Theme,
) -> Cell<'static> {
    let [(current, current_style), (weekly, weekly_style)] =
        profile_usage_parts(entry, subdued, now, th);
    aligned_cell(
        Line::from(vec![
            Span::styled(current, current_style),
            Span::raw("  "),
            Span::styled(weekly, weekly_style),
        ]),
        Alignment::Left,
    )
}

#[cfg(test)]
mod tests {
    use super::{draw_profile_table, profile_usage_parts, profile_usage_unavailable};
    use crate::ui::test_support::app_with_profiles;
    use crate::usage::{ResetCountdown, UsageEntry, UsagePhase, UsageSnapshot, UsageWindow};
    use ratatui::{backend::TestBackend, Terminal};

    fn ready_usage(current: UsageWindow, weekly: UsageWindow) -> UsageEntry {
        UsageEntry {
            phase: UsagePhase::Ready,
            last: Some(UsageSnapshot {
                current: Some(current),
                weekly: Some(weekly),
            }),
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

    #[test]
    fn profile_usage_columns_combine_percent_and_reset() {
        let parts = profile_usage_parts(
            ready_usage(usage_window(72, 0, 4, 30), usage_window(52, 2, 16, 0)),
            false,
            0,
            &crate::theme::default_theme(),
        );
        let texts: Vec<String> = parts.into_iter().map(|(text, _)| text).collect();

        assert_eq!(texts, vec![" 72%(4h 30m)", " 52%(2d 16h)"]);
    }

    #[test]
    fn profile_usage_headers_name_the_remaining_windows() {
        let app = app_with_profiles();
        let mut terminal = Terminal::new(TestBackend::new(160, 8)).expect("terminal");
        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_profile_table(frame, &app, area);
            })
            .expect("draw profile table");
        let buffer = terminal.backend().buffer();
        let header: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 1)].symbol())
            .collect();

        assert!(header.contains("5H(LEFT)"), "{header}");
        assert!(header.contains("1W(LEFT)"), "{header}");
        assert!(header.contains("UPDATED"), "{header}");
        assert!(!header.contains("RESET"), "{header}");
    }

    #[test]
    fn profile_usage_unavailable_when_any_window_is_zero() {
        assert!(profile_usage_unavailable(ready_usage(
            usage_window(0, 0, 4, 30),
            usage_window(52, 2, 16, 0),
        )));
        assert!(profile_usage_unavailable(ready_usage(
            usage_window(72, 0, 4, 30),
            usage_window(0, 2, 16, 0),
        )));
        assert!(!profile_usage_unavailable(ready_usage(
            usage_window(72, 0, 4, 30),
            usage_window(52, 2, 16, 0),
        )));
    }
}
