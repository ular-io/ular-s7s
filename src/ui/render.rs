//! TUI rendering: k9s-style header (logo + hotkeys) / session search table / preview /
//! session details (questions list + work/answers) / status bar / modal windows.

use super::{App, DetailFocus, Focus, MessageKind, Screen, SessionDetailState, TextInput, UiMode};
use crate::handoff::WorkKind;
use crate::theme::Theme;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Cell, Clear, List, ListItem, Padding, Paragraph, Row, Table,
        TableState, Wrap,
    },
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Right-aligned ASCII art logo (5 rows, width 16, splitting "s-7-s" partitions).
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
const SHORTCUTS_SESSION: [&[(&str, &str)]; 2] = [
    // Filter controls
    &[
        ("/", "Search"),
        ("a", "Agents"),
        ("f", "Folder"),
        ("0", "Clear"),
    ],
    // Session operations
    &[
        ("enter", "Resume Session"),
        ("ctrl+n", "New Session"),
        ("ctrl+r", "Rename Session"),
        ("ctrl+d", "Delete Session"),
    ],
];

/// Column 2 specific to the session details view. Arrow key navigations are left only in help.
const SHORTCUTS_DETAIL: [&[(&str, &str)]; 2] = [
    &[(".", "Toggle Tool Logs")],
    &[
        ("enter", "Resume Session"),
        ("ctrl+n", "New Session"),
        ("ctrl+r", "Rename Session"),
        ("ctrl+d", "Delete Session"),
    ],
];

/// Column 2 specific to the profiles view.
const SHORTCUTS_PROFILE: [&[(&str, &str)]; 2] = [
    &[("1..5", "Set Order"), ("space", "Toggle Order")],
    &[
        ("+", "Add Profile"),
        ("ctrl+e", "Edit Profile"),
        ("ctrl+d", "Delete Profile"),
    ],
];

/// Column 3 shared across all views (screen rotation, refreshes, help).
const SHORTCUTS_COMMON: &[(&str, &str)] = &[
    (":", "Quick Command"),
    ("!", "Terminal Command"),
    ("ctrl+u", "Refresh"),
    ("?", "Help"),
];

const HELP_GLOBAL: &[(&str, &str)] = &[
    ("?", "Open/close help"),
    ("esc/q", "Close help or clear current state"),
    ("q / ctrl+c", "Quit from table mode (press twice)"),
    (":", "Quick command palette"),
    ("!", "Terminal command in session folder"),
    ("ctrl+u", "Update sessions and usage"),
    ("1..5", "Filter by numbered profile"),
    ("0", "Clear profile filter"),
];

const HELP_SESSION: &[(&str, &str)] = &[
    ("enter", "Resume selected session"),
    ("ctrl+n", "New session (profile/folder dialog)"),
    (
        "ctrl+shift+n",
        "New session with context (fallback: `:` palette)",
    ),
    ("/", "Keyword search"),
    ("a", "Select agent filter"),
    ("f", "Select folder filter"),
    ("g/home", "Go to top"),
    ("G/end", "Go to bottom"),
    ("pageup", "Scroll preview up"),
    ("pagedown", "Scroll preview down"),
    ("ctrl+r", "Rename session"),
    ("ctrl+d/del", "Delete session"),
];

const HELP_DETAIL: &[(&str, &str)] = &[
    ("enter", "Resume session"),
    ("ctrl+n", "New session (profile/folder dialog)"),
    (
        "ctrl+shift+n",
        "New session with context (fallback: `:` palette)",
    ),
    (".", "Show/hide tool calls & results"),
    ("pageup/pagedown", "Scroll work panel"),
    ("g/home", "First question / scroll top"),
    ("G/end", "Last question / scroll bottom"),
    ("ctrl+r", "Rename session"),
    ("ctrl+d/del", "Delete session"),
    ("←/h", "Back to session list"),
];

const HELP_PROFILE: &[(&str, &str)] = &[
    ("ctrl+n", "New session (profile/folder dialog)"),
    ("1..5", "Insert profile at shortcut position"),
    ("space", "Toggle profile shortcut at end"),
    ("+", "Add profile"),
    ("ctrl+e", "Edit profile"),
    ("ctrl+d", "Delete profile"),
    ("g/home", "Go to top"),
    ("G/end", "Go to bottom"),
    ("→/l", "Return to session list"),
];

/// Renders the entire application frame.
pub fn draw(f: &mut Frame, app: &App) {
    // Paint the theme background/foreground under the whole frame first. Widgets that
    // set only fg patch on top of this, so every panel inherits the theme background.
    f.render_widget(Block::default().style(app.theme.base_style()), f.area());

    // Header content is capped at five rows (matching the logo and numbered profile limit).
    let header_h = 5.min(f.area().height.saturating_sub(4));
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h), // Header (hotkeys left, logo right)
            Constraint::Min(3),           // Body (table + preview)
            Constraint::Length(1),        // Status bar
        ])
        .split(f.area());

    draw_header(f, app, root[0]);

    if app.screen == Screen::Profile {
        // Profile view: full-width table without preview panel.
        draw_profile_table(f, app, root[1]);
    } else if app.screen == Screen::Detail {
        draw_detail(f, app, root[1]);
    } else if app.mode == UiMode::Keyword {
        // Keyword mode: overlays search prompt box on top of the main body (k9s-style).
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(root[1]);
        draw_search_prompt(f, app, body[0]);
        draw_body(f, app, body[1]);
    } else {
        draw_body(f, app, root[1]);
    }

    draw_status_bar(f, app, root[2]);

    if backdrop_dimmed(app.mode) {
        dim_backdrop(f, &app.theme);
    }

    match app.mode {
        UiMode::AgentModal => draw_agent_modal(f, app),
        UiMode::FolderModal => draw_folder_modal(f, app),
        UiMode::DeleteConfirm => draw_delete_confirm(f, app),
        UiMode::Rename => draw_rename_modal(f, app),
        UiMode::ProfileForm => draw_profile_form(f, app),
        UiMode::ProfileDeleteConfirm => draw_profile_delete_confirm(f, app),
        UiMode::ProfileDirConfirm => draw_profile_dir_confirm(f, app),
        UiMode::NewSession => draw_new_session_modal(f, app),
        UiMode::ProjectDirConfirm => draw_project_dir_confirm(f, app),
        UiMode::QuickCommand => draw_quick_command(f, app),
        UiMode::ThemeSelect => draw_theme_select(f, app),
        UiMode::Help => draw_help(f, app),
        UiMode::Message => draw_message_modal(f, app),
        _ => {}
    }
}

/// Label shown during usage queries. Shared by header usage section and STATUS column;
/// fade pulse animations are applied solely to this label.
const LOADING_LABEL: &str = "Loading...";

/// Error label for profiles with missing config directories. Rendered in place of usage stats
/// in header section and profile table USAGE cells (width 30); STATUS preserves "Error" state.
const MISSING_DIR_LABEL: &str = "Config folder not found";

/// Logged out and installation error labels. Shared by header usage section and STATUS column.
const NOT_LOGGED_IN_LABEL: &str = "Not logged in";
const NOT_INSTALLED_LABEL: &str = "Not installed";

/// Loading pulse animation sequence: normal -> light -> lighter -> invisible -> lighter -> light -> normal ...
const PULSE_SEQ: [u8; 6] = [0, 1, 2, 3, 2, 1];
/// Pulse step duration. Set to double the main loop refresh rate (100ms in main.rs) to avoid
/// aliasing, guaranteeing that each animation frame renders for at least two frames (1.2s period).
const PULSE_STEP_MS: u128 = 200;

fn pulse_level_now() -> u8 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    PULSE_SEQ[((ms / PULSE_STEP_MS) as usize) % PULSE_SEQ.len()]
}

fn is_eye_closed_now() -> bool {
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
fn pulse_span(span: Span<'static>, level: u8, th: &Theme) -> Span<'static> {
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
fn reset_label_current(reset: Option<crate::usage::ResetCountdown>) -> String {
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

// Weekly countdown discards minutes, tracking down to hours: `(2d  6h)`, `(   17h)`, `(    2h)`.
fn reset_label_weekly(reset: Option<crate::usage::ResetCountdown>) -> String {
    let Some(r) = reset else {
        return String::new();
    };
    if r.days > 0 {
        format!("({}d {:>2}h)", r.days, r.hours)
    } else {
        format!("(   {:>2}h)", r.hours)
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
fn usage_spans(entry: crate::usage::UsageEntry, th: &Theme) -> Vec<Span<'static>> {
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

fn profile_usage_unavailable(entry: crate::usage::UsageEntry) -> bool {
    entry.last.is_some_and(|snapshot| {
        snapshot.current.is_some_and(|window| window.pct_left == 0)
            || snapshot.weekly.is_some_and(|window| window.pct_left == 0)
    })
}

fn profile_usage_parts(
    entry: crate::usage::UsageEntry,
    _tick: usize,
    subdued: bool,
    th: &Theme,
) -> [(String, Style); 4] {
    use crate::usage::{UsagePhase, UsageSnapshot, UsageWindow};

    let pct_style = |window: UsageWindow| -> Style {
        if window.pct_left >= 50 {
            Style::default().fg(th.usage_high)
        } else {
            Style::default().fg(th.usage_low)
        }
    };

    let window_parts = |window: Option<UsageWindow>,
                        weekly: bool,
                        style_override: Option<Style>|
     -> [(String, Style); 2] {
        let fallback_style = style_override.unwrap_or_else(|| Style::default().fg(th.dim));
        match window {
            Some(window) => {
                let reset = if weekly {
                    reset_label_weekly(window.reset)
                } else {
                    reset_label_current(window.reset)
                }
                .trim_start_matches('(')
                .trim_end_matches(')')
                .to_string();
                let reset = if reset.is_empty() {
                    "-".to_string()
                } else {
                    reset
                };
                [
                    (
                        format!("{:>3}%", window.pct_left),
                        style_override.unwrap_or_else(|| pct_style(window)),
                    ),
                    (reset, style_override.unwrap_or_else(|| th.soft_dim())),
                ]
            }
            None => [
                ("-".to_string(), fallback_style),
                ("-".to_string(), fallback_style),
            ],
        }
    };

    let snapshot_parts =
        |snapshot: UsageSnapshot, style_override: Option<Style>| -> [(String, Style); 4] {
            let current = window_parts(snapshot.current, false, style_override);
            let weekly = window_parts(snapshot.weekly, true, style_override);
            [
                current[0].clone(),
                current[1].clone(),
                weekly[0].clone(),
                weekly[1].clone(),
            ]
        };

    let subdued_style = subdued.then(|| th.soft_dim());
    if let Some(style) = subdued_style {
        if let Some(snapshot) = entry.last {
            return snapshot_parts(snapshot, Some(style));
        }
        return [
            ("-".to_string(), style),
            ("-".to_string(), style),
            ("-".to_string(), style),
            ("-".to_string(), style),
        ];
    }

    match (entry.phase, entry.last) {
        (UsagePhase::Loading, Some(snapshot)) | (UsagePhase::Failed, Some(snapshot)) => {
            snapshot_parts(snapshot, Some(th.soft_dim()))
        }
        (UsagePhase::Loading, None) => [
            ("-".to_string(), th.soft_dim()),
            ("-".to_string(), th.soft_dim()),
            ("-".to_string(), th.soft_dim()),
            ("-".to_string(), th.soft_dim()),
        ],
        (UsagePhase::Ready, Some(snapshot)) => snapshot_parts(snapshot, None),
        _ => [
            ("-".to_string(), Style::default().fg(th.dim)),
            ("-".to_string(), Style::default().fg(th.dim)),
            ("-".to_string(), Style::default().fg(th.dim)),
            ("-".to_string(), Style::default().fg(th.dim)),
        ],
    }
}

/// Merged USAGE cell (width 30) for profile tables. Value rendering aligns segments internally
/// (`5H(4) - RESET(8) - 1W(4) - RESET(8)` separated by 2 spaces), padded with `-` if usage or resets
/// cannot be resolved. Status messages (e.g. Logged Out, Not Installed) are delegated to the STATUS column.
fn profile_usage_cell(
    entry: crate::usage::UsageEntry,
    tick: usize,
    subdued: bool,
    th: &Theme,
) -> Cell<'static> {
    let parts = profile_usage_parts(entry, tick, subdued, th);
    aligned_cell(
        Line::from(vec![
            Span::styled(format!("{:>4}", parts[0].0), parts[0].1),
            Span::raw("  "),
            Span::styled(format!("{:^8}", parts[1].0), parts[1].1),
            Span::raw("  "),
            Span::styled(format!("{:>4}", parts[2].0), parts[2].1),
            Span::raw("  "),
            Span::styled(format!("{:^8}", parts[3].0), parts[3].1),
        ]),
        Alignment::Left,
    )
}

/// Renders top header: left side displays profile quick keys, usage stats, and hotkey grid; right side shows right-aligned ASCII logo.
fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let numbered = app.profiles.numbered_profiles();
    let is_loading = numbered
        .iter()
        .any(|p| app.usage.entry(&p.id).phase == crate::usage::UsagePhase::Loading);
    let logo_len = 16;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(logo_len)])
        .split(area);

    // Layout from left: 1 space padding -> quick-keys + usage column -> screen-specific columns (2) + common column (1).
    // Right logo is pushed to right margin using a flexible spacer (Min(0)).
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
    while visible > 0 && total > cols[0].width {
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
    constraints.push(Constraint::Min(0)); // Flexible spacing up to the right logo
    let key_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(cols[0]);

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
    f.render_widget(Paragraph::new(logo_lines), cols[1]);
}

/// Body: left session table and right preview panel.
fn draw_body(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    draw_table(f, app, cols[0]);
    draw_preview(f, app, cols[1]);
}

/// Overlay text box for search query (k9s-style). Only rendered during Keyword filter mode.
fn draw_search_prompt(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(th.accent).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            " Search ",
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(th.base_style()), area);
    f.render_widget(block, area);

    let content_w = inner.width.saturating_sub(1) as usize;
    let wrap_width = content_w.saturating_sub(2);
    // "/ " prefix occupies 2 cells; subsequent lines are padded with two spaces ("  ").
    const PREFIX_W: u16 = 2;
    let mut lines = Vec::new();
    let wrapped = wrap_w(&app.filter.keyword, wrap_width);
    if wrapped.is_empty() {
        lines.push(Line::from(Span::styled("/ ", Style::default().fg(th.dim))));
    } else {
        for (i, chunk) in wrapped.iter().enumerate() {
            let prefix = if i == 0 { "/ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(th.dim)),
                Span::raw(chunk.clone()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(lines), inner);

    // Cursor position calculation: wrap text prior to cursor under matching rules to isolate line and column metrics.
    let cursor = app.keyword_cursor.min(app.filter.keyword.len());
    let before = &app.filter.keyword[..cursor];
    let before_lines = wrap_w(before, wrap_width);
    let (line_idx, col) = match before_lines.last() {
        Some(last) => (
            before_lines.len().saturating_sub(1),
            UnicodeWidthStr::width(last.as_str()),
        ),
        None => (0, 0),
    };
    let cursor_x = inner.x.saturating_add(PREFIX_W).saturating_add(col as u16);
    let cursor_y = inner.y.saturating_add(line_idx as u16);
    // Hard cursor rendered only if coordinates stay within the text box boundaries.
    if cursor_x < inner.x.saturating_add(inner.width)
        && cursor_y < inner.y.saturating_add(inner.height)
    {
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Left session table. Title exhibits `sessions[filter: count]`.
fn draw_table(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let table_focus = app.focus == Focus::Table && app.mode == UiMode::Table;
    let table_dimmed = app.focus == Focus::Preview && app.mode == UiMode::Table;
    let header_style = if table_dimmed {
        th.soft_dim().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
    };
    let text_style = if table_dimmed {
        th.soft_dim()
    } else {
        Style::default()
    };
    let table_style = if table_dimmed {
        th.soft_dim()
    } else {
        Style::default()
    };

    let header = Row::new(vec![
        Cell::from("A"),
        Cell::from("FOLDER"),
        Cell::from("UPDATED"),
        Cell::from("TITLE"),
        Cell::from(format!("{:>4}", "Q")),
        Cell::from(format!("{:>5} ", "SIZE")),
    ])
    .style(header_style);

    // Truncates the TITLE column to fill the remaining width.
    // inner area = area.width - borders (2). Subtracting highlight_symbol (1) +
    // fixed columns (A 4 + FOLDER 18 + UPDATED 11 + Q 4 + SIZE 6 = 43) +
    // spacing (5 gaps for 6 columns) yields TITLE width.
    // UPDATED consists of 10 characters + 1 right padding to add space before TITLE.
    // SIZE consists of 5 right-aligned characters + 1 right margin before the border.
    // Leaves 1 cell padding to prevent right border double-width character overflow.
    let title_w = (area.width as usize).saturating_sub(2 + 1 + 43 + 5 + 1);

    let rows: Vec<Row> = app
        .filtered
        .iter()
        .map(|&i| {
            let s = &app.sessions[i];
            let (tag, color) = agent_tag(s.agent, th);
            let tag_style = if table_dimmed {
                th.soft_dim()
            } else {
                Style::default().fg(color)
            };
            Row::new(vec![
                Cell::from(Span::styled(tag, tag_style)),
                Cell::from(Span::styled(truncate_w(&s.folder, 18), text_style)),
                Cell::from(Span::styled(s.date_str(), text_style)),
                Cell::from(Span::styled(truncate_w(&s.title(), title_w), text_style)),
                Cell::from(Span::styled(
                    format!("{:>4}", s.user_turns.len()),
                    text_style,
                )),
                Cell::from(Span::styled(format!("{:>5} ", s.size_str()), text_style)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(18),
        Constraint::Length(11),
        Constraint::Min(10),
        Constraint::Length(4),
        Constraint::Length(6),
    ];

    // sessions[filter: count] / sessions[count]
    let title = if app.filter.is_active() {
        format!(
            " Session[{}: {}] ",
            app.filter.describe_with(|id| app.profile_name(id)),
            app.filtered.len()
        )
    } else {
        format!(" Session[{}] ", app.filtered.len())
    };

    let row_highlight_bg = if table_focus {
        th.selection_bg
    } else {
        th.selection_inactive_bg
    };
    let row_highlight_style = if table_dimmed {
        th.soft_dim()
            .bg(row_highlight_bg)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
            .bg(row_highlight_bg)
            .fg(th.selection_fg)
            .add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, widths)
        .style(table_style)
        .header(header)
        .column_spacing(1)
        .block(titled_block_nav(&title, table_focus, true, true, th.accent))
        .row_highlight_style(row_highlight_style)
        // Reserves 1 space to the left of all rows (padding). The highlighted row spans across this space.
        .highlight_symbol(" ");

    // Persists viewport state across frames: updating selection instructs ratatui to scroll minimally,
    // keeping selection visible (cursor moves inside viewport; scrolls only at bounds).
    let mut state: std::cell::RefMut<TableState> = app.table_state.borrow_mut();
    if app.filtered.is_empty() {
        state.select(None);
    } else {
        state.select(Some(app.selected));
    }
    f.render_stateful_widget(table, area, &mut state);
    let offset = state.offset();
    drop(state);
    // viewport = inner height - header (1 row) (area.height - 3 including top/bottom borders).
    let viewport = (area.height as usize).saturating_sub(3);
    draw_vscrollbar(
        f,
        area,
        table_focus,
        offset,
        app.filtered.len(),
        viewport,
        th,
    );
}

/// Profile list table (profiles view only, full width).
fn draw_profile_table(f: &mut Frame, app: &App, area: Rect) {
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

    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 200) as usize)
        .unwrap_or(0);

    let header_style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    // USAGE is a single merged column: formatted internally as `5H(4) RESET(8) 1W(4) RESET(8)` separated by 2 spaces
    // to mimic discrete columns, while allowing status messages like logout to occupy a single line.
    let header = Row::new(vec![
        aligned_cell("#", Alignment::Center),
        aligned_cell("A", Alignment::Center),
        Cell::from("NAME"),
        Cell::from("STATUS"),
        Cell::from(format!(
            "{:>4}  {:^8}  {:>4}  {:^8}",
            "5H", "RESET", "1W", "RESET"
        )),
        aligned_cell("SESSION", Alignment::Right),
        Cell::from("CONFIG DIR"),
    ])
    .style(header_style);

    // Truncates CONFIG DIR column to fill the remaining width.
    // Fixed columns (4 + 4 + 20 + 13 + 30 + 7 = 78) + gaps (6 * 2) + borders (2) + highlight (1) + margin (1).
    let config_dir_w = (area.width as usize).saturating_sub(78 + 12 + 2 + 1 + 1);

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
            // If config directory is missing, render error label inside USAGE cell (width 30)
            // instead of usage stats (reads side-by-side with STATUS "Error" to clearly expose cause).
            let usage_cell = if entry.phase == crate::usage::UsagePhase::MissingDir {
                let style = if subdued {
                    th.soft_dim()
                } else {
                    Style::default().fg(th.error)
                };
                Cell::from(Span::styled(MISSING_DIR_LABEL, style))
            } else {
                profile_usage_cell(entry, tick, subdued, th)
            };
            let mut row = Row::new(vec![
                aligned_cell(Line::from(Span::styled(num, key_style)), Alignment::Center),
                aligned_cell(Line::from(Span::styled(tag, tag_style)), Alignment::Center),
                Cell::from(Span::styled(truncate_w(&p.name, 20), name_style)),
                Cell::from(status_span),
                usage_cell,
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
        Constraint::Length(30), // USAGE column (merged 5H+RESET+1W+RESET)
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
            true,
            th.accent,
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

/// Formats path string, substituting home directory prefix with `~`.
fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            if rest.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", rest.to_string_lossy());
        }
    }
    path.to_string_lossy().into_owned()
}

/// Profile creation/edit form: agent radio options, text inputs (Name, Config Path), and Save/Cancel buttons.
fn draw_profile_form(f: &mut Frame, app: &App) {
    use super::FormFocus;
    use crate::model::Agent;

    let Some(form) = &app.profile_form else {
        return;
    };
    let th = &app.theme;
    let title = if form.editing_id.is_some() {
        " Edit Profile "
    } else {
        " Add Profile "
    };

    // Error notice (error color) or Antigravity limitation banner (muted).
    let notice: Option<(String, Color)> = if let Some(err) = &form.error {
        Some((err.clone(), th.error))
    } else if Agent::all()[form.agent_idx] == Agent::Antigravity {
        Some((
            "Antigravity: config env not supported — usage/resume runs on the default account"
                .to_string(),
            th.muted,
        ))
    } else if !form.builtin && !form.agy_allowed {
        // Since Antigravity cannot be selected during creation (or editing other agents),
        // show this notice permanently to clarify the dim state.
        Some((
            "Antigravity is not selectable — custom config folders are not supported".to_string(),
            th.muted,
        ))
    } else {
        None
    };

    // Allocates an extra row only if error/notice is present, maintaining a unified 1-row padding in dialog.
    let h = if notice.is_some() { 13 } else { 12 };
    let area = centered_fixed_rect(72, h, f.area());
    let block = modal_block(title, th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);

    let mut constraints = vec![
        Constraint::Length(1), // Agent radio buttons
        Constraint::Length(3), // Name
        Constraint::Length(3), // Config Path
    ];
    if notice.is_some() {
        constraints.push(Constraint::Length(1)); // Error / Notice
    }
    constraints.push(Constraint::Length(1)); // Padding spacer
    constraints.push(Constraint::Length(1)); // Buttons

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // Agent radio layout: (•) Claude   ( ) Antigravity   ( ) Codex
    let agent_focused = form.focus == FormFocus::Agent;
    let mut radio = vec![Span::styled(
        "Agent  ",
        if agent_focused {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        },
    )];
    for (i, agent) in Agent::all().iter().enumerate() {
        let selected = i == form.agent_idx;
        let mark = if selected { "(•) " } else { "( ) " };
        let style = if form.builtin || !form.agent_enabled(i) {
            // Dim built-in agents (unchangeable type) or restricted selections (Antigravity).
            th.soft_dim()
        } else if selected && agent_focused {
            Style::default().fg(th.on_accent).bg(th.accent)
        } else if selected {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            th.soft_dim()
        };
        radio.push(Span::styled(
            format!("{}{}", mark, crate::profile::agent_display_name(*agent)),
            style,
        ));
        radio.push(Span::raw("   "));
    }
    f.render_widget(Paragraph::new(Line::from(radio)), rows[0]);

    form_input(
        f,
        rows[1],
        " Name ",
        &form.name,
        form.focus == FormFocus::Name,
        th,
    );
    form_input(
        f,
        rows[2],
        " Config Path ",
        &form.path,
        form.focus == FormFocus::Path,
        th,
    );

    if let Some((text, color)) = &notice {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_w(text, rows[3].width as usize),
                Style::default().fg(*color),
            ))),
            rows[3],
        );
    }

    // Buttons: highlighted only when focused on the button row.
    let buttons_focused = form.focus == FormFocus::Buttons;
    let (focused_style, unfocused) = button_styles(th);
    let (save_style, cancel_style) = if !buttons_focused {
        (unfocused, unfocused)
    } else if form.save_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("   Save   ", save_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    let button_row = if notice.is_some() { rows[5] } else { rows[4] };
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        button_row,
    );
}

/// Single-line input box styled with a label. If focused, renders a Thick/accent border and displays hardware cursor.
fn form_input(
    f: &mut Frame,
    area: Rect,
    label: &str,
    input: &TextInput,
    focused: bool,
    th: &Theme,
) {
    let (border_type, style) = if focused {
        (
            BorderType::Thick,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )
    } else {
        (BorderType::Plain, Style::default().fg(th.dim))
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(style)
        .title(Span::styled(label.to_string(), style))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let (visible, cursor_x) = input_view(input, inner.width as usize);
    f.render_widget(Paragraph::new(visible), inner);
    if focused {
        f.set_cursor_position((inner.x.saturating_add(cursor_x), inner.y));
    }
}

/// Profile deletion confirmation modal. Explicitly warns that the actual folder is preserved on disk.
fn draw_profile_delete_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let (name, path, count) = app
        .pending_profile_delete
        .and_then(|idx| app.profiles.profiles.get(idx))
        .map(|p| {
            let count = app.sessions.iter().filter(|s| s.profile_id == p.id).count();
            (p.name.clone(), display_path(&p.path), count)
        })
        .unwrap_or_else(|| ("?".to_string(), "?".to_string(), 0));

    let area = centered_fixed_rect(70, 11, f.area());
    let block = modal_block(" Delete Profile ", th.error).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let content = vec![
        Line::from(Span::styled(
            truncate_w(&name, inner_w),
            Style::default().fg(th.error).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_w(&format!("path: {}", path), inner_w),
            Style::default().fg(th.dim),
        )),
        Line::from(Span::styled(
            format!("sessions: {}", count),
            Style::default().fg(th.dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "The folder on disk is NOT deleted. Sessions are only removed from the list.",
            th.soft_dim(),
        )),
    ];

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (cancel_style, delete_style) = if app.delete_ok_focused {
        (unfocused, focused_style)
    } else {
        (focused_style, unfocused)
    };
    let buttons = Line::from(vec![
        Span::styled("  Delete  ", delete_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

/// Modal verifying creation of a missing config directory (and execution of login subprocess) upon profile save.
fn draw_profile_dir_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let (path_str, agent, login) = app
        .profile_form
        .as_ref()
        .map(|form| {
            let agent = crate::model::Agent::all()
                [form.agent_idx.min(crate::model::Agent::all().len() - 1)];
            let path = crate::config::expand(form.path.value.trim());
            let login = crate::profile::login_runnable(agent, &path);
            (display_path(&path), agent, login)
        })
        .unwrap_or(("?".to_string(), crate::model::Agent::Claude, false));

    let area = centered_fixed_rect(70, 12, f.area());
    let block = modal_block(" Create Config Folder ", th.warning).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let mut content = vec![
        Line::from(Span::styled(
            truncate_w(&path_str, inner_w),
            Style::default().fg(th.warning).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "This folder does not exist. Create it now?",
            th.soft_dim(),
        )),
        Line::from(""),
    ];
    if login {
        content.push(Line::from(Span::styled(
            format!(
                "{} will launch for login after the folder is created.",
                agent.label()
            ),
            th.soft_dim(),
        )));
        content.push(Line::from(Span::styled(
            "Log in, then exit the agent to return to s7s.",
            Style::default().fg(th.dim),
        )));
    } else {
        content.push(Line::from(Span::styled(
            "Only the folder will be created. Antigravity does not support custom config folders — log in manually.",
            th.soft_dim(),
        )));
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (ok_style, cancel_style) = if app.dir_create_ok_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("  Create  ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

/// Confirmation modal for creating a new project folder under `config::projects_dir()`
/// when the New Session folder input is a bare name with no existing folder.
/// Create makes the folder and starts the session; Cancel returns to the dialog.
fn draw_project_dir_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let path_str = app
        .project_dir_pending
        .as_ref()
        .map(|p| display_path(p))
        .unwrap_or_else(|| "?".to_string());

    let area = centered_fixed_rect(70, 8, f.area());
    let block =
        modal_block(" Create Project Folder ", th.warning).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let content = vec![
        Line::from(Span::styled(
            truncate_w(&path_str, inner_w),
            Style::default().fg(th.warning).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "This folder does not exist. Create it and start the session?",
            th.soft_dim(),
        )),
    ];

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (ok_style, cancel_style) = if app.dir_create_ok_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("  Create  ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

/// `:`/`!` Quick Command window modal (command palette / terminal command mode).
/// Width is around 120 cells (downscaled on smaller terminals),
/// height scales dynamically up to 10 lines of matched results. Confirms options via Enter, exits via Esc.
/// Matches are sorted with enabled items first; disabled entries are visually dimmed.
fn draw_quick_command(f: &mut Frame, app: &App) {
    use super::quick::{QuickMode, VIEWPORT};

    let Some(state) = app.quick.as_ref() else {
        return;
    };
    let th = &app.theme;
    let terminal = state.mode == QuickMode::Terminal;
    let total = if terminal {
        state.term_items.len()
    } else {
        state.items.len()
    };
    let view = total.clamp(1, VIEWPORT);

    // Inner height segments: input (1) + divider (1) + matches viewport (view) + divider (1) + description (1).
    // Outer height = inner + borders (2). Outer width = content width 120
    // + margins (2) + borders (2) + horizontal padding (2) = 126 (downscaled on narrow terminals).
    let outer_h = (view as u16 + 6).min(f.area().height);
    let outer_w = 126.min(f.area().width);
    // Vertical centering on initial open only. Fixes top to prevent vertical shifting
    // when search dynamic results change the modal height.
    let centered = centered_fixed_rect(outer_w, outer_h, f.area());
    let top = match state.anchor_y.get() {
        Some(y) => y.min(f.area().height.saturating_sub(outer_h)),
        None => {
            state.anchor_y.set(Some(centered.y));
            centered.y
        }
    };
    let area = Rect::new(centered.x, top, centered.width, outer_h);
    let title = if terminal {
        " Terminal Command "
    } else {
        " Quick Command "
    };
    let block = modal_block(title, th.accent).padding(Padding::horizontal(1));
    let inner = render_modal(f, area, block, th);
    if inner.height < 5 || inner.width < 4 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),           // Search input
            Constraint::Length(1),           // Divider
            Constraint::Length(view as u16), // Command list
            Constraint::Length(1),           // Divider
            Constraint::Length(1),           // Selected item description
        ])
        .split(inner);

    // Horizontal thin borders above/below the list view. Connects ends to thick vertical borders
    // by overwriting lateral margin cells with joint characters `┠` and `┨`.
    let border_style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    let border_l = area.x + 1;
    let border_r = area.x + area.width.saturating_sub(2);
    for idx in [1, 3] {
        let sep = Rect::new(
            rows[idx].x.saturating_sub(1),
            rows[idx].y,
            rows[idx].width.saturating_add(2),
            rows[idx].height,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(sep.width as usize),
                Style::default().fg(th.dim),
            ))),
            sep,
        );
        let buf = f.buffer_mut();
        buf[(border_l, sep.y)]
            .set_symbol("┠")
            .set_style(border_style);
        buf[(border_r, sep.y)]
            .set_symbol("┨")
            .set_style(border_style);
    }

    // Input line: the prompt character mirrors the mode key (`:` palette / `!` terminal)
    // + hardware terminal cursor rendering.
    let prompt = if terminal { "! " } else { ": " };
    let input_w = (rows[0].width as usize).saturating_sub(prompt.len());
    let (visible, cursor_x) = input_view(&state.input, input_w);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                prompt,
                Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(visible),
        ])),
        rows[0],
    );
    f.set_cursor_position((rows[0].x + prompt.len() as u16 + cursor_x, rows[0].y));

    // Command matches (viewport scroll and cursor tracking managed by key handlers).
    // Expands left margin by 1 cell to fill background colors up to the left border padding.
    // Right margin remains empty to prevent visual overlap between highlighted rows and the scrollbar track.
    // Starting text column preserved via leading whitespace (matching highlight background colors).
    let list_area = Rect::new(
        rows[2].x.saturating_sub(1),
        rows[2].y,
        rows[2].width.saturating_add(1),
        rows[2].height,
    );
    let w = list_area.width as usize;
    let lines: Vec<ListItem> = if total == 0 {
        let msg = if terminal {
            " No command history"
        } else {
            " No matching commands"
        };
        vec![ListItem::new(Line::from(msg)).style(th.soft_dim())]
    } else if terminal {
        // Terminal mode: history commands (no shortcut column). The selected row is the
        // recall highlight; None keeps focus on the input line with no highlighted row.
        state
            .term_items
            .iter()
            .enumerate()
            .skip(state.scroll)
            .take(view)
            .map(|(i, cmd)| {
                let style = if state.term_selected == Some(i) {
                    Style::default().fg(th.selection_fg).bg(th.selection_bg)
                } else {
                    Style::default()
                };
                let label = truncate_w(&format!(" {cmd}"), w);
                let pad = w.saturating_sub(label.len());
                ListItem::new(Line::from(vec![
                    Span::styled(label, style),
                    Span::styled(" ".repeat(pad), style),
                ]))
            })
            .collect()
    } else {
        state
            .items
            .iter()
            .enumerate()
            .skip(state.scroll)
            .take(view)
            .map(|(i, item)| {
                let spec = item.spec();
                let selected = i == state.cursor;
                let (label_style, sc_style) = match (selected, item.enabled) {
                    (true, true) => {
                        let s = Style::default().fg(th.selection_fg).bg(th.selection_bg);
                        (s.add_modifier(Modifier::BOLD), s)
                    }
                    (true, false) => {
                        let s = th.soft_dim().add_modifier(Modifier::REVERSED);
                        (s, s)
                    }
                    // Normal state: bold text for commands, soft dim style for shortcuts.
                    (false, true) => (Style::default().add_modifier(Modifier::BOLD), th.soft_dim()),
                    (false, false) => (th.soft_dim(), th.soft_dim()),
                };
                let label = format!(" {}", spec.label);
                // Keyboard shortcuts are right-aligned (with 1 padding cell within the highlighted area).
                // Residual center width is padded via label_style to color the highlighted row evenly.
                let shortcut = spec
                    .shortcut
                    .map(|sc| format!("({sc}) "))
                    .unwrap_or_default();
                let pad = w.saturating_sub(label.len() + shortcut.len());
                ListItem::new(Line::from(vec![
                    Span::styled(label, label_style),
                    Span::styled(" ".repeat(pad), label_style),
                    Span::styled(shortcut, sc_style),
                ]))
            })
            .collect()
    };
    f.render_widget(List::new(lines), list_area);

    // Description footer. Terminal mode always shows the execution folder; palette mode
    // details the reason if disabled, or shows command-specific single line help text.
    let desc = if terminal {
        state
            .term_folder
            .as_ref()
            .map(|p| format!("Run in: {}", p.to_string_lossy()))
    } else {
        state
            .items
            .get(state.cursor)
            .map(|item| {
                if item.enabled {
                    item.spec().description.unwrap_or("")
                } else {
                    "Not available in this window"
                }
            })
            .map(str::to_string)
    };
    if let Some(desc) = desc.filter(|d| !d.is_empty()) {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_w(&desc, rows[4].width as usize),
                th.soft_dim(),
            ))),
            rows[4],
        );
    }

    // Renders scrollbar (arrows + thumb) within the matched list segment of the right border
    // if matched elements overflow the viewport height.
    // Passes a virtual area including the top/bottom dividers, positioning the arrows
    // exactly on the first and last rows of the list.
    if total > view {
        let sb = Rect::new(
            area.x,
            rows[2].y.saturating_sub(1),
            area.width.saturating_sub(1),
            view as u16 + 2,
        );
        draw_vscrollbar(f, sb, true, state.scroll, total, view, th);
    }
}

/// Theme selection dialog: built-in + custom themes with a dark/light/custom tag.
/// Moving the cursor applies the theme immediately (the whole frame, including this
/// dialog, re-renders in the previewed theme); Enter commits, Esc reverts. No buttons.
fn draw_theme_select(f: &mut Frame, app: &App) {
    let Some(state) = app.theme_select.as_ref() else {
        return;
    };
    let th = &app.theme;
    const VIEW_MAX: usize = 14;
    // Only the current category (Dark or Light) is listed; Left/Right swaps it.
    let visible = state.visible();
    let total = visible.len();
    let view = total.clamp(1, VIEW_MAX);

    // Height: borders (2) + top padding (1) + list rows + spacer (1) + hint (1).
    let h = (view as u16).saturating_add(5).min(f.area().height);
    let area = centered_fixed_rect(46, h, f.area());
    // Left/Right swap the whole list between Dark and Light; mirror the session
    // table's top-border navigation arrows, showing only the navigable direction:
    // Dark view can only go Right (→ Light), Light view only Left (← Dark).
    let arrow = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    let title = if state.dark_view {
        " Select Theme · Dark "
    } else {
        " Select Theme · Light "
    };
    let mut block = modal_block(title, th.accent).padding(Padding::new(1, 1, 1, 0));
    if !state.dark_view {
        block = block.title_top(Line::from("━ ← ").left_aligned().style(arrow));
    }
    if state.dark_view {
        block = block.title_top(Line::from(" → ━").right_aligned().style(arrow));
    }
    let inner = render_modal(f, area, block, th);
    if inner.height < 3 || inner.width < 8 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(view as u16), // Theme list
            Constraint::Length(1),           // Spacer
            Constraint::Length(1),           // Key hint
        ])
        .split(inner);

    // Viewport scroll: keep the persisted top row, shifting it only when the
    // cursor would leave the window. Moving the selection up within the visible
    // window therefore does not scroll the list.
    let mut offset = state.scroll.get().min(total.saturating_sub(view));
    if state.cursor < offset {
        offset = state.cursor;
    } else if state.cursor >= offset + view {
        offset = state.cursor + 1 - view;
    }
    state.scroll.set(offset);
    let w = rows[0].width as usize;
    let items: Vec<ListItem> = visible
        .iter()
        .map(|&ti| &state.themes[ti])
        .enumerate()
        .skip(offset)
        .take(view)
        .map(|(i, theme)| {
            let tag = if theme.custom {
                "custom"
            } else if theme.dark {
                "dark"
            } else {
                "light"
            };
            let (name_style, tag_style) = if i == state.cursor {
                let s = Style::default().fg(th.selection_fg).bg(th.selection_bg);
                (s.add_modifier(Modifier::BOLD), s)
            } else {
                (Style::default(), th.soft_dim())
            };
            // Tag right-aligned with 1 trailing pad; residual width painted in the row style.
            let label = format!(" {}", truncate_w(&theme.name, w.saturating_sub(10)));
            let tag = format!("{tag} ");
            let pad = w.saturating_sub(label.width() + tag.width());
            ListItem::new(Line::from(vec![
                Span::styled(label, name_style),
                Span::styled(" ".repeat(pad), name_style),
                Span::styled(tag, tag_style),
            ]))
        })
        .collect();
    f.render_widget(List::new(items), rows[0]);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑↓ preview · enter apply · esc cancel",
            th.soft_dim(),
        )))
        .alignment(Alignment::Center),
        rows[2],
    );

    if total > view {
        let sb = Rect::new(
            area.x,
            rows[0].y.saturating_sub(1),
            area.width.saturating_sub(1),
            view as u16 + 2,
        );
        draw_vscrollbar(f, sb, true, offset, total, view, th);
    }
}

/// `?` Help screen modal. Temporary overlay, excluded from main Screen rotation loops.
fn draw_help(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let area = f.area();
    f.render_widget(Clear, area);
    f.render_widget(Block::default().style(th.base_style()), area);

    let title = " Help - Keyboard Shortcuts ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(th.accent).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            title,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);

    let col1 = help_lines(
        &[("GLOBAL", HELP_GLOBAL), ("SESSION LIST", HELP_SESSION)],
        th,
    );
    let col2 = help_lines(
        &[
            ("SESSION DETAIL", HELP_DETAIL),
            ("PROFILE LIST", HELP_PROFILE),
        ],
        th,
    );
    for (area, lines) in [(cols[0], col1), (cols[1], col2)] {
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("<esc>", th.key_style()),
            Span::styled(" Back  ", th.soft_dim()),
            Span::styled("<?>", th.key_style()),
            Span::styled(" Close help", th.soft_dim()),
        ]))
        .alignment(Alignment::Center),
        rows[1],
    );
}

fn help_lines(sections: &[(&str, &[(&str, &str)])], th: &Theme) -> Vec<Line<'static>> {
    // Key column width: tracks maximum width among all keys rendered.
    // Pads to build two left-aligned columns (table structure) separating keys from their labels.
    let key_w = sections
        .iter()
        .flat_map(|(_, items)| items.iter())
        .map(|(key, _)| format!("<{key}>").width())
        .max()
        .unwrap_or(0);

    let mut lines = Vec::new();
    for (si, (title, items)) in sections.iter().enumerate() {
        if si > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            (*title).to_string(),
            Style::default().fg(th.success).add_modifier(Modifier::BOLD),
        )));
        for (key, action) in *items {
            lines.push(Line::from(vec![
                Span::styled(pad_w(&format!("<{key}>"), key_w), th.key_style()),
                Span::raw("  "),
                Span::styled((*action).to_string(), th.soft_dim()),
            ]));
        }
    }
    lines
}

/// Right preview panel: lists sanitized user questions from the selected session.
fn draw_preview(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let preview_focus = app.focus == Focus::Preview && app.mode == UiMode::Table;
    let block = titled_block_nav(" Prompt ", preview_focus, true, true, th.accent);
    let inner_w = area.width.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();
    if let Some(s) = app.current() {
        lines.extend(session_meta_lines(s, inner_w, th, false));
        lines.push(Line::from(Span::styled(
            "─".repeat(inner_w.max(1)),
            Style::default().fg(th.dim),
        )));

        for (idx, turn) in s.user_turns.iter().enumerate() {
            lines.push(Line::from(Span::styled(
                format!("● Q{}", idx + 1),
                Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
            )));
            for display_line in preview_turn_lines(turn) {
                let (raw_line, style) = match display_line {
                    PreviewTurnLine::Content(line) => (line.to_string(), Style::default()),
                    PreviewTurnLine::Omission(count) => (
                        format!("────── ⋯ {count} lines omitted ⋯ ──────"),
                        th.soft_dim().add_modifier(Modifier::DIM),
                    ),
                };
                // Pre-wrap content using width constraints (handles long contiguous words and tab tabstops).
                // Disables Paragraph's native wrapping; uses this pre-computed result as the sole line sequence.
                for wrapped in wrap_w(&raw_line, inner_w) {
                    lines.push(Line::from(Span::styled(wrapped, style)));
                }
            }
            lines.push(Line::from(Span::styled(
                "─".repeat(inner_w.max(1)),
                Style::default().fg(th.dim),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No sessions to show.",
            Style::default().fg(th.dim),
        )));
    }

    // Computes and sets the maximum scroll boundary based on total lines and viewport height.
    // Limits scrolling bounds; if content fits within viewport (lines <= viewport), defaults to 0,
    // which disables scrolling via Up/Down keys.
    let total = lines.len();
    let viewport = (area.height as usize).saturating_sub(2);
    app.preview_max_scroll
        .set(total.saturating_sub(viewport).min(u16::MAX as usize) as u16);

    // Native Paragraph wrapping is disabled as `wrap_w` already handled formatting.
    // (Combining with native wrapping disrupts width metrics, leaking long lines past borders
    // and causing discrepancies in scroll boundaries.) Each `Line` maps exactly to one terminal row.
    let para = Paragraph::new(lines)
        .block(block)
        .scroll((app.preview_scroll, 0));
    f.render_widget(para, area);
    draw_vscrollbar(
        f,
        area,
        preview_focus,
        app.preview_scroll as usize,
        total,
        viewport,
        th,
    );
}

/// Shared helper for preview/detail left panels: constructs 5 session metadata rows (Project, Name, Created, Modified, ID).
/// `dimmed` renders the accent-colored values in soft-dim too (whole panel unfocused).
fn session_meta_lines(
    s: &crate::model::Session,
    inner_w: usize,
    th: &Theme,
    dimmed: bool,
) -> Vec<Line<'static>> {
    let accent_style = if dimmed {
        th.soft_dim().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
    };
    let mut lines: Vec<Line> = Vec::new();
    let project_prefix = "● Project: ";
    let full_path = s.cwd.to_string_lossy().into_owned();
    // Allocates residual width to brackets and full paths: total - prefix - folder - " (" - ")".
    let path_avail = inner_w
        .saturating_sub(project_prefix.width())
        .saturating_sub(s.folder.width())
        .saturating_sub(3);
    lines.push(Line::from(vec![
        Span::styled(project_prefix, th.soft_dim()),
        Span::styled(s.folder.clone(), accent_style),
        Span::styled(
            format!(
                " ({})",
                truncate_w_with_ellipsis(&full_path, path_avail, "...")
            ),
            th.soft_dim(),
        ),
    ]));
    let name_prefix = "● Name: ";
    let name_w = inner_w.saturating_sub(name_prefix.width());
    lines.push(Line::from(vec![
        Span::styled(name_prefix, th.soft_dim()),
        Span::styled(
            truncate_w_with_ellipsis(&s.title(), name_w, "..."),
            accent_style,
        ),
    ]));
    let created_prefix = "● Created at: ";
    lines.push(Line::from(vec![
        Span::styled(created_prefix, th.soft_dim()),
        Span::styled(s.created_str(), th.soft_dim()),
    ]));
    let updated_prefix = "● Updated at: ";
    lines.push(Line::from(vec![
        Span::styled(updated_prefix, th.soft_dim()),
        Span::styled(s.updated_str(), th.soft_dim()),
    ]));
    let (tag, _) = agent_tag(s.agent, th);
    let id_prefix = "● Id: ";
    let id_w = inner_w.saturating_sub(id_prefix.width());
    lines.push(Line::from(vec![
        Span::styled(id_prefix, th.soft_dim()),
        Span::styled(
            truncate_w_with_ellipsis(&format!("[{}] {}", tag.trim(), s.id), id_w, "..."),
            th.soft_dim(),
        ),
    ]));
    lines
}

/// Maximum lines allowed for a single work entry in the right panel (excess is truncated with omission placeholder).
const WORK_ENTRY_MAX_LINES: usize = 120;
/// Maximum lines allowed for the final answer.
const FINAL_ANSWER_MAX_LINES: usize = 400;

/// Session details view: left questions list and right selected question's work/answers panel.
fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let Some(detail) = &app.detail else {
        return;
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(area);
    draw_detail_prompt(f, app, cols[0], detail);
    draw_detail_work(f, app, cols[1], detail);
}

/// Details left column: user questions list formatted identically to search preview.
/// Highlights the selected question with a reversed-background block, matching table row highlights.
fn draw_detail_prompt(f: &mut Frame, app: &App, area: Rect, detail: &SessionDetailState) {
    let th = &app.theme;
    let focused = detail.focus == DetailFocus::Questions && app.mode == UiMode::Table;
    let block = titled_block_nav(" Prompt ", focused, true, true, th.accent);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let inner_w = inner.width as usize;

    let Some(s) = app.sessions.get(detail.session_idx) else {
        return;
    };

    let sep_line = || {
        Line::from(Span::styled(
            "─".repeat(inner_w.max(1)),
            Style::default().fg(th.dim),
        ))
    };

    // Prepares content rows. Captures the selected question's surrounding row indices
    // (separator above/below) so the scroll adjustment keeps the whole block visible.
    // Same dimming rule as the session table: when this panel loses focus (Work panel
    // focused) the whole panel renders soft-dim and the selection switches to the
    // inactive reversed highlight. Selected lines are right-padded so the background
    // fills the entire row width.
    let sel_bg = if focused {
        th.selection_bg
    } else {
        th.selection_inactive_bg
    };
    let mut rows: Vec<Line> = session_meta_lines(s, inner_w, th, !focused);
    rows.push(sep_line());
    let mut sel_top = rows.len().saturating_sub(1);
    let mut sel_bottom = sel_top;
    for (idx, turn) in detail.turns.iter().enumerate() {
        let selected = idx == detail.selected;
        if selected {
            sel_top = rows.len().saturating_sub(1);
        }
        let title_style = if selected {
            if focused {
                Style::default()
                    .bg(sel_bg)
                    .fg(th.selection_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                th.soft_dim()
                    .bg(sel_bg)
                    .add_modifier(Modifier::REVERSED | Modifier::BOLD)
            }
        } else if focused {
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD)
        } else {
            th.soft_dim()
        };
        let title = format!("● Q{}", idx + 1);
        rows.push(Line::from(Span::styled(
            if selected {
                pad_w(&title, inner_w)
            } else {
                title
            },
            title_style,
        )));
        let body_style = if selected {
            if focused {
                Style::default().bg(sel_bg).fg(th.selection_fg)
            } else {
                th.soft_dim().bg(sel_bg).add_modifier(Modifier::REVERSED)
            }
        } else if focused {
            Style::default()
        } else {
            th.soft_dim()
        };
        for display_line in preview_turn_lines(&turn.user) {
            let (raw_line, style) = match display_line {
                PreviewTurnLine::Content(line) => (line.to_string(), body_style),
                PreviewTurnLine::Omission(count) => {
                    let s = th.soft_dim().add_modifier(Modifier::DIM);
                    (
                        format!("────── ⋯ {count} lines omitted ⋯ ──────"),
                        if selected && focused {
                            s.bg(sel_bg)
                        } else if selected {
                            s.bg(sel_bg).add_modifier(Modifier::REVERSED)
                        } else {
                            s
                        },
                    )
                }
            };
            for wrapped in wrap_w(&raw_line, inner_w) {
                rows.push(Line::from(Span::styled(
                    if selected {
                        pad_w(&wrapped, inner_w)
                    } else {
                        wrapped
                    },
                    style,
                )));
            }
        }
        rows.push(sep_line());
        if selected {
            sel_bottom = rows.len().saturating_sub(1);
        }
    }

    // Adjusts scroll offset: shifts viewport bounds to keep the selected question block visible inside inner area.
    let h = inner.height as usize;
    let max_off = rows.len().saturating_sub(h);
    let mut off = (detail.left_scroll.get() as usize).min(max_off);
    if sel_bottom >= off + h {
        off = sel_bottom + 1 - h;
    }
    if sel_top < off {
        off = sel_top;
    }
    let off = off.min(max_off);
    detail.left_scroll.set(off.min(u16::MAX as usize) as u16);

    let total_rows = rows.len();
    let visible: Vec<Line> = rows.into_iter().skip(off).take(h).collect();
    f.render_widget(Paragraph::new(visible), inner);
    draw_vscrollbar(f, area, focused, off, total_rows, h, th);
}

/// Details right column: intermediate agent work entries and the final answer for the selected question.
/// Tool Call/Result entries are hidden by default, toggled via `.` key (`app.detail_show_tools`).
fn draw_detail_work(f: &mut Frame, app: &App, area: Rect, detail: &SessionDetailState) {
    let th = &app.theme;
    let focused = detail.focus == DetailFocus::Work && app.mode == UiMode::Table;
    let title = format!(" Q{} Work & Answer ", detail.selected + 1);
    let block = titled_block_nav(&title, focused, true, false, th.accent);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let wrap_width = inner.width as usize;

    // Collapses contiguous runs of hidden Tool Calls / Results into a single line placeholder.
    let flush_hidden = |lines: &mut Vec<Line>, hidden_run: &mut usize| {
        if *hidden_run == 0 {
            return;
        }
        lines.push(Line::from(Span::styled(
            format!(
                "⋯ {} tool call/result hidden · press <.> to show ⋯",
                hidden_run
            ),
            th.soft_dim().add_modifier(Modifier::DIM),
        )));
        lines.push(Line::from(""));
        *hidden_run = 0;
    };

    let mut lines: Vec<Line> = Vec::new();
    if let Some(turn) = detail.turns.get(detail.selected) {
        if turn.work_entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "No intermediate work extracted for this turn.",
                Style::default().fg(th.dim),
            )));
            lines.push(Line::from(""));
        }
        let mut hidden_run = 0usize;
        for (i, entry) in turn.work_entries.iter().enumerate() {
            if !app.detail_show_tools
                && matches!(entry.kind, WorkKind::ToolCall | WorkKind::ToolResult)
            {
                hidden_run += 1;
                continue;
            }
            flush_hidden(&mut lines, &mut hidden_run);
            let heading_style = match entry.kind {
                WorkKind::AssistantText => th.soft_dim().add_modifier(Modifier::BOLD),
                WorkKind::ToolCall => th.key_style(),
                WorkKind::ToolResult => {
                    Style::default().fg(th.success).add_modifier(Modifier::BOLD)
                }
            };
            lines.push(Line::from(Span::styled(
                format!("● {} {}", entry.kind.heading(), i + 1),
                heading_style,
            )));
            let body_style = match entry.kind {
                WorkKind::AssistantText => Style::default(),
                _ => th.soft_dim(),
            };
            push_capped_lines(
                &mut lines,
                &entry.text,
                wrap_width,
                body_style,
                WORK_ENTRY_MAX_LINES,
                th,
            );
            lines.push(Line::from(""));
        }
        flush_hidden(&mut lines, &mut hidden_run);
        lines.push(Line::from(Span::styled(
            "● Final Answer",
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )));
        match turn.final_answer.as_deref() {
            Some(answer) if !answer.trim().is_empty() => {
                push_capped_lines(
                    &mut lines,
                    answer,
                    wrap_width,
                    Style::default(),
                    FINAL_ANSWER_MAX_LINES,
                    th,
                );
            }
            _ => lines.push(Line::from(Span::styled(
                "No final assistant answer extracted.",
                Style::default().fg(th.dim),
            ))),
        }
    } else {
        lines.push(Line::from(Span::styled(
            "No turn selected.",
            Style::default().fg(th.dim),
        )));
    }

    // Computes and records the maximum scroll boundary at rendering, clamping active scroll offsets.
    let h = inner.height as usize;
    let max_off = lines.len().saturating_sub(h).min(u16::MAX as usize) as u16;
    detail.right_max_scroll.set(max_off);
    let off = detail.right_scroll.get().min(max_off);
    detail.right_scroll.set(off);

    let total_lines = lines.len();
    f.render_widget(Paragraph::new(lines).scroll((off, 0)), inner);
    draw_vscrollbar(f, area, focused, off as usize, total_lines, h, th);
}

/// Wraps multi-line text by width and appends to `out`. Truncates lines exceeding `max_lines`,
/// appending an omission count placeholder to handle large tool execution logs efficiently.
fn push_capped_lines(
    out: &mut Vec<Line<'static>>,
    text: &str,
    wrap_width: usize,
    style: Style,
    max_lines: usize,
    th: &Theme,
) {
    let mut count = 0usize;
    let mut omitted = 0usize;
    for raw in text.lines() {
        let chunks = if raw.is_empty() {
            vec![String::new()]
        } else {
            wrap_w(raw, wrap_width.max(1))
        };
        for chunk in chunks {
            if count >= max_lines {
                omitted += 1;
                continue;
            }
            out.push(Line::from(Span::styled(chunk, style)));
            count += 1;
        }
    }
    if omitted > 0 {
        out.push(Line::from(Span::styled(
            format!("────── ⋯ {omitted} more lines ⋯ ──────"),
            th.soft_dim().add_modifier(Modifier::DIM),
        )));
    }
}

/// Footer status bar: displays keyword queries / filters on the left, scan stats on the right.
fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let th = &app.theme;
    let dim_style = Style::default().fg(th.dim);
    let left: Line = if app.mode == UiMode::Keyword {
        Line::from(Span::styled(" enter confirm  ·  esc cancel ", dim_style))
    } else if app.mode == UiMode::Help {
        Line::from(Span::styled(" esc/q/? close help ", dim_style))
    } else if app.mode == UiMode::ThemeSelect {
        Line::from(Span::styled(
            " ↑↓ preview  ·  enter apply  ·  esc cancel ",
            dim_style,
        ))
    } else if let Some(msg) = &app.status_msg {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                format!(" {} ", msg),
                Style::default().fg(th.on_accent).bg(th.accent),
            ),
        ])
    } else if app.screen == Screen::Detail {
        Line::from(Span::styled(" . toggle tool logs  ·  ← back ", dim_style))
    } else if app.mode == UiMode::NewSession {
        Line::from(Span::styled(
            " enter open/select  ·  ↑↓ move focus  ·  tab focus  ·  space select  ·  → complete  ·  esc close ",
            dim_style,
        ))
    } else if app.mode == UiMode::Rename {
        Line::from(Span::styled(" enter save  ·  esc cancel ", dim_style))
    } else if app.filter.is_active() {
        Line::from(vec![
            Span::styled(" filter: ", Style::default().fg(th.on_accent).bg(th.accent)),
            Span::styled(
                format!(" {}", app.filter.describe_with(|id| app.profile_name(id))),
                th.soft_dim(),
            ),
        ])
    } else {
        Line::from(Span::styled(" q/ctrl+c quit ", dim_style))
    };

    f.render_widget(Paragraph::new(left), area);
}

/// Agent multi-select modal dialog. Uses consistent styling matching deletion confirm dialog.
fn draw_agent_modal(f: &mut Frame, app: &App) {
    let Some(m) = &app.agent_modal else {
        return;
    };
    let th = &app.theme;
    // content height = number of items + borders (2) + top/bottom padding (2).
    let h = (m.labels.len() as u16) + 4;
    let area = centered_fixed_rect(44, h, f.area());
    let inner = render_modal(f, area, modal_block(" Select Agents ", th.accent), th);
    let inner_w = inner.width as usize;

    let items: Vec<ListItem> = m
        .labels
        .iter()
        .enumerate()
        .map(|(i, label)| modal_list_item(i, label, m, inner_w, th))
        .collect();
    f.render_widget(List::new(items), inner);
}

/// Folder multi-select modal dialog (keyword search + folder list). Consistent styling matching deletion confirm dialog.
fn draw_folder_modal(f: &mut Frame, app: &App) {
    let Some(m) = &app.folder_modal else {
        return;
    };
    let th = &app.theme;
    let full = f.area();
    // Width 86, capped at 80% of the screen on narrow terminals.
    let w = 86.min(full.width * 8 / 10);
    let h = 20u16.min(full.height);
    let area = centered_fixed_rect(w, h, full);
    let title = format!(" Select Folders ({}) ", m.labels.len());
    // No vertical padding: the input sits directly under the top border and the
    // list runs to the bottom border (no spacer/footer rows).
    let block = modal_block(&title, th.accent).padding(Padding::horizontal(1));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    // Inner segments: search input (1 row) + divider + list + divider + footer.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let query = if app.folder_query.is_empty() {
        Span::styled("(type to filter folders)", Style::default().fg(th.dim))
    } else {
        Span::raw(format!("{}▏", app.folder_query))
    };
    f.render_widget(Paragraph::new(Line::from(query)), rows[0]);

    // Thin horizontal dividers below the input and above the footer, ends joined
    // to the thick vertical borders with `┠`/`┨` (same styling as Quick Command).
    let border_style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    for idx in [1, 3] {
        let sep = Rect::new(
            rows[idx].x.saturating_sub(1),
            rows[idx].y,
            rows[idx].width.saturating_add(2),
            rows[idx].height,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(sep.width as usize),
                Style::default().fg(th.dim),
            ))),
            sep,
        );
        let buf = f.buffer_mut();
        buf[(area.x + 1, sep.y)]
            .set_symbol("┠")
            .set_style(border_style);
        buf[(area.x + area.width.saturating_sub(2), sep.y)]
            .set_symbol("┨")
            .set_style(border_style);
    }

    // Footer: full path of the focused folder in soft dim. Folder names are cwd
    // basenames, so resolve through the first session carrying that folder name.
    let focus_path = m.labels.get(m.cursor).and_then(|name| {
        app.sessions
            .iter()
            .find(|s| &s.folder == name)
            .map(|s| s.cwd.to_string_lossy().into_owned())
    });
    if let Some(path) = focus_path {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_w(&path, rows[4].width as usize),
                th.soft_dim(),
            ))),
            rows[4],
        );
    }

    // Viewport scroll: keep the persisted top row, shifting it only when the
    // cursor would leave the window. Moving the selection up within the visible
    // window therefore does not scroll the list.
    let list_h = rows[2].height as usize;
    let total = m.labels.len();
    let mut offset = m.scroll.get().min(total.saturating_sub(list_h));
    if m.cursor < offset {
        offset = m.cursor;
    } else if list_h > 0 && m.cursor >= offset + list_h {
        offset = m.cursor + 1 - list_h;
    }
    m.scroll.set(offset);
    let items: Vec<ListItem> = m
        .labels
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_h)
        .map(|(i, label)| modal_list_item(i, label, m, inner_w, th))
        .collect();
    f.render_widget(List::new(items), rows[2]);

    if total > list_h {
        let sb = Rect::new(
            area.x,
            rows[2].y.saturating_sub(1),
            area.width.saturating_sub(1),
            list_h as u16 + 2,
        );
        draw_vscrollbar(f, sb, true, offset, total, list_h, th);
    }
}

/// New session creation dialog (read-only source plus profile, model, and folder dropdown controls).
///
/// Renders focused controls with highlighted thick borders. Expanded dropdown overlays
/// directly below the respective control (only one dropdown may open at a time).
/// Folder list draws query-matching entries in normal text at the top, and unmatched entries
/// in soft dim (matching "left" label color) at the bottom.
/// Title of the shared New Session dialog. Context details are rendered in the
/// modal body so the outer title remains stable at narrow terminal widths.
fn new_session_title(state: &super::NewSessionState) -> &'static str {
    if state.context.is_some() {
        " New Session with Context "
    } else {
        " New Session "
    }
}

/// Source session title for the read-only context control.
fn new_session_source_title(state: &super::NewSessionState, inner_w: usize) -> Option<String> {
    state
        .context
        .as_ref()
        .map(|ctx| truncate_w(&ctx.title, inner_w))
}

/// Desired width is 102 cells (20 wider than the original dialog), capped at
/// 80% of the current terminal width.
fn new_session_modal_width(screen_w: u16) -> u16 {
    102.min(screen_w.saturating_mul(4) / 5)
}

fn draw_new_session_modal(f: &mut Frame, app: &App) {
    use super::NewSessionFocus;
    let Some(state) = &app.new_session else {
        return;
    };
    let th = &app.theme;
    let full = f.area();
    let w = new_session_modal_width(full.width);
    let profile_count = app.profiles.profiles.len();

    let profile_open = state.dropdown_open && state.focus == NewSessionFocus::Profile;
    let model_open = state.dropdown_open && state.focus == NewSessionFocus::Model;

    // Contextual dialogs add one read-only source control above the editable controls.
    // The ordinary dialog remains 14 rows high and does not reserve an empty row.
    // Dropdown options overlay directly on top without shifting the main dialog borders (see overlay section at end of function).
    let source_h = if state.context.is_some() { 3 } else { 0 };
    let h = (14 + source_h).min(full.height);
    let area = centered_fixed_rect(w, h, full);

    let block = modal_block(new_session_title(state), th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(source_h), // Read-only source control (contextual only)
            Constraint::Length(3),        // Profile combo box
            Constraint::Length(3),        // Model combo box
            Constraint::Length(3),        // Folder combo box
            Constraint::Length(1),        // Spacer
            Constraint::Length(1),        // OK/Cancel button row
        ])
        .split(inner);

    // ---- Source Session (read-only, dimmed, and excluded from focus navigation) ----
    if state.context.is_some() {
        let source_style = th.soft_dim();
        let source_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(source_style)
            .title(Span::styled(" Context Source ", source_style))
            .padding(Padding::horizontal(1));
        let source_inner = source_block.inner(rows[0]);
        f.render_widget(source_block, rows[0]);
        if let Some(title) = new_session_source_title(state, source_inner.width as usize) {
            f.render_widget(
                Paragraph::new(Span::styled(title, source_style)),
                source_inner,
            );
        }
    }

    // Combo box border styles depending on focus.
    let combo_block = |title: &str, focused: bool| {
        let (style, border_type) = if focused {
            (
                Style::default().add_modifier(Modifier::BOLD),
                BorderType::Thick,
            )
        } else {
            (Style::default().fg(th.dim), BorderType::Plain)
        };
        Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(style)
            .title(Span::styled(title.to_string(), style))
            .padding(Padding::horizontal(1))
    };

    // ---- Profile Combo Box (text inputs not supported) ----
    let profile_focused = state.focus == NewSessionFocus::Profile;
    let profile_block = combo_block(" Profile ▾ ", profile_focused);
    let profile_inner = profile_block.inner(rows[1]);
    f.render_widget(profile_block, rows[1]);
    if let Some(p) = app.profiles.profiles.get(state.profile_idx) {
        let mut spans = vec![Span::styled(
            format!("{} / {}  ", p.agent.label(), p.name),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        spans.extend(usage_spans(app.usage.entry(&p.id), th));
        f.render_widget(Paragraph::new(Line::from(spans)), profile_inner);
    }

    // ---- Model Combo Box (text inputs not supported) ----
    let model_focused = state.focus == NewSessionFocus::Model;
    let model_block = combo_block(" Model ▾ ", model_focused);
    let model_inner = model_block.inner(rows[2]);
    f.render_widget(model_block, rows[2]);
    if let Some(opt) = state.model_options.get(state.model_idx) {
        let label_style = if opt.missing {
            // Default configured model absent from retrieved options: triggers red alert and disables OK button.
            Style::default().fg(th.error).add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        let inner_w = model_inner.width as usize;
        let label_txt = truncate_w(&opt.label, inner_w);
        let used = label_txt.width() + 2;
        let mut spans = vec![Span::styled(format!("{label_txt}  "), label_style)];
        if !opt.note.is_empty() && inner_w > used {
            spans.push(Span::styled(
                truncate_w(&opt.note, inner_w - used),
                th.soft_dim(),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), model_inner);
    }

    // ---- Folder Combo Box (supports text inputs) ----
    let folder_focused = state.focus == NewSessionFocus::Folder;
    let folder_block = combo_block(" Folder ▾ ", folder_focused);
    let input_inner = folder_block.inner(rows[3]);
    f.render_widget(folder_block, rows[3]);
    if state.input.value.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "enter path directly, e.g. ~/DevSpace/project",
                th.soft_dim(),
            ))),
            input_inner,
        );
        if folder_focused {
            f.set_cursor_position((input_inner.x, input_inner.y));
        }
    } else {
        let (visible, cursor_x) = input_view(&state.input, input_inner.width as usize);
        f.render_widget(Paragraph::new(visible), input_inner);
        if folder_focused {
            f.set_cursor_position((input_inner.x.saturating_add(cursor_x), input_inner.y));
        }
    }

    // Buttons row: centered OK/Cancel options. Focused buttons render in bright blue boxes;
    // unfocused buttons render in light gray boxes with gray dim text (matching "left" labels).
    // Errors are displayed on the left side.
    let buttons_focused = state.focus == NewSessionFocus::Buttons;
    // When a missing model (not present in available lists) is selected, OK is disabled.
    // Focus styling is ignored, and confirming via Enter yields error messages.
    let ok_disabled = state
        .model_options
        .get(state.model_idx)
        .is_some_and(|o| o.missing);
    let (focused_style, unfocused) = button_styles(th);
    let (cancel_style, ok_style) = if !buttons_focused {
        (unfocused, unfocused)
    } else if state.ok_focused {
        (
            unfocused,
            if ok_disabled {
                unfocused
            } else {
                focused_style
            },
        )
    } else {
        (focused_style, unfocused)
    };
    let buttons = Line::from(vec![
        Span::styled("    OK    ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[5],
    );
    if let Some(err) = &state.error {
        // Truncate errors to not overlap with the centered button block (width 25).
        let err_w = (rows[5].width as usize).saturating_sub(25) / 2;
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                truncate_w(err, err_w.saturating_sub(1)),
                Style::default().fg(th.error),
            ))),
            rows[5],
        );
    }

    // ---- Dropdown Popup Overlay ----
    // Overlays the open dropdown list directly beneath the active combo box while keeping
    // dialog boundaries fixed. Drawing last overlays elements on top of buttons/status bars
    // and allows extending past dialog frames to screen bottoms (clamped with scroll tracking).
    if state.dropdown_open {
        let anchor = if profile_open {
            rows[1]
        } else if model_open {
            rows[2]
        } else {
            rows[3]
        };
        // Overlap by 1 row with the combo box bottom border to visually join control and list.
        let popup_y = anchor.y.saturating_add(anchor.height).saturating_sub(1);
        let avail = full.height.saturating_sub(popup_y) as usize;
        let total = if profile_open {
            profile_count.max(1)
        } else if model_open {
            state.model_options.len().max(1)
        } else {
            state.ordered.len().max(1)
        };
        let popup_h = (total + 2).min(avail); // Includes 2 rows for popup borders
        if popup_h < 3 {
            // Suppress popup rendering if terminal height is extremely constrained.
            return;
        }
        let popup_rect = Rect {
            x: anchor.x,
            y: popup_y,
            width: anchor.width,
            height: popup_h as u16,
        };
        f.render_widget(Clear, popup_rect);
        f.render_widget(Block::default().style(th.base_style()), popup_rect);
        // Active dropdown combo boxes are focused (thick borders);
        // style popup frames in thick borders to join lines seamlessly.
        let popup_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().add_modifier(Modifier::BOLD));
        let popup_inner = popup_block.inner(popup_rect);
        f.render_widget(popup_block, popup_rect);
        // Overwrite top border with joint characters (e.g. `┣━┫`) to merge with combo frames (same technique as details view).
        let join_w = popup_rect.width as usize;
        let join_line = if join_w > 2 {
            format!("┣{}┫", "━".repeat(join_w - 2))
        } else {
            "━━".to_string()
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                join_line,
                Style::default().add_modifier(Modifier::BOLD),
            ))),
            Rect {
                x: popup_rect.x,
                y: popup_rect.y,
                width: popup_rect.width,
                height: 1,
            },
        );

        let list_h = popup_inner.height as usize;
        let inner_w = popup_inner.width as usize;
        let cursor = if profile_open {
            state.profile_cursor
        } else if model_open {
            state.model_cursor
        } else {
            state.folder_cursor.unwrap_or(0)
        };
        let offset = if cursor >= list_h {
            cursor + 1 - list_h
        } else {
            0
        };

        let items: Vec<ListItem> = if profile_open {
            // Profile list: usage shown next to names, ● indicating committed selection, highlight background indicates cursor focus.
            let name_w = app
                .profiles
                .profiles
                .iter()
                .map(|p| format!("{} / {}", p.agent.label(), p.name).width())
                .max()
                .unwrap_or(1);
            app.profiles
                .profiles
                .iter()
                .enumerate()
                .skip(offset)
                .take(list_h)
                .map(|(i, p)| {
                    let selected = i == state.profile_idx;
                    let mark = if selected { "●" } else { "○" };
                    let label = pad_w(&format!("{} / {}", p.agent.label(), p.name), name_w);
                    let mut spans = vec![
                        Span::styled(
                            format!(" {mark} "),
                            if selected {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                th.soft_dim()
                            },
                        ),
                        Span::styled(format!("{label}  "), Style::default()),
                    ];
                    spans.extend(usage_spans(app.usage.entry(&p.id), th));
                    let item = ListItem::new(Line::from(spans));
                    if i == state.profile_cursor {
                        item.style(Style::default().fg(th.selection_fg).bg(th.selection_bg))
                    } else {
                        item
                    }
                })
                .collect()
        } else if model_open {
            // Model list: ● indicating committed selection, label + description in soft dim, missing model in red.
            let label_w = state
                .model_options
                .iter()
                .map(|o| o.label.width())
                .max()
                .unwrap_or(1);
            state
                .model_options
                .iter()
                .enumerate()
                .skip(offset)
                .take(list_h)
                .map(|(i, opt)| {
                    let selected = i == state.model_idx;
                    let mark = if selected { "●" } else { "○" };
                    let label = pad_w(&opt.label, label_w);
                    let label_style = if opt.missing {
                        Style::default().fg(th.error)
                    } else {
                        Style::default()
                    };
                    let mut spans = vec![
                        Span::styled(
                            format!(" {mark} "),
                            if selected {
                                Style::default().add_modifier(Modifier::BOLD)
                            } else {
                                th.soft_dim()
                            },
                        ),
                        Span::styled(format!("{label}  "), label_style),
                    ];
                    if !opt.note.is_empty() {
                        let used = 3 + label_w + 2;
                        spans.push(Span::styled(
                            truncate_w(&opt.note, inner_w.saturating_sub(used)),
                            th.soft_dim(),
                        ));
                    }
                    let item = ListItem::new(Line::from(spans));
                    if i == state.model_cursor {
                        item.style(Style::default().fg(th.selection_fg).bg(th.selection_bg))
                    } else {
                        item
                    }
                })
                .collect()
        } else {
            // Folder list: query-matched (top, normal color) / unmatched (bottom, soft dim color).
            state
                .ordered
                .iter()
                .enumerate()
                .skip(offset)
                .take(list_h)
                .filter_map(|(pos, &folder_i)| state.folders.get(folder_i).map(|p| (pos, p)))
                .map(|(pos, path)| {
                    let label = folder_name_only(path);
                    let text = format!(" {} ", truncate_w(&label, inner_w.saturating_sub(2)));
                    let style = if state.folder_cursor == Some(pos) {
                        Style::default().fg(th.selection_fg).bg(th.selection_bg)
                    } else if pos < state.match_count {
                        Style::default()
                    } else {
                        th.soft_dim()
                    };
                    ListItem::new(Line::from(text)).style(style)
                })
                .collect()
        };
        f.render_widget(List::new(items), popup_inner);
    }
}

fn folder_name_only(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Individual modal list item: checkbox mark + label (truncated to inner width) + cursor highlight.
fn modal_list_item<'a>(
    i: usize,
    label: &str,
    m: &super::ModalState,
    inner_w: usize,
    th: &Theme,
) -> ListItem<'a> {
    let mark = if m.selected.contains(&i) {
        "[x]"
    } else {
        "[ ]"
    };
    let text = truncate_w(&format!("{} {}", mark, label), inner_w);
    let style = if i == m.cursor {
        Style::default().fg(th.selection_fg).bg(th.selection_bg)
    } else {
        Style::default()
    };
    ListItem::new(Line::from(text)).style(style)
}

/// Deletion confirmation modal. Renders centered title + 1 space margins/padding,
/// fixed height based on content lines, and Tab-navigated centered button row.
fn draw_delete_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let (agent, title, cwd, id) = app
        .pending_delete
        .and_then(|idx| app.sessions.get(idx))
        .map(|s| {
            (
                s.agent.label().to_string(),
                s.title(),
                s.cwd.to_string_lossy().to_string(),
                s.id.clone(),
            )
        })
        .unwrap_or_else(|| {
            (
                "?".to_string(),
                "?".to_string(),
                "?".to_string(),
                "?".to_string(),
            )
        });

    // Content rows: title/cwd/id + spacer + notice = 5 lines; spacer + buttons = 2 lines.
    // Height calculation: borders (2) + padding (2) + 5 + 2 = 11.
    let area = centered_fixed_rect(70, 10, f.area());
    let block = modal_block(" Delete Session ", th.error).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    // Truncates text by inner width boundaries to prevent double-width characters from clipping the border.
    let prefix_w = 1 + agent.width() + 2; // "[" + agent + "] "
    let content = vec![
        Line::from(vec![
            Span::styled("[", Style::default().fg(th.dim)),
            Span::styled(
                agent,
                Style::default().fg(th.error).add_modifier(Modifier::BOLD),
            ),
            Span::styled("] ", Style::default().fg(th.dim)),
            Span::raw(truncate_w(&title, inner_w.saturating_sub(prefix_w))),
        ]),
        Line::from(Span::styled(
            truncate_w(&format!("cwd: {}", cwd), inner_w),
            Style::default().fg(th.dim),
        )),
        Line::from(Span::styled(
            truncate_w(&format!("id: {}", id), inner_w),
            Style::default().fg(th.dim),
        )),
        Line::from(""),
        Line::from(Span::styled("This action cannot be undone.", th.soft_dim())),
    ];

    // Splits inner layout: content (Min(0)) + spacer (1 row) + buttons (1 row).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    f.render_widget(Paragraph::new(content), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (cancel_style, delete_style) = if app.delete_ok_focused {
        (unfocused, focused_style)
    } else {
        (focused_style, unfocused)
    };
    let buttons = Line::from(vec![
        Span::styled("  Delete  ", delete_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}

/// Generic message alert dialog. Adapts highlight borders depending on severity,
/// wraps body strings to inner widths, and draws centered confirmation "OK" button.
fn draw_message_modal(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let Some(m) = &app.message else {
        return;
    };
    let color = match m.kind {
        MessageKind::Info => th.accent,
        MessageKind::Warn => th.warning,
        MessageKind::Error => th.error,
    };

    let full = f.area();
    // Width calculation: longest body/title line + paddings (margins 2 + borders 2 + padding 2 = 6). Clamped to 34..80.
    let content_w = m
        .lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(0)
        .max(m.title.width());
    let w = (content_w as u16)
        .saturating_add(6)
        .clamp(34.min(full.width), full.width.min(80));
    // Inner area width = outer width - margins (2) - borders (2) - padding (2).
    let inner_w = (w as usize).saturating_sub(6);

    // Wraps body text based on inner width constraints (handles long path strings). Empty strings remain as empty rows.
    let mut body: Vec<Line> = Vec::new();
    for line in &m.lines {
        if line.is_empty() {
            body.push(Line::from(""));
        } else {
            for wrapped in wrap_w(line, inner_w) {
                body.push(Line::from(Span::raw(wrapped)));
            }
        }
    }

    // Height calculation: borders (2) + padding (2) + body + spacer (1) + button (1).
    let h = (body.len() as u16).saturating_add(6).min(full.height);
    let area = centered_fixed_rect(w, h, full);
    let inner = render_modal(f, area, modal_block(&m.title, color), th);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    f.render_widget(Paragraph::new(body), rows[0]);

    let button = Line::from(Span::styled(
        "  OK  ",
        Style::default()
            .fg(crate::theme::contrast_fg(color))
            .bg(color)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(Paragraph::new(button).alignment(Alignment::Center), rows[1]);
}

/// Session rename modal dialog.
fn draw_rename_modal(f: &mut Frame, app: &App) {
    use super::RenameFocus;

    let th = &app.theme;
    let Some(state) = app.rename_modal.as_ref() else {
        return;
    };
    // Target session captured when opening the modal (independent of search selection, shared by details screen).
    let (agent, id) = app
        .rename_target
        .and_then(|idx| app.sessions.get(idx))
        .map(|s| (s.agent.label().to_string(), s.id.clone()))
        .unwrap_or_else(|| ("?".to_string(), "?".to_string()));

    let area = centered_fixed_rect(72, 11, f.area());
    let block = modal_block(" Rename Session ", th.accent).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Agent / ID row
            Constraint::Length(1), // Spacer
            Constraint::Length(3), // Input box
            Constraint::Length(2), // Margin padding
            Constraint::Length(1), // Buttons
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("[{}] {}", agent, id),
            Style::default().fg(th.dim),
        ))),
        rows[0],
    );
    f.render_widget(Paragraph::new(""), rows[1]);

    let input_focused = state.focus == RenameFocus::Input;
    let (border_type, style) = if input_focused {
        (
            BorderType::Thick,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )
    } else {
        (BorderType::Plain, Style::default().fg(th.dim))
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(style)
        .padding(Padding::horizontal(1));
    let input_inner = input_block.inner(rows[2]);
    f.render_widget(input_block, rows[2]);
    let (visible, cursor_x) = input_view(&state.input, input_inner.width as usize);
    f.render_widget(Paragraph::new(visible), input_inner);
    if input_focused {
        f.set_cursor_position((input_inner.x.saturating_add(cursor_x), input_inner.y));
    }

    // Buttons
    let buttons_focused = state.focus == RenameFocus::Buttons;
    let (focused_style, unfocused) = button_styles(th);
    let (ok_style, cancel_style) = if !buttons_focused {
        (unfocused, unfocused)
    } else if state.ok_focused {
        (focused_style, unfocused)
    } else {
        (unfocused, focused_style)
    };
    let buttons = Line::from(vec![
        Span::styled("    OK    ", ok_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[4],
    );
}

/// Returns the horizontal viewport containing the cursor without mutating the input text.
///
/// Since the terminal hardware cursor overlays the text, no placeholder characters are
/// inserted, preventing trailing characters from shifting.
fn input_view(state: &TextInput, width: usize) -> (String, u16) {
    if width == 0 {
        return (String::new(), 0);
    }

    let cursor = state.cursor.min(state.value.len());
    let mut start = 0;
    let max_cursor_x = width.saturating_sub(1);

    while UnicodeWidthStr::width(&state.value[start..cursor]) > max_cursor_x {
        start = next_char_boundary(&state.value, start);
    }

    let visible = truncate_w(&state.value[start..], width);
    let cursor_x = UnicodeWidthStr::width(&state.value[start..cursor]) as u16;
    (visible, cursor_x)
}

fn next_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    index + s[index..].chars().next().map_or(0, char::len_utf8)
}

// ---- Helpers ----

/// Unified modal container Block: centered title + 1 space padding in all directions.
fn modal_block(title: &str, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
        .padding(Padding::new(1, 1, 1, 1))
}

/// Shared modal renderer: Clears the outer bounds and draws the Block inset by 1 margin cell laterally.
/// This margin absorbs overlapping double-width characters from the background behind,
/// preventing frame borders from getting clipped. Returns the inner content Rect.
/// `Clear` resets cells to the terminal default, so the theme base is repainted under the modal.
/// Dialog modes that fade the screen behind them. ThemeSelect is excluded because
/// the backdrop IS the live theme preview; Help repaints the full frame anyway;
/// Table/Keyword are not dialogs.
fn backdrop_dimmed(mode: UiMode) -> bool {
    !matches!(
        mode,
        UiMode::Table | UiMode::Keyword | UiMode::ThemeSelect | UiMode::Help
    )
}

/// Percentage each backdrop cell moves toward the theme background while a dialog is open.
const BACKDROP_FADE_PCT: u32 = 50;

/// Fades every rendered cell toward the theme background so the dialog painted
/// afterwards stands out (dialogs repaint their own area via `render_modal`).
/// `Color::Reset` backgrounds are terminal-owned and can't be blended, so they
/// stay untouched and only foregrounds fade, using the dark-flag assumption —
/// same fallback as pulse fades (`Theme::bg_rgb`).
fn dim_backdrop(f: &mut Frame, th: &Theme) {
    let target = th.bg_rgb();
    let reset_fg = if th.dark {
        (235, 235, 235)
    } else {
        (16, 16, 16)
    };
    let area = f.area();
    let buf = f.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            let fg = match cell.fg {
                Color::Reset => reset_fg,
                c => crate::theme::color_rgb(c),
            };
            cell.fg = fade_toward(fg, target);
            if cell.bg != Color::Reset {
                cell.bg = fade_toward(crate::theme::color_rgb(cell.bg), target);
            }
        }
    }
}

/// Linear blend of `c` toward `target` by `BACKDROP_FADE_PCT` percent.
fn fade_toward((r, g, b): (u8, u8, u8), (tr, tg, tb): (u8, u8, u8)) -> Color {
    let mix = |c: u8, t: u8| -> u8 {
        ((u32::from(c) * (100 - BACKDROP_FADE_PCT) + u32::from(t) * BACKDROP_FADE_PCT) / 100) as u8
    };
    Color::Rgb(mix(r, tr), mix(g, tg), mix(b, tb))
}

fn render_modal(f: &mut Frame, outer: Rect, block: Block<'static>, th: &Theme) -> Rect {
    f.render_widget(Clear, outer);
    f.render_widget(Block::default().style(th.base_style()), outer);
    let block_area = Rect {
        x: outer.x + 1,
        y: outer.y,
        width: outer.width.saturating_sub(2),
        height: outer.height,
    };
    let inner = block.inner(block_area);
    f.render_widget(block, block_area);
    inner
}

/// Block capable of rendering left/right navigation arrows at the corners of the top frame.
/// `nav_left` / `nav_right` dictate navigable directions when focused.
/// Arrows only overlay if focused (hidden in unfocused blocks).
fn titled_block_nav(
    title: &str,
    focused: bool,
    nav_left: bool,
    nav_right: bool,
    focus_color: Color,
) -> Block<'static> {
    // Focus states: focused uses Cyan thick lines (Thick); unfocused uses default thin lines (Plain).
    let (border_type, style) = if focused {
        (
            BorderType::Thick,
            Style::default()
                .fg(focus_color)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (BorderType::Plain, Style::default())
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(style)
        .title(Span::styled(title.to_string(), style))
        .title_alignment(Alignment::Center);
    // Arrow indicators indicating navigation via Left/Right keys.
    // Since arrows only show in focused (thick-bordered) boxes, a thick horizontal dash (━)
    // is appended to the outer side to connect with the frame: e.g., `┏━ ← ━ … ━ → ━┓`.
    if focused && nav_left {
        block = block.title_top(Line::from("━ ← ").left_aligned().style(style));
    }
    if focused && nav_right {
        block = block.title_top(Line::from(" → ━").right_aligned().style(style));
    }
    block
}

/// Renders scrollbar tracks over the right vertical border of focused panels.
/// Since borders cannot load text via standard title APIs, this directly overwrites
/// target buffer cells after the widget renders.
///
/// - Places `↑` directly under the top border cell, and `↓` directly above the bottom border cell.
/// - Draws a solid block (`█`) representing the thumb position and ratio inside the track.
///   Thumb height scales to `viewport / total`; position correlates with `offset / max_offset`.
///   If scroll is not required (`total <= viewport`), renders arrows only without the solid block.
fn draw_vscrollbar(
    f: &mut Frame,
    area: Rect,
    focused: bool,
    offset: usize,
    total: usize,
    viewport: usize,
    th: &Theme,
) {
    if !focused || area.width == 0 || area.height < 4 {
        return;
    }
    let style = Style::default().fg(th.accent).add_modifier(Modifier::BOLD);
    let x = area.x + area.width - 1;
    let top_y = area.y + 1;
    let bottom_y = area.y + area.height - 2;
    {
        let buf = f.buffer_mut();
        buf[(x, top_y)].set_symbol("↑").set_style(style);
        buf[(x, bottom_y)].set_symbol("↓").set_style(style);
    }

    // Draws thumb inside track (between the bottom of top arrow and top of bottom arrow).
    if area.height < 5 || total <= viewport || viewport == 0 {
        return;
    }
    let track_start = top_y + 1;
    let track_len = (bottom_y - 1 - track_start + 1) as usize; // = height - 4
    let max_offset = total - viewport;
    let offset = offset.min(max_offset);
    let thumb_len = (track_len * viewport / total).max(1).min(track_len);
    // Rounding alignment: sticks to the top of track if at maximum, bottom if at minimum.
    let thumb_off = ((track_len - thumb_len) * offset + max_offset / 2) / max_offset;
    let buf = f.buffer_mut();
    for i in 0..thumb_len {
        let y = track_start + (thumb_off + i) as u16;
        buf[(x, y)].set_symbol("█").set_style(style);
    }
}

/// Dialog button styles: `(focused, unfocused)`.
fn button_styles(th: &Theme) -> (Style, Style) {
    (
        Style::default()
            .fg(th.button_focus_fg)
            .bg(th.button_focus_bg)
            .add_modifier(Modifier::BOLD),
        Style::default().fg(th.button_fg).bg(th.button_bg),
    )
}

fn agent_tag(agent: crate::model::Agent, th: &Theme) -> (&'static str, Color) {
    use crate::model::Agent;
    match agent {
        Agent::Claude => ("CLD ", th.agent_claude),
        Agent::Antigravity => ("AGY ", th.agent_antigravity),
        Agent::Codex => ("CDX ", th.agent_codex),
    }
}

/// Truncates string based on visual character width (accounting for double-width characters).
fn truncate_w(s: &str, max_w: usize) -> String {
    truncate_w_with_ellipsis(s, max_w, "…")
}

/// Right pads string with spaces based on visual width (accounting for double-width characters).
/// Standard `format!("{:<w$}")` counts character lengths, causing alignment bugs with double-width characters.
fn pad_w(s: &str, w: usize) -> String {
    let cur = s.width();
    if cur >= w {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(w - cur))
    }
}

fn truncate_w_with_ellipsis(s: &str, max_w: usize, ellipsis: &str) -> String {
    if s.width() <= max_w {
        return s.to_string();
    }
    if max_w == 0 {
        return String::new();
    }

    let ellipsis_w = ellipsis.width();
    if max_w <= ellipsis_w {
        return ellipsis
            .chars()
            .scan(0usize, |w, ch| {
                let cw = UnicodeWidthStr::width(ch.to_string().as_str());
                if *w + cw > max_w {
                    return None;
                }
                *w += cw;
                Some(ch)
            })
            .collect();
    }

    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if w + cw + ellipsis_w > max_w {
            out.push_str(ellipsis);
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// Wraps text based on visual width constraints (accounts for double-width characters, no ellipsis).
fn wrap_w(s: &str, max_w: usize) -> Vec<String> {
    // Tab stops (space expansion width). unicode-width counts `\t` as 0 width, but terminals expand it.
    // This discrepancy causes text to overflow past frames. Expands tabs to spaces before calculation to match widths.
    const TAB_STOP: usize = 4;
    if max_w == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for ch in s.chars() {
        // Tab: fills with space up to next tab stop (wraps if exceeding max width).
        if ch == '\t' {
            for _ in 0..(TAB_STOP - (current_w % TAB_STOP)) {
                if current_w + 1 > max_w {
                    lines.push(std::mem::take(&mut current));
                    current_w = 0;
                }
                current.push(' ');
                current_w += 1;
            }
            continue;
        }
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if current_w > 0 && current_w + cw > max_w {
            lines.push(current);
            current = String::new();
            current_w = 0;
        }
        if cw > max_w {
            // Extremely rare case where a single character width exceeds max width; handle safely on its own line.
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
                current_w = 0;
            }
            lines.push(ch.to_string());
            continue;
        }
        current.push(ch);
        current_w += cw;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

#[derive(Debug, PartialEq, Eq)]
enum PreviewTurnLine<'a> {
    Content(&'a str),
    Omission(usize),
}

/// If user query exceeds 8 lines, preserves first 4 and last 4 lines, rendering an omission placeholder for the rest.
fn preview_turn_lines(turn: &str) -> Vec<PreviewTurnLine<'_>> {
    let lines: Vec<&str> = turn.lines().collect();
    if lines.len() <= 8 {
        return lines.into_iter().map(PreviewTurnLine::Content).collect();
    }

    lines[..4]
        .iter()
        .copied()
        .map(PreviewTurnLine::Content)
        .chain(std::iter::once(PreviewTurnLine::Omission(lines.len() - 8)))
        .chain(
            lines[lines.len() - 4..]
                .iter()
                .copied()
                .map(PreviewTurnLine::Content),
        )
        .collect()
}

/// Centers a fixed size rectangle (clamped if exceeding terminal area).
fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::{
        input_view, new_session_modal_width, new_session_source_title, new_session_title, pad_w,
        preview_turn_lines, profile_usage_parts, profile_usage_unavailable, truncate_w,
        usage_spans, PreviewTurnLine,
    };
    use crate::ui::TextInput;
    use crate::usage::{ResetCountdown, UsageEntry, UsagePhase, UsageSnapshot, UsageWindow};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

    /// Constructs a mock App on the session table screen with one 2-turn session.
    fn session_app() -> crate::ui::App {
        use crate::model::{Agent, Session};
        use std::path::PathBuf;
        crate::ui::App::new(
            crate::config::Config::load(),
            crate::profile::ProfileStore {
                profiles: Vec::new(),
            },
            vec![Session {
                agent: Agent::Codex,
                profile_id: String::new(),
                id: "session-1".to_string(),
                source_path: None,
                cwd: PathBuf::from("/tmp"),
                folder: "tmp".to_string(),
                mtime_ms: 0,
                ctime_ms: 0,
                size_bytes: 0,
                user_turns: vec!["first question".to_string(), "second question".to_string()],
                search_blob: String::new(),
                assistant_blob: String::new(),
                title_hint: Some("demo".to_string()),
                title_fixed: false,
            }],
            "1 sessions".to_string(),
        )
    }

    /// Constructs a mock App initialized down to the details view with a 2-turn session via Right arrow key inputs.
    fn detail_app() -> crate::ui::App {
        let mut app = session_app();
        let right = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        app.on_key_table(right);
        app.on_key_table(right);
        app
    }

    #[test]
    fn backdrop_dim_applies_to_dialog_modes_only() {
        use crate::ui::UiMode;
        for mode in [
            UiMode::Table,
            UiMode::Keyword,
            UiMode::ThemeSelect,
            UiMode::Help,
        ] {
            assert!(!super::backdrop_dimmed(mode), "{mode:?} must not dim");
        }
        for mode in [
            UiMode::AgentModal,
            UiMode::FolderModal,
            UiMode::DeleteConfirm,
            UiMode::Rename,
            UiMode::ProfileForm,
            UiMode::ProfileDeleteConfirm,
            UiMode::ProfileDirConfirm,
            UiMode::NewSession,
            UiMode::ProjectDirConfirm,
            UiMode::QuickCommand,
            UiMode::Message,
        ] {
            assert!(super::backdrop_dimmed(mode), "{mode:?} must dim");
        }
    }

    #[test]
    fn fade_toward_moves_halfway_to_target() {
        assert_eq!(
            super::fade_toward((100, 100, 100), (0, 0, 0)),
            Color::Rgb(50, 50, 50)
        );
        assert_eq!(
            super::fade_toward((0, 0, 0), (200, 100, 50)),
            Color::Rgb(100, 50, 25)
        );
    }

    #[test]
    fn backdrop_fades_cells_behind_open_dialog() {
        let mut app = session_app();
        app.theme = crate::theme::default_theme();
        let backend = TestBackend::new(200, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (before_fg, before_bg) = (buffer[(0, 0)].fg, buffer[(0, 0)].bg);
        // 'a' opens the agent filter dialog; the frame behind it must fade.
        app.on_key_table(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        terminal.draw(|f| super::draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let (after_fg, after_bg) = (buffer[(0, 0)].fg, buffer[(0, 0)].bg);
        let target = app.theme.bg_rgb();
        assert_eq!(
            after_fg,
            super::fade_toward(crate::theme::color_rgb(before_fg), target)
        );
        assert_eq!(
            after_bg,
            super::fade_toward(crate::theme::color_rgb(before_bg), target)
        );
        assert_ne!(before_fg, after_fg);
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = *buffer.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Locates the buffer cell where `needle` starts. Assumes single-width symbols
    /// (test data is ASCII plus single-width bullets), so char offset == cell x.
    fn find_cell(terminal: &Terminal<TestBackend>, needle: &str) -> (u16, u16) {
        let buffer = terminal.backend().buffer();
        let area = *buffer.area();
        for y in 0..area.height {
            let mut row = String::new();
            for x in 0..area.width {
                row.push_str(buffer[(x, y)].symbol());
            }
            if let Some(pos) = row.find(needle) {
                return (row[..pos].chars().count() as u16, y);
            }
        }
        panic!("needle not found in buffer: {needle}");
    }

    #[test]
    fn detail_screen_renders_two_columns_with_selected_row_background() {
        let app = detail_app();
        assert_eq!(app.screen, crate::ui::Screen::Detail);

        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|f| super::draw(f, &app)).expect("draw");
        let text = buffer_text(&terminal);

        // Left prompt column (focused -> thick borders) and right work/answers column.
        assert!(text.contains(" Prompt "));
        assert!(text.contains(" Q1 Work & Answer "));
        assert!(text.contains("● Q1"));
        assert!(text.contains("● Q2"));
        // Selected question (Q1) uses the focused selection background across the padded row;
        // unselected Q2 keeps the base background. Thick joint overlays are gone.
        let (x1, y1) = find_cell(&terminal, "● Q1");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(x1, y1)].bg, app.theme.selection_bg);
        assert_eq!(buffer[(x1 + 10, y1)].bg, app.theme.selection_bg);
        let (x2, y2) = find_cell(&terminal, "● Q2");
        assert_ne!(buffer[(x2, y2)].bg, app.theme.selection_bg);
        assert!(!text.contains("┣━"));
        // Fallback banner (empty intermediate work logs warning) and final assistant answer section.
        assert!(text.contains("No intermediate work extracted for this turn."));
        assert!(text.contains("● Final Answer"));
    }

    #[test]
    fn detail_screen_work_focus_uses_inactive_selection_background() {
        let mut app = detail_app();
        app.on_key_detail(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)); // Right focus

        let backend = TestBackend::new(100, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|f| super::draw(f, &app)).expect("draw");
        let text = buffer_text(&terminal);

        // Left panel loses focus: whole panel dims (same rule as the session table) —
        // selection switches to the inactive reversed highlight and unselected
        // questions render in soft-dim (muted).
        assert!(text.contains(" Q1 Work & Answer "));
        let (x1, y1) = find_cell(&terminal, "● Q1");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(x1, y1)].bg, app.theme.selection_inactive_bg);
        assert!(buffer[(x1, y1)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED));
        let (x2, y2) = find_cell(&terminal, "● Q2");
        assert_eq!(buffer[(x2, y2)].fg, app.theme.muted);
        assert!(!text.contains("┝━"));
    }

    #[test]
    fn detail_work_panel_hides_tools_by_default_and_dot_shows_them() {
        let mut app = detail_app();
        // Injects tool execution logs into the first turn (fallback turns are empty by default).
        if let Some(d) = app.detail.as_mut() {
            d.turns[0].work_entries = vec![
                crate::handoff::WorkEntry {
                    // Use ASCII text as double-width (Korean) characters split cell columns in the TestBackend buffer.
                    kind: crate::handoff::WorkKind::AssistantText,
                    text: "starting work now".to_string(),
                },
                crate::handoff::WorkEntry {
                    kind: crate::handoff::WorkKind::ToolCall,
                    text: "cargo build".to_string(),
                },
                crate::handoff::WorkEntry {
                    kind: crate::handoff::WorkKind::ToolResult,
                    text: "build ok".to_string(),
                },
            ];
        }

        let backend = TestBackend::new(110, 32);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|f| super::draw(f, &app)).expect("draw");
        let text = buffer_text(&terminal);

        // Default: Tool Calls / Results collapsed & hidden under placeholder text, Assistant text visible.
        assert!(text.contains("starting work now"));
        assert!(!text.contains("● Tool Call"));
        assert!(!text.contains("cargo build"));
        assert!(text.contains("2 tool call/result hidden"));

        // After toggle char `.`: reveals intermediate tool logs.
        app.on_key_detail(KeyEvent::new(KeyCode::Char('.'), KeyModifiers::NONE));
        terminal.draw(|f| super::draw(f, &app)).expect("draw");
        let text = buffer_text(&terminal);
        assert!(text.contains("● Tool Call"));
        assert!(text.contains("cargo build"));
        assert!(!text.contains("hidden"));
    }

    #[test]
    fn rename_input_cursor_does_not_insert_a_display_cell() {
        let state = TextInput {
            value: "앞뒤".to_string(),
            cursor: "앞".len(),
        };

        assert_eq!(input_view(&state, 10), ("앞뒤".to_string(), 2));
    }

    #[test]
    fn rename_input_scrolls_horizontally_to_keep_cursor_visible() {
        let state = TextInput {
            value: "가나다라마바사".to_string(),
            cursor: "가나다라마바사".len(),
        };

        assert_eq!(input_view(&state, 5), ("바사".to_string(), 4));
    }

    #[test]
    fn preview_keeps_queries_with_eight_lines() {
        let turn = "1\n2\n3\n4\n5\n6\n7\n8";

        assert_eq!(
            preview_turn_lines(turn),
            vec![
                PreviewTurnLine::Content("1"),
                PreviewTurnLine::Content("2"),
                PreviewTurnLine::Content("3"),
                PreviewTurnLine::Content("4"),
                PreviewTurnLine::Content("5"),
                PreviewTurnLine::Content("6"),
                PreviewTurnLine::Content("7"),
                PreviewTurnLine::Content("8"),
            ]
        );
    }

    #[test]
    fn preview_collapses_queries_over_eight_lines() {
        let turn = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13";

        assert_eq!(
            preview_turn_lines(turn),
            vec![
                PreviewTurnLine::Content("1"),
                PreviewTurnLine::Content("2"),
                PreviewTurnLine::Content("3"),
                PreviewTurnLine::Content("4"),
                PreviewTurnLine::Omission(5),
                PreviewTurnLine::Content("10"),
                PreviewTurnLine::Content("11"),
                PreviewTurnLine::Content("12"),
                PreviewTurnLine::Content("13"),
            ]
        );
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

    #[test]
    fn profile_usage_columns_split_percent_and_reset() {
        let parts = profile_usage_parts(
            ready_usage(usage_window(72, 0, 4, 30), usage_window(52, 2, 16, 0)),
            0,
            false,
            &crate::theme::default_theme(),
        );
        let texts: Vec<String> = parts.into_iter().map(|(text, _)| text).collect();

        assert_eq!(texts, vec![" 72%", "4h 30m", " 52%", "2d 16h"]);
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

    /// Constructs a mock App focused on the preview panel (right panel) using the provided user turns.
    fn preview_app(turns: Vec<String>) -> crate::ui::App {
        use crate::model::{Agent, Session};
        use std::path::PathBuf;
        let mut app = crate::ui::App::new(
            crate::config::Config::load(),
            crate::profile::ProfileStore {
                profiles: Vec::new(),
            },
            vec![Session {
                agent: Agent::Codex,
                profile_id: String::new(),
                id: "s1".to_string(),
                source_path: None,
                cwd: PathBuf::from("/tmp"),
                folder: "tmp".to_string(),
                mtime_ms: 0,
                ctime_ms: 0,
                size_bytes: 0,
                user_turns: turns,
                search_blob: String::new(),
                assistant_blob: String::new(),
                title_hint: Some("demo".to_string()),
                title_fixed: false,
            }],
            "1 sessions".to_string(),
        );
        app.on_key_table(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)); // Preview focus
        app
    }

    fn render_at(app: &crate::ui::App, w: u16, h: u16) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| super::draw(f, app)).unwrap();
    }

    /// Mock App with `n` sessions in distinct folders and the folder modal opened.
    fn folder_modal_app(n: usize) -> crate::ui::App {
        use crate::model::{Agent, Session};
        use std::path::PathBuf;
        let sessions: Vec<Session> = (0..n)
            .map(|i| Session {
                agent: Agent::Codex,
                profile_id: String::new(),
                id: format!("session-{i}"),
                source_path: None,
                cwd: PathBuf::from(format!("/tmp/folder{i:02}")),
                folder: format!("folder{i:02}"),
                mtime_ms: 0,
                ctime_ms: 0,
                size_bytes: 0,
                user_turns: vec!["question".to_string()],
                search_blob: String::new(),
                assistant_blob: String::new(),
                title_hint: None,
                title_fixed: false,
            })
            .collect();
        let mut app = crate::ui::App::new(
            crate::config::Config::load(),
            crate::profile::ProfileStore {
                profiles: Vec::new(),
            },
            sessions,
            format!("{n} sessions"),
        );
        app.on_key_table(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        app
    }

    #[test]
    fn folder_modal_moving_up_within_window_does_not_scroll() {
        let mut app = folder_modal_app(30);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        for _ in 0..29 {
            app.on_key_folder_modal(down);
        }
        render_at(&app, 100, 30);
        // Dialog is 20 rows -> 14 list rows; cursor at 29 pins the bottom-most window.
        let m = app.folder_modal.as_ref().unwrap();
        assert_eq!(m.scroll.get(), 16);

        for _ in 0..3 {
            app.on_key_folder_modal(up);
        }
        render_at(&app, 100, 30);
        let m = app.folder_modal.as_ref().unwrap();
        assert_eq!(m.cursor, 26);
        assert_eq!(
            m.scroll.get(),
            16,
            "Moving up inside the visible window must not scroll"
        );
    }

    #[test]
    fn folder_modal_scrolls_only_when_cursor_leaves_window() {
        let mut app = folder_modal_app(30);
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        for _ in 0..29 {
            app.on_key_folder_modal(down);
        }
        render_at(&app, 100, 30);
        // Move above the window top (offset 16): offset must follow the cursor.
        for _ in 0..14 {
            app.on_key_folder_modal(up);
        }
        render_at(&app, 100, 30);
        let m = app.folder_modal.as_ref().unwrap();
        assert_eq!(m.cursor, 15);
        assert_eq!(m.scroll.get(), 15);
        // Moving back down within the window keeps the offset.
        app.on_key_folder_modal(down);
        render_at(&app, 100, 30);
        let m = app.folder_modal.as_ref().unwrap();
        assert_eq!(m.scroll.get(), 15);
    }

    #[test]
    fn preview_does_not_scroll_when_content_fits() {
        let app = preview_app(vec!["short".to_string()]);
        render_at(&app, 100, 40); // Large screen: content fits in viewport -> max scroll is 0
        assert_eq!(app.preview_max_scroll.get(), 0);

        let mut app = app;
        for _ in 0..5 {
            app.on_key_table(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(
            app.preview_scroll, 0,
            "Should not scroll when content fits in viewport"
        );
    }

    #[test]
    fn preview_scrolls_when_content_overflows() {
        let turns: Vec<String> = (1..=30).map(|i| format!("question {i}")).collect();
        let mut app = preview_app(turns);
        render_at(&app, 60, 10); // Small screen: content overflows -> scroll is enabled
        assert!(app.preview_max_scroll.get() > 0);

        app.on_key_table(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert!(
            app.preview_scroll > 0,
            "Should scroll when content overflows"
        );
    }

    fn new_session_state(
        context: Option<crate::ui::SessionContextRef>,
    ) -> crate::ui::NewSessionState {
        crate::ui::NewSessionState {
            profile_idx: 0,
            focus: crate::ui::NewSessionFocus::Buttons,
            dropdown_open: false,
            profile_cursor: 0,
            model_options: Vec::new(),
            model_idx: 0,
            model_cursor: 0,
            input: TextInput {
                value: String::new(),
                cursor: 0,
            },
            folders: Vec::new(),
            ordered: Vec::new(),
            match_count: 0,
            folder_cursor: None,
            ok_focused: true,
            error: None,
            context,
        }
    }

    #[test]
    fn new_session_title_plain_without_context() {
        let state = new_session_state(None);
        assert_eq!(new_session_title(&state), " New Session ");
    }

    #[test]
    fn new_session_width_is_20_cells_wider_and_capped_at_80_percent() {
        assert_eq!(new_session_modal_width(200), 102);
        assert_eq!(new_session_modal_width(128), 102);
        assert_eq!(new_session_modal_width(120), 96);
        assert_eq!(new_session_modal_width(100), 80);
        assert_eq!(new_session_modal_width(60), 48);
    }

    #[test]
    fn contextual_new_session_separates_outer_and_source_titles() {
        use unicode_width::UnicodeWidthStr;
        let ctx = crate::ui::SessionContextRef {
            agent: crate::model::Agent::Codex,
            profile_id: "builtin-codex".to_string(),
            session_id: "abc".to_string(),
            title: "아주 아주 아주 아주 긴 한국어 세션 제목이 여기에 들어갑니다".to_string(),
        };
        let state = new_session_state(Some(ctx));
        assert_eq!(new_session_title(&state), " New Session with Context ");

        for w in [78usize, 56, 30] {
            let title = new_session_source_title(&state, w).expect("source title");
            assert!(
                title.width() <= w,
                "width {w}: source title too wide ({})",
                title.width()
            );
            assert!(!title.contains("codex"));
        }
    }

    #[test]
    fn contextual_source_is_a_dimmed_read_only_control() {
        let mut app = session_app();
        app.theme = crate::theme::default_theme();
        app.mode = crate::ui::UiMode::NewSession;
        app.new_session = Some(new_session_state(Some(crate::ui::SessionContextRef {
            agent: crate::model::Agent::Codex,
            profile_id: "builtin-codex".to_string(),
            session_id: "abc".to_string(),
            title: "source-session-title".to_string(),
        })));

        let mut terminal = Terminal::new(TestBackend::new(160, 30)).expect("terminal");
        terminal.draw(|f| super::draw(f, &app)).expect("draw");
        let (label_x, label_y) = find_cell(&terminal, "Context Source");
        let (value_x, value_y) = find_cell(&terminal, "source-session-title");
        let buffer = terminal.backend().buffer();
        let muted = app.theme.muted;

        assert_eq!(buffer[(label_x, label_y)].fg, muted);
        assert_eq!(buffer[(value_x, value_y)].fg, muted);
        let border_x = (0..label_x)
            .rev()
            .find(|&x| buffer[(x, label_y)].symbol() == "┌")
            .expect("plain source border");
        assert_eq!(buffer[(border_x, label_y)].fg, muted);
        assert!(!(border_x..=label_x + "Context Source".len() as u16)
            .any(|x| buffer[(x, label_y)].symbol() == "▾"));
    }
}
