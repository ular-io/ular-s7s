//! Session detail screen rendering: the two-column layout — left questions
//! list (mirroring the search preview) and the right per-turn work-and-answer
//! panel with tool-call collapsing.

use crate::handoff::WorkKind;
use crate::model::format_local_datetime_seconds;
use crate::theme::Theme;
use crate::ui::components::modal::titled_block_nav;
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::{pad_w, wrap_w};
use crate::ui::render::{preview_turn_lines, session_meta_lines, PreviewTurnLine};
use crate::ui::{App, DetailFocus, SessionDetailState, UiMode};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Maximum lines allowed for a single work entry in the right panel (excess is truncated with omission placeholder).
const WORK_ENTRY_MAX_LINES: usize = 120;
/// Maximum lines allowed for the final answer.
const FINAL_ANSWER_MAX_LINES: usize = 400;

/// Session details view: left questions list and right selected question's work/answers panel.
pub(crate) fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
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
        let mut title_spans = vec![Span::styled(title.clone(), title_style)];
        if let Some(timestamp) = turn.submitted_at_ms.and_then(format_local_datetime_seconds) {
            let timestamp_style = if selected && focused {
                th.soft_dim().bg(sel_bg)
            } else if selected {
                th.soft_dim().bg(sel_bg).add_modifier(Modifier::REVERSED)
            } else {
                th.soft_dim()
            };
            let timestamp = format!("  {timestamp}");
            title_spans.push(Span::styled(
                if selected {
                    pad_w(&timestamp, inner_w.saturating_sub(title.width()))
                } else {
                    timestamp
                },
                timestamp_style,
            ));
        } else if selected {
            title_spans[0] = Span::styled(pad_w(&title, inner_w), title_style);
        }
        rows.push(Line::from(title_spans));
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
