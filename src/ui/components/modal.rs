//! Modal framing and backdrop primitives shared by every dialog: the thick
//! titled block, the clear-and-repaint renderer, navigation-arrow blocks, form
//! input boxes, dropdown frames, button styles, and the behind-dialog backdrop
//! fade.

use crate::theme::Theme;
use crate::ui::components::text::truncate_w;
use crate::ui::render::input_view;
use crate::ui::{TextInput, UiMode};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph},
    Frame,
};

pub(crate) fn modal_block(title: &str, color: Color) -> Block<'static> {
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

/// Dialog modes that fade the screen behind them. ThemeSelect is excluded because
/// the backdrop IS the live theme preview; Help repaints the full frame anyway;
/// Table/Keyword are not dialogs.
pub(crate) fn backdrop_dimmed(mode: UiMode) -> bool {
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
pub(crate) fn dim_backdrop(f: &mut Frame, th: &Theme) {
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
pub(crate) fn fade_toward((r, g, b): (u8, u8, u8), (tr, tg, tb): (u8, u8, u8)) -> Color {
    let mix = |c: u8, t: u8| -> u8 {
        ((u32::from(c) * (100 - BACKDROP_FADE_PCT) + u32::from(t) * BACKDROP_FADE_PCT) / 100) as u8
    };
    Color::Rgb(mix(r, tr), mix(g, tg), mix(b, tb))
}

/// Shared modal renderer: Clears the outer bounds and draws the Block inset by 1 margin cell laterally.
/// This margin absorbs overlapping double-width characters from the background behind,
/// preventing frame borders from getting clipped. Returns the inner content Rect.
/// `Clear` resets cells to the terminal default, so the theme base is repainted under the modal.
pub(crate) fn render_modal(f: &mut Frame, outer: Rect, block: Block<'static>, th: &Theme) -> Rect {
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
/// `dimmed` = another pane has focus: the frame and title fade with the pane's content.
pub(crate) fn titled_block_nav(
    title: &str,
    focused: bool,
    dimmed: bool,
    nav_left: bool,
    nav_right: bool,
    th: &Theme,
) -> Block<'static> {
    // Focus states: focused uses accent thick lines (Thick); unfocused uses thin lines
    // (Plain), soft-dim while another pane has focus and default otherwise (no pane
    // focused while an overlay owns input).
    let (border_type, style) = if focused {
        (
            BorderType::Thick,
            Style::default().fg(th.accent).add_modifier(Modifier::BOLD),
        )
    } else if dimmed {
        (BorderType::Plain, th.soft_dim())
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

/// Single-line form input box titled with its label (three rows with borders).
/// Focused: Thick accent border and the hardware cursor; unfocused: a Plain `dim`
/// border, lighter than a panel because it sits inside a dialog. A whole-value
/// selection paints only the text, and only while focused. `placeholder` fills an
/// empty box only while it is not focused, because the cursor sits where it would
/// start.
pub(crate) fn form_input(
    f: &mut Frame,
    area: Rect,
    label: &str,
    input: &TextInput,
    focused: bool,
    placeholder: &str,
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
    let width = inner.width as usize;
    let (visible, cursor_x) = input_view(input, width);
    let value = if input.value.is_empty() && !focused {
        Span::styled(truncate_w(placeholder, width), th.soft_dim())
    } else if focused && input.select_all {
        Span::styled(
            visible,
            Style::default().fg(th.selection_fg).bg(th.selection_bg),
        )
    } else {
        Span::raw(visible)
    };
    f.render_widget(Paragraph::new(Line::from(value)), inner);
    if focused {
        f.set_cursor_position((inner.x.saturating_add(cursor_x), inner.y));
    }
}

/// Clears and frames a dropdown popup that overlaps the bottom border of its
/// combo box: a thick `border` frame whose top edge is redrawn as `┣━┫` to join
/// the combo. Returns the inner area. `dialog` is the dialog the combo sits in:
/// one extra column left of the popup is cleared only on rows below it, where a
/// backdrop double-width glyph could bleed into the border, so the dialog's own
/// bottom border keeps its `━` beside the popup.
pub(crate) fn dropdown_frame(
    f: &mut Frame,
    popup: Rect,
    dialog: Rect,
    border: Style,
    th: &Theme,
) -> Rect {
    f.render_widget(Clear, popup);
    f.render_widget(Block::default().style(th.base_style()), popup);
    let below_dialog = dialog.bottom().max(popup.y);
    if popup.x > 0 && popup.bottom() > below_dialog {
        let margin = Rect::new(popup.x - 1, below_dialog, 1, popup.bottom() - below_dialog);
        f.render_widget(Clear, margin);
        f.render_widget(Block::default().style(th.base_style()), margin);
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(border);
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let width = popup.width as usize;
    let join = if width > 2 {
        format!("┣{}┫", "━".repeat(width - 2))
    } else {
        "━━".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(join, border)),
        Rect { height: 1, ..popup },
    );
    inner
}

/// Thin divider across a dropdown popup's row `y`, in `line` style, its ends
/// joined to the thick side borders as `┠`/`┨` in `border` style.
pub(crate) fn dropdown_divider(f: &mut Frame, popup: Rect, y: u16, border: Style, line: Style) {
    if popup.width < 2 {
        return;
    }
    f.render_widget(
        Paragraph::new(Span::styled("─".repeat(popup.width as usize), line)),
        Rect::new(popup.x, y, popup.width, 1),
    );
    let buf = f.buffer_mut();
    buf[(popup.x, y)].set_symbol("┠").set_style(border);
    buf[(popup.right() - 1, y)]
        .set_symbol("┨")
        .set_style(border);
}

/// Dialog button styles: `(focused, unfocused)`.
pub(crate) fn button_styles(th: &Theme) -> (Style, Style) {
    (
        Style::default()
            .fg(th.button_focus_fg)
            .bg(th.button_focus_bg)
            .add_modifier(Modifier::BOLD),
        Style::default().fg(th.button_fg).bg(th.button_bg),
    )
}

#[cfg(test)]
mod tests {
    use super::{backdrop_dimmed, fade_toward};
    use crate::ui::UiMode;
    use ratatui::style::Color;

    #[test]
    fn backdrop_dim_applies_to_dialog_modes_only() {
        for mode in [
            UiMode::Table,
            UiMode::Keyword,
            UiMode::ThemeSelect,
            UiMode::Help,
        ] {
            assert!(!backdrop_dimmed(mode), "{mode:?} must not dim");
        }
        for mode in [
            UiMode::AgentModal,
            UiMode::FolderModal,
            UiMode::DeleteConfirm,
            UiMode::Rename,
            UiMode::ChangeFolder,
            UiMode::ProfileForm,
            UiMode::ProfileDeleteConfirm,
            UiMode::ProfileDirConfirm,
            UiMode::NewSession,
            UiMode::ProjectDirConfirm,
            UiMode::QuickCommand,
            UiMode::Message,
            UiMode::WorkspaceEdit,
            UiMode::WorkspaceDeleteConfirm,
        ] {
            assert!(backdrop_dimmed(mode), "{mode:?} must dim");
        }
    }

    #[test]
    fn fade_toward_moves_halfway_to_target() {
        assert_eq!(
            fade_toward((100, 100, 100), (0, 0, 0)),
            Color::Rgb(50, 50, 50)
        );
        assert_eq!(
            fade_toward((0, 0, 0), (200, 100, 50)),
            Color::Rgb(100, 50, 25)
        );
    }
}
