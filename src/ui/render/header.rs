//! Header: logo, per-screen shortcut columns, and the profile usage rows.

use super::usage::{is_eye_closed_now, pulse_level_now, pulse_span, usage_spans};
use crate::ui::components::text::{pad_w, truncate_w};
use crate::ui::{App, Focus, Screen};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Right-aligned ASCII art logo (5 rows, width 16, splitting "s-7-s" partitions).
const LOGO_WIDTH: u16 = 16;
const LOGO_GAP: u16 = 2;
const LOGO_PARTS: [(&str, &str, &str); 5] = [
    ("", "    ██████      ", ""),
    (" ____", "   ██", "____  "),
    ("/ ___) ", "██", "/ ___) "),
    ("\\___ \\", "██", " \\___ \\ "),
    ("(____/", "██", " (____/ "),
];

const ICON_EYES_OPEN: [&str; 5] = [
    "  ▄▄     ▄▄ ",
    "  ▀▀     ▀▀ ",
    " ████   ████",
    " ████▄▄▄████",
    "  ▀███████▀ ",
];

const ICON_EYES_CLOSED: [&str; 5] = [
    "            ",
    " ▀▀▀▀   ▀▀▀▀",
    " ████   ████",
    " ████▄▄▄████",
    "  ▀███████▀ ",
];

/// Screen-specific hotkeys (columns 1 and 2 of the left hotkey grid, structured as `(key, action)`).
/// Column 3 is filled by `SHORTCUTS_COMMON`, and the right profile usage keys column is dynamically generated.
pub(super) const SHORTCUTS_SESSION: [&[(&str, &str)]; 2] = [
    // Filter controls
    &[
        ("/", "Search"),
        ("a", "Agents"),
        ("f", "Folder"),
        ("c", "Copy to Clipboard"),
        ("0", "Clear"),
    ],
    // Session operations. The header has five rows; the conditional context
    // source heading exposes Ctrl+O beside its target instead.
    &[
        ("enter", "Resume Session"),
        ("ctrl+n", "New Session"),
        ("ctrl+r", "Rename Session"),
        ("ctrl+d", "Delete Session"),
        ("ctrl+b", "Bookmark"),
    ],
];

/// Column 2 specific to the session details view. Arrow key navigations are left only in help.
pub(super) const SHORTCUTS_DETAIL: [&[(&str, &str)]; 2] = [
    &[(".", "Expand Prompt / Tools"), ("c", "Copy to Clipboard")],
    &[
        ("enter", "Resume Session"),
        ("ctrl+n", "New Session"),
        ("ctrl+r", "Rename Session"),
        ("ctrl+d", "Delete Session"),
        ("ctrl+b", "Bookmark"),
    ],
];

/// Column 2 specific to the profiles view.
pub(super) const SHORTCUTS_PROFILE: [&[(&str, &str)]; 2] = [
    &[("1..5", "Set Order"), ("space", "Toggle Order")],
    &[
        ("+", "Add Profile"),
        ("ctrl+e", "Edit Profile"),
        ("ctrl+d", "Delete Profile"),
    ],
];

/// Session screen columns while its workspace pane is focused.
pub(super) const SHORTCUTS_WORKSPACE_LIST: [&[(&str, &str)]; 2] = [
    &[("/", "Search"), ("+", "Add Workspace")],
    &[
        ("enter", "Edit Workspace"),
        ("ctrl+d", "Delete Workspace"),
        ("esc", "Close List"),
    ],
];

/// Column 3 shared across all views (screen rotation, refreshes, help).
pub(super) const SHORTCUTS_COMMON: &[(&str, &str)] = &[
    (":", "Quick Command"),
    ("!", "Terminal Command"),
    ("ctrl+w", "Open Workspace"),
    ("ctrl+u", "Refresh"),
    ("?", "Help"),
];

/// Renders top header: left side displays profile quick keys, usage stats, and hotkey grid; right side shows the logo when every hotkey column fits.
pub(super) fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let numbered = app.profiles.numbered_profiles();
    let is_loading = numbered
        .iter()
        .any(|p| app.usage.entry(&p.id).phase == crate::usage::UsagePhase::Loading);
    // Layout from left: 1 space padding -> quick-keys + usage column -> screen-specific columns (2) + common column (1).
    // Quick-keys column width = key (3) + space (1) + name (max of active profiles, clamped between 12-18)
    //                          + usage leading space (1) + usage segments (current 12 + space 1 + weekly 12 + " left" 5)
    //                          + right padding 8 (margin to column 2).
    // Left hotkey columns dynamically fit to content maximum width + 4 right padding. Drops rightmost columns if space is constrained.
    let name_w = numbered
        .iter()
        .map(|p| p.name.as_str().width())
        .max()
        .unwrap_or(12)
        .clamp(12, 18);
    let agent_col_w = 4 + name_w as u16 + 39;
    let screen_cols: &[&[(&str, &str)]; 2] = match app.screen {
        Screen::Session if app.focus == Focus::Workspaces => &SHORTCUTS_WORKSPACE_LIST,
        Screen::Session => &SHORTCUTS_SESSION,
        Screen::Profile => &SHORTCUTS_PROFILE,
        Screen::Detail => &SHORTCUTS_DETAIL,
    };
    let left_cols: [&[(&str, &str)]; 3] = [screen_cols[0], screen_cols[1], SHORTCUTS_COMMON];
    // Keys are padded to each column's widest `<key>` so action descriptions start at
    // one aligned column: `<key><pad> action`.
    let key_widths: Vec<usize> = left_cols
        .iter()
        .map(|col| {
            col.iter()
                .map(|(key, _)| UnicodeWidthStr::width(format!("<{key}>").as_str()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let left_widths: Vec<u16> = left_cols
        .iter()
        .zip(&key_widths)
        .map(|(col, key_w)| {
            let action_w = col
                .iter()
                .map(|(_, action)| UnicodeWidthStr::width(*action))
                .max()
                .unwrap_or(0);
            (key_w + 1 + action_w) as u16 + 4
        })
        .collect();
    // 1-space padding to the left of the quick-keys column.
    const LEFT_PAD: u16 = 1;
    let mut visible = left_widths.len();
    let mut total = LEFT_PAD + agent_col_w + left_widths.iter().sum::<u16>();
    let show_logo = area.width >= total.saturating_add(LOGO_GAP + LOGO_WIDTH);
    let (key_area, logo_area) = if show_logo {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(LOGO_GAP), Constraint::Length(LOGO_WIDTH)])
            .split(area);
        (cols[0], Some(cols[1]))
    } else {
        (area, None)
    };

    while visible > 0 && total > key_area.width {
        visible -= 1;
        total -= left_widths[visible];
    }
    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(LEFT_PAD),
        Constraint::Length(agent_col_w),
    ];
    constraints.extend(
        left_widths[..visible]
            .iter()
            .map(|w| Constraint::Length(*w)),
    );
    constraints.push(Constraint::Min(0)); // Flexible trailing space, including the logo gap when shown
    let key_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(key_area);

    // Quick-keys column (rendered after 1 space padding, index=1): maps numbered profiles to hotkeys (<1>..<5>) and remaining usage percentage.
    // Always top-aligned. Profile names are padded to fit width, keeping usage stats left-aligned.
    let th = &app.theme;
    let key_style = th.key_style();
    let agent_lines: Vec<Line> = numbered
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let key_txt = format!("<{}>", i + 1);
            let name = pad_w(&truncate_w(&p.name, name_w), name_w);
            let entry = app.usage.entry(&p.id);
            let mut spans = vec![
                Span::styled(key_txt, key_style),
                Span::styled(format!(" {}", name), th.soft_dim()),
                Span::raw(" "),
            ];
            // During Loading queries, only the "Loading..." placeholder in the usage column fades/pulses.
            let usage = usage_spans(entry, th);
            if entry.phase == crate::usage::UsagePhase::Loading {
                let level = pulse_level_now();
                spans.extend(usage.into_iter().map(|s| pulse_span(s, level, th)));
            } else {
                spans.extend(usage);
            }
            Line::from(spans)
        })
        .collect();
    // Quick-keys column is rendered directly with top alignment (index=1).
    f.render_widget(Paragraph::new(agent_lines), key_cols[1]);

    // Renders hotkey columns sequentially (appended after quick-keys, index = 2 + ci).
    for (ci, col) in left_cols.iter().take(visible).enumerate() {
        let key_w = key_widths[ci];
        let mut lines: Vec<Line> = Vec::new();
        for (key, action) in col.iter() {
            lines.push(Line::from(vec![
                Span::styled(pad_w(&format!("<{key}>"), key_w), th.key_style()),
                Span::styled(format!(" {}", action), th.soft_dim()),
            ]));
        }
        f.render_widget(Paragraph::new(lines), key_cols[2 + ci]);
    }

    let Some(logo_area) = logo_area else {
        return;
    };

    // Logo: ASCII art rendered left-aligned within the rightmost column to maintain art alignment.
    // Center '7' segment highlighted in the same color as the eye (U) icon; flanking 's' segments in accent.
    // If any active profile is loading, render the blinking eye (U) icon instead of the standard logo.
    let closed = is_eye_closed_now();
    let logo_lines: Vec<Line> = if is_loading {
        (0..5)
            .map(|i| {
                let icon = if closed {
                    ICON_EYES_CLOSED[i]
                } else {
                    ICON_EYES_OPEN[i]
                };
                Line::from(Span::styled(icon, Style::default().fg(th.accent)))
            })
            .collect()
    } else {
        LOGO_PARTS
            .iter()
            .map(|(s1, seven, s2)| {
                Line::from(vec![
                    Span::styled(
                        *s1,
                        Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        *seven,
                        Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        *s2,
                        Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
                    ),
                ])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(logo_lines), logo_area);
}
