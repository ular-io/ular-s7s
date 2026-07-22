//! The theme selection dialog with live preview.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R9). The
//! file owns the dialog state (`ThemeSelectState`), the `App` open and key
//! handling (cursor moves apply the theme immediately for live preview; Enter
//! persists, Esc reverts), and the rendering. `ThemeSelectState` is re-exported
//! from `ui` so the existing `crate::ui::ThemeSelectState` path stays stable.

use crate::ui::components::modal::{modal_block, render_modal};
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::truncate_w;
use crate::ui::render::centered_fixed_rect;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Padding, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

// ---- State ----

/// Theme selection dialog state. Cursor moves apply the theme immediately
/// (live preview); Esc restores `original`, Enter persists the selection.
pub struct ThemeSelectState {
    /// Selectable themes: built-ins followed by custom (themes/*.toml) entries.
    pub themes: Vec<crate::theme::Theme>,
    /// Whether the visible list shows Dark (true) or Light (false) themes.
    /// Left/Right swaps the entire displayed list between the two categories.
    pub dark_view: bool,
    /// Cursor index within the currently visible (category-filtered) list.
    pub cursor: usize,
    /// Top visible row. Managed by the renderer (which knows the viewport height):
    /// the cursor moves freely within the window and the offset only shifts when
    /// the cursor would fall outside it, so selecting upward no longer scrolls.
    pub scroll: std::cell::Cell<usize>,
    /// Active theme at dialog open time, restored on Esc.
    original: crate::theme::Theme,
}

impl ThemeSelectState {
    /// Indices into `themes` belonging to the currently visible category.
    pub fn visible(&self) -> Vec<usize> {
        self.themes
            .iter()
            .enumerate()
            .filter(|(_, t)| t.dark == self.dark_view)
            .map(|(i, _)| i)
            .collect()
    }
}

// ---- Input ----

impl App {
    /// Opens the theme selection dialog with the cursor on the active theme.
    pub(crate) fn open_theme_select(&mut self) {
        let themes = crate::theme::all_themes();
        if themes.is_empty() {
            return;
        }
        let active = themes
            .iter()
            .position(|t| t.key == self.theme.key)
            .unwrap_or(0);
        let dark_view = themes.get(active).map(|t| t.dark).unwrap_or(true);
        // Cursor position within the active theme's own category.
        let cursor = themes
            .iter()
            .enumerate()
            .filter(|(_, t)| t.dark == dark_view)
            .position(|(i, _)| i == active)
            .unwrap_or(0);
        self.theme_select = Some(ThemeSelectState {
            themes,
            dark_view,
            cursor,
            scroll: std::cell::Cell::new(0),
            original: self.theme.clone(),
        });
        self.mode = UiMode::ThemeSelect;
        self.status_msg = None;
    }

    /// Theme dialog keys: Up/Down move within the visible list (clamped — no wrap)
    /// and apply the theme immediately (live preview); Left shows the Dark list and
    /// Right shows the Light list (swapping the entire displayed set); Enter persists
    /// the selection and closes; Esc restores the theme active at open time. No buttons.
    pub fn on_key_theme_select(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        let Some(state) = self.theme_select.as_mut() else {
            self.mode = UiMode::Table;
            return;
        };
        let visible = state.visible();
        let vlen = visible.len();
        match key.code {
            KeyCode::Esc => {
                self.theme = state.original.clone();
                self.theme_select = None;
                self.mode = UiMode::Table;
            }
            KeyCode::Enter => {
                crate::theme::save_selected(&self.theme.key);
                self.status_msg = Some(format!("Theme: {}", self.theme.name));
                self.theme_select = None;
                self.mode = UiMode::Table;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.cursor = state.cursor.saturating_sub(1);
                self.theme = state.themes[visible[state.cursor]].clone();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.cursor = (state.cursor + 1).min(vlen.saturating_sub(1));
                self.theme = state.themes[visible[state.cursor]].clone();
            }
            // Left → Dark list, Right → Light list (swap the whole visible list).
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') => {
                let want_dark = matches!(key.code, KeyCode::Left | KeyCode::Char('h'));
                if want_dark != state.dark_view {
                    state.dark_view = want_dark;
                    state.cursor = 0;
                    state.scroll.set(0);
                    if let Some(&idx) = state.visible().first() {
                        self.theme = state.themes[idx].clone();
                    }
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                state.cursor = 0;
                self.theme = state.themes[visible[state.cursor]].clone();
            }
            KeyCode::End | KeyCode::Char('G') => {
                state.cursor = vlen.saturating_sub(1);
                self.theme = state.themes[visible[state.cursor]].clone();
            }
            _ => {}
        }
    }
}

// ---- Render ----

/// Theme selection dialog: built-in + custom themes with a dark/light/custom tag.
/// Moving the cursor applies the theme immediately (the whole frame, including this
/// dialog, re-renders in the previewed theme); Enter commits, Esc reverts. No buttons.
pub(crate) fn draw_theme_select(f: &mut Frame, app: &App) {
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
