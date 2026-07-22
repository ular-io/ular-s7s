//! Profile screen rendering: the profile list table (with the merged usage
//! cell and status column) and the add/edit form, deletion, and
//! config-directory confirmation modals.

use crate::theme::Theme;
use crate::ui::components::modal::{button_styles, modal_block, render_modal, titled_block_nav};
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::truncate_w;
use crate::ui::render::{
    agent_tag, centered_fixed_rect, display_path, input_view, pulse_level_now, pulse_span,
    reset_label_current, reset_label_weekly, LOADING_LABEL, MISSING_DIR_LABEL, NOT_INSTALLED_LABEL,
    NOT_LOGGED_IN_LABEL,
};
use crate::ui::{App, TextInput, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Cell, Padding, Paragraph, Row, Table, TableState, Wrap},
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

/// Profile creation/edit form: agent radio options, text inputs (Name, Config Path), and Save/Cancel buttons.
pub(crate) fn draw_profile_form(f: &mut Frame, app: &App) {
    use crate::model::Agent;
    use crate::ui::FormFocus;

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
pub(crate) fn draw_profile_delete_confirm(f: &mut Frame, app: &App) {
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
pub(crate) fn draw_profile_dir_confirm(f: &mut Frame, app: &App) {
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

#[cfg(test)]
mod tests {
    use super::{profile_usage_parts, profile_usage_unavailable};
    use crate::usage::{ResetCountdown, UsageEntry, UsagePhase, UsageSnapshot, UsageWindow};

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
}
