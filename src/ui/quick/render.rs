//! Quick Command modal rendering: the input line, the top/bottom thin dividers,
//! the palette command list (with shortcut column) or terminal history list, the
//! selected-item description footer, and the overflow scrollbar.

use super::state::QuickMode;
use super::VIEWPORT;
use crate::ui::components::modal::{modal_block, render_modal};
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::truncate_w;
use crate::ui::render::{centered_fixed_rect, input_view};
use crate::ui::App;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Padding, Paragraph},
    Frame,
};

/// `:`/`!` Quick Command window modal (command palette / terminal command mode).
/// Width is around 120 cells (downscaled on smaller terminals),
/// height scales dynamically up to 10 lines of matched results. Confirms options via Enter, exits via Esc.
/// Matches are sorted with enabled items first; disabled entries are visually dimmed.
pub(crate) fn draw_quick_command(f: &mut Frame, app: &App) {
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
