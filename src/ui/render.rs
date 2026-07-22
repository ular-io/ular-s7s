//! TUI rendering: k9s-style header (logo + hotkeys) / session search table / preview /
//! session details (questions list + work/answers) / status bar / modal windows.

use super::components::modal::{
    backdrop_dimmed, button_styles, dim_backdrop, modal_block, render_modal,
};
use super::components::scrollbar::draw_vscrollbar;
use super::components::text::{pad_w, truncate_w, truncate_w_with_ellipsis, wrap_w};
use super::{next_char_boundary, App, MessageKind, Screen, TextInput, UiMode};
use crate::theme::Theme;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap},
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
        super::profile::render::draw_profile_table(f, app, root[1]);
    } else if app.screen == Screen::Detail {
        super::detail::render::draw_detail(f, app, root[1]);
    } else if app.mode == UiMode::Keyword {
        // Keyword mode: overlays search prompt box on top of the main body (k9s-style).
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(3)])
            .split(root[1]);
        super::session::render::draw_search_prompt(f, app, body[0]);
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
        UiMode::ProfileForm => super::profile::render::draw_profile_form(f, app),
        UiMode::ProfileDeleteConfirm => super::profile::render::draw_profile_delete_confirm(f, app),
        UiMode::ProfileDirConfirm => super::profile::render::draw_profile_dir_confirm(f, app),
        UiMode::NewSession => super::new_session::render::draw_new_session_modal(f, app),
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

// Weekly countdown discards minutes, tracking down to hours: `(2d  6h)`, `(   17h)`, `(    2h)`.
pub(crate) fn reset_label_weekly(reset: Option<crate::usage::ResetCountdown>) -> String {
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

    super::session::render::draw_table(f, app, cols[0]);
    super::session::render::draw_preview(f, app, cols[1]);
}

/// Formats path string, substituting home directory prefix with `~`.
pub(crate) fn display_path(path: &std::path::Path) -> String {
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

/// Shared helper for preview/detail left panels: constructs 5 session metadata rows (Project, Name, Created, Modified, ID).
/// `dimmed` renders the accent-colored values in soft-dim too (whole panel unfocused).
pub(crate) fn session_meta_lines(
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
pub(crate) fn input_view(state: &TextInput, width: usize) -> (String, u16) {
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

// ---- Helpers ----

pub(crate) fn agent_tag(agent: crate::model::Agent, th: &Theme) -> (&'static str, Color) {
    use crate::model::Agent;
    match agent {
        Agent::Claude => ("CLD ", th.agent_claude),
        Agent::Antigravity => ("AGY ", th.agent_antigravity),
        Agent::Codex => ("CDX ", th.agent_codex),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PreviewTurnLine<'a> {
    Content(&'a str),
    Omission(usize),
}

/// If user query exceeds 8 lines, preserves first 4 and last 4 lines, rendering an omission placeholder for the rest.
pub(crate) fn preview_turn_lines(turn: &str) -> Vec<PreviewTurnLine<'_>> {
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
pub(crate) fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::{input_view, pad_w, preview_turn_lines, truncate_w, usage_spans, PreviewTurnLine};
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
            crate::ui::components::modal::fade_toward(crate::theme::color_rgb(before_fg), target)
        );
        assert_eq!(
            after_bg,
            crate::ui::components::modal::fade_toward(crate::theme::color_rgb(before_bg), target)
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
