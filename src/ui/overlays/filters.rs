//! Agent and folder filter multi-select modals.
//!
//! Extracted from `ui::mod` and `ui::render` per the refactoring plan (R9). The
//! file owns the shared multi-select `ModalState`, the `App` key handling for
//! both modals, and their rendering. `ModalState` is re-exported from `ui` so
//! the existing `crate::ui::ModalState` path stays stable. The filter mutations
//! these handlers perform (`self.filter`, `self.recompute()`) reach `App`'s
//! private members declared in the ancestor `ui` module without widening.

use crate::model::Agent;
use crate::theme::Theme;
use crate::ui::components::modal::{modal_block, render_modal};
use crate::ui::components::scrollbar::draw_vscrollbar;
use crate::ui::components::text::truncate_w;
use crate::ui::render::centered_fixed_rect;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Padding, Paragraph},
    Frame,
};
use std::collections::HashSet;

// ---- State ----

/// Shared state for multi-select modals.
pub struct ModalState {
    pub labels: Vec<String>,
    pub cursor: usize,
    pub selected: HashSet<usize>,
    /// Top visible row for scrollable modal lists (folder modal). Managed by the
    /// renderer (which knows the viewport height): the cursor moves freely within
    /// the window and the offset only shifts when the cursor would fall outside
    /// it, so moving the selection upward within the window does not scroll.
    pub scroll: std::cell::Cell<usize>,
}

impl ModalState {
    fn new(labels: Vec<String>, preselected: HashSet<usize>) -> Self {
        ModalState {
            labels,
            cursor: 0,
            selected: preselected,
            scroll: std::cell::Cell::new(0),
        }
    }
    fn move_cursor(&mut self, delta: isize) {
        if self.labels.is_empty() {
            return;
        }
        let len = self.labels.len() as isize;
        let mut c = self.cursor as isize + delta;
        if c < 0 {
            c = 0;
        }
        if c >= len {
            c = len - 1;
        }
        self.cursor = c as usize;
    }
    fn toggle(&mut self) {
        if self.labels.is_empty() {
            return;
        }
        if self.selected.contains(&self.cursor) {
            self.selected.remove(&self.cursor);
        } else {
            self.selected.insert(self.cursor);
        }
    }
}

// ---- Input ----

impl App {
    pub(crate) fn open_agent_modal(&mut self) {
        let labels: Vec<String> = Agent::all().iter().map(|a| a.label().to_string()).collect();
        let mut pre = HashSet::new();
        for (i, a) in Agent::all().iter().enumerate() {
            if self.filter.agents.contains(a) {
                pre.insert(i);
            }
        }
        self.agent_modal = Some(ModalState::new(labels, pre));
        self.mode = UiMode::AgentModal;
    }

    /// Syncs agent selection in the modal to `filter.agents` (enables live list updates while toggling).
    fn sync_agent_selection_to_filter(&mut self) {
        if let Some(m) = &self.agent_modal {
            let mut set = HashSet::new();
            for (i, a) in Agent::all().iter().enumerate() {
                if m.selected.contains(&i) {
                    set.insert(*a);
                }
            }
            self.filter.agents = set;
        }
    }

    fn confirm_agent_modal(&mut self) {
        self.sync_agent_selection_to_filter();
        self.agent_modal = None;
        self.mode = UiMode::Table;
        self.recompute();
    }

    pub(crate) fn open_folder_modal(&mut self) {
        self.folder_query.clear();
        self.rebuild_folder_visible();
        let mut pre = HashSet::new();
        for (vis_i, &all_i) in self.folder_visible.iter().enumerate() {
            if self.filter.folders.contains(&self.all_folders[all_i]) {
                pre.insert(vis_i);
            }
        }
        let labels: Vec<String> = self
            .folder_visible
            .iter()
            .map(|&i| self.all_folders[i].clone())
            .collect();
        self.folder_modal = Some(ModalState::new(labels, pre));
        self.mode = UiMode::FolderModal;
    }

    /// Re-evaluates visible folder lists in folder modal based on `folder_query`.
    /// Existing selections (relative to `all_folders`) are preserved in `filter.folders`.
    fn rebuild_folder_visible(&mut self) {
        let q = crate::normalize::nfc_lower(&self.folder_query);
        self.folder_visible = self
            .all_folders
            .iter()
            .enumerate()
            .filter(|(_, f)| q.is_empty() || crate::normalize::nfc_lower(f).contains(&q))
            .map(|(i, _)| i)
            .collect();
    }

    /// Syncs folder selection in the modal to `filter.folders` (retains checks across query filter changes).
    fn sync_folder_selection_to_filter(&mut self) {
        if let Some(m) = &self.folder_modal {
            for (vis_i, &all_i) in self.folder_visible.iter().enumerate() {
                let name = &self.all_folders[all_i];
                if m.selected.contains(&vis_i) {
                    self.filter.folders.insert(name.clone());
                } else {
                    self.filter.folders.remove(name);
                }
            }
        }
    }

    fn confirm_folder_modal(&mut self) {
        self.sync_folder_selection_to_filter();
        self.folder_modal = None;
        self.mode = UiMode::Table;
        self.recompute();
    }

    pub fn on_key_agent_modal(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(m) = &mut self.agent_modal {
                    m.move_cursor(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(m) = &mut self.agent_modal {
                    m.move_cursor(1);
                }
            }
            KeyCode::Char(' ') => {
                if let Some(m) = &mut self.agent_modal {
                    m.toggle();
                }
                self.sync_agent_selection_to_filter();
                self.recompute();
            }
            KeyCode::Enter => self.confirm_agent_modal(),
            KeyCode::Esc => {
                self.agent_modal = None;
                self.mode = UiMode::Table;
            }
            _ => {}
        }
    }

    /// Handles key inputs in the Folder filter modal (supporting incremental search).
    pub fn on_key_folder_modal(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up => {
                if let Some(m) = &mut self.folder_modal {
                    m.move_cursor(-1);
                }
            }
            KeyCode::Down => {
                if let Some(m) = &mut self.folder_modal {
                    m.move_cursor(1);
                }
            }
            KeyCode::Char(' ') => {
                if let Some(m) = &mut self.folder_modal {
                    m.toggle();
                }
                self.sync_folder_selection_to_filter();
                self.recompute();
            }
            KeyCode::Char(c) => {
                self.folder_query.push(c);
                self.refresh_folder_modal_labels();
            }
            KeyCode::Backspace => {
                self.folder_query.pop();
                self.refresh_folder_modal_labels();
            }
            KeyCode::Enter => self.confirm_folder_modal(),
            KeyCode::Esc => {
                // Active selections are already synced to filter; simply close modal and recompute.
                self.folder_modal = None;
                self.mode = UiMode::Table;
                self.recompute();
            }
            _ => {}
        }
    }

    fn refresh_folder_modal_labels(&mut self) {
        // Keep active selections preserved in `filter` before reordering the visible lists.
        self.sync_folder_selection_to_filter();
        self.rebuild_folder_visible();
        let mut pre = HashSet::new();
        for (vis_i, &all_i) in self.folder_visible.iter().enumerate() {
            if self.filter.folders.contains(&self.all_folders[all_i]) {
                pre.insert(vis_i);
            }
        }
        let labels: Vec<String> = self
            .folder_visible
            .iter()
            .map(|&i| self.all_folders[i].clone())
            .collect();
        self.folder_modal = Some(ModalState::new(labels, pre));
    }
}

// ---- Render ----

/// Agent multi-select modal dialog. Uses consistent styling matching deletion confirm dialog.
pub(crate) fn draw_agent_modal(f: &mut Frame, app: &App) {
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
pub(crate) fn draw_folder_modal(f: &mut Frame, app: &App) {
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
    m: &ModalState,
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
