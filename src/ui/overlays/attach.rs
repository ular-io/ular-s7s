//! Attach confirmation for a Claude session held by Claude Code's daemon.
//!
//! Resuming such a session with s7s's flagged template exits 1, so Enter asks
//! instead (`App::divert_live_session`). Attaching shares the worker's screen
//! and input with any terminal already attached — which is not observable from
//! outside the daemon — so Cancel is the default. The handover itself runs in
//! `runtime::handover_attach` through `resume::run_attach`.

use crate::agent_status::LiveStatus;
use crate::ui::components::modal::{button_styles, modal_block, render_modal};
use crate::ui::components::text::truncate_w;
use crate::ui::render::centered_fixed_rect;
use crate::ui::{App, UiMode};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Padding, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

// ---- State ----

/// Session waiting for the Attach/Cancel answer.
pub struct PendingAttach {
    pub idx: usize,
    /// Background job id passed to `claude attach`.
    pub job: String,
    pub status: LiveStatus,
}

/// Confirmed attach, taken by the event loop for the terminal handover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachRequest {
    pub idx: usize,
    pub job: String,
}

// ---- Input ----

impl App {
    pub(crate) fn open_attach_confirm(&mut self, idx: usize, job: String, status: LiveStatus) {
        self.pending_attach = Some(PendingAttach { idx, job, status });
        self.attach_ok_focused = false; // Cancel by default: another terminal may be attached.
        self.mode = UiMode::AttachConfirm;
        self.status_msg = None;
    }

    fn close_attach_confirm(&mut self) {
        self.pending_attach = None;
        self.mode = UiMode::Table;
    }

    fn confirm_attach(&mut self) {
        if let Some(p) = self.pending_attach.take() {
            self.attach_request = Some(AttachRequest {
                idx: p.idx,
                job: p.job,
            });
        }
        self.mode = UiMode::Table;
    }

    pub fn on_key_attach_confirm(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                self.attach_ok_focused = !self.attach_ok_focused;
            }
            KeyCode::Enter => {
                if self.attach_ok_focused {
                    self.confirm_attach();
                } else {
                    self.close_attach_confirm();
                }
            }
            KeyCode::Esc => self.close_attach_confirm(),
            _ => {}
        }
    }
}

// ---- Render ----

pub(crate) fn draw_attach_confirm(f: &mut Frame, app: &App) {
    let th = &app.theme;
    let Some(p) = &app.pending_attach else {
        return;
    };
    let (agent, title) = app
        .sessions
        .get(p.idx)
        .map(|s| (s.agent.label().to_string(), s.title()))
        .unwrap_or_else(|| ("?".to_string(), "?".to_string()));

    // Content: title + spacer + 2 notice rows + spacer + 1 hint row = 6 lines;
    // spacer + buttons = 2 lines. Height: borders (2) + top padding (1) + 6 + 2 = 11.
    let area = centered_fixed_rect(76, 11, f.area());
    let block =
        modal_block(" Attach Background Session ", th.warning).padding(Padding::new(1, 1, 1, 0));
    let inner = render_modal(f, area, block, th);
    let inner_w = inner.width as usize;

    let prefix_w = 1 + agent.width() + 2; // "[" + agent + "] "
    let content = vec![
        Line::from(vec![
            Span::styled("[", Style::default().fg(th.dim)),
            Span::styled(
                agent,
                Style::default().fg(th.warning).add_modifier(Modifier::BOLD),
            ),
            Span::styled("] ", Style::default().fg(th.dim)),
            Span::raw(truncate_w(&title, inner_w.saturating_sub(prefix_w))),
        ]),
        Line::from(""),
        Line::from(Span::raw(truncate_w(&p.status.holder_sentence(), inner_w))),
        Line::from(Span::raw(truncate_w(
            "If another terminal is showing it, both share its screen and input.",
            inner_w,
        ))),
        Line::from(""),
        Line::from(Span::styled(
            truncate_w(
                "Back to s7s: ctrl+z, or ← then ctrl+c twice; it keeps running.",
                inner_w,
            ),
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
    f.render_widget(Paragraph::new(content), rows[0]);

    let (focused_style, unfocused) = button_styles(th);
    let (cancel_style, attach_style) = if app.attach_ok_focused {
        (unfocused, focused_style)
    } else {
        (focused_style, unfocused)
    };
    let buttons = Line::from(vec![
        Span::styled("  Attach  ", attach_style),
        Span::raw("     "),
        Span::styled("  Cancel  ", cancel_style),
    ]);
    f.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        rows[2],
    );
}
