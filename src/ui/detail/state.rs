//! Session detail screen state, focus model, and the pure transition helpers
//! (question selection and right-panel scroll) driven by key handling.

/// Focused column in the session details screen (left questions list <-> right workspace task details). Toggled via ←/→.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailFocus {
    /// Left column: list of user questions (navigated via ↑/↓).
    Questions,
    /// Right column: detailed agent workspace tasks and answers (scrolled via ↑/↓).
    Work,
}

/// State for the session details screen, instantiated on entering from the search preview screen via →.
pub struct SessionDetailState {
    /// Target session index in the `sessions` vector.
    pub session_idx: usize,
    /// List of handoff turns parsed from raw session files.
    pub turns: Vec<crate::handoff::HandoffTurn>,
    /// Selected turn index in the left questions panel.
    pub selected: usize,
    /// The single turn whose prompt is expanded to full length in the left panel,
    /// bypassing the `preview_turn_lines` omission. At most one turn is expanded at a
    /// time — expanding another collapses the previous one. Toggled via `.` while the
    /// Prompt panel is focused (ignored for turns short enough to show in full already).
    pub expanded_prompt: Option<usize>,
    pub focus: DetailFocus,
    /// Scroll offset for the left questions list (lines). Adjusted in render to keep the
    /// selected item visible; also driven manually by ↑/↓ while the expanded prompt
    /// overflows the panel (see `left_scrollable`).
    pub left_scroll: std::cell::Cell<u16>,
    /// Whether the expanded prompt is taller than the panel viewport, so ↑/↓ scroll the
    /// panel instead of moving the turn selection. Recomputed every render pass.
    pub left_scrollable: std::cell::Cell<bool>,
    /// Lower/upper bounds for `left_scroll` while the expanded prompt overflows, so the
    /// scroll stays within the expanded turn's block. Recomputed every render pass.
    pub left_scroll_min: std::cell::Cell<u16>,
    pub left_scroll_max: std::cell::Cell<u16>,
    /// Scroll offset for the right details panel (lines).
    pub right_scroll: std::cell::Cell<u16>,
    /// Maximum scroll limit for the right details panel. Calculated and updated in the render pass post-wrapping.
    pub right_max_scroll: std::cell::Cell<u16>,
}

impl SessionDetailState {
    /// Moves selection in the left questions list. Resets the right details panel's scroll offset on changes.
    pub(crate) fn move_selection(&mut self, delta: isize) {
        if self.turns.is_empty() {
            return;
        }
        let len = self.turns.len() as isize;
        let next = (self.selected as isize + delta).clamp(0, len - 1) as usize;
        if next != self.selected {
            self.selected = next;
            self.right_scroll.set(0);
        }
    }

    /// Scrolls the right details panel (clamped to available scroll range).
    pub(crate) fn scroll_work(&self, delta: isize) {
        let max = self.right_max_scroll.get() as isize;
        let cur = self.right_scroll.get() as isize;
        self.right_scroll.set((cur + delta).clamp(0, max) as u16);
    }

    /// Scrolls the left prompt panel within the expanded turn's block (clamped to the
    /// render-computed bounds). Only meaningful while `left_scrollable` is set.
    pub(crate) fn scroll_prompt(&self, delta: isize) {
        let min = self.left_scroll_min.get() as isize;
        let max = self.left_scroll_max.get() as isize;
        let cur = self.left_scroll.get() as isize;
        self.left_scroll
            .set((cur + delta).clamp(min, max.max(min)) as u16);
    }
}
