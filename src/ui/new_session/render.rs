//! New Session dialog rendering: the read-only context source control, the
//! profile/model/folder combo boxes, the OK/Cancel button row, and the dropdown
//! overlay popup.

use crate::ui::components::modal::{button_styles, modal_block, render_modal};
use crate::ui::components::text::{pad_w, truncate_w};
use crate::ui::render::{centered_fixed_rect, input_view, usage_spans};
use crate::ui::App;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Padding, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// New session creation dialog (read-only source plus profile, model, and folder dropdown controls).
///
/// Renders focused controls with highlighted thick borders. Expanded dropdown overlays
/// directly below the respective control (only one dropdown may open at a time).
/// Folder list draws query-matching entries in normal text at the top, and unmatched entries
/// in soft dim (matching "left" label color) at the bottom.
/// Title of the shared New Session dialog. Context details are rendered in the
/// modal body so the outer title remains stable at narrow terminal widths.
fn new_session_title(state: &crate::ui::NewSessionState) -> &'static str {
    if state.context.is_some() {
        " New Session with Context "
    } else {
        " New Session "
    }
}

/// Source session title for the read-only context control.
fn new_session_source_title(state: &crate::ui::NewSessionState, inner_w: usize) -> Option<String> {
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

pub(crate) fn draw_new_session_modal(f: &mut Frame, app: &App) {
    use crate::ui::NewSessionFocus;
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

#[cfg(test)]
mod tests {
    use super::{new_session_modal_width, new_session_source_title, new_session_title};
    use crate::ui::TextInput;

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
}
