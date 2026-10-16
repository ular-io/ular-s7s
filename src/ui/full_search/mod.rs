//! Full-content search mode on the Session screen (`ctrl+/` or the palette's
//! Full Search Sessions).
//!
//! The prompt sits where the keyword prompt does and shares
//! `UiMode::Keyword` with it while it owns input (`FullSearchState::editing`
//! tells them apart), so the event loop, paste routing, refresh deferral, and
//! pane fading treat both prompts alike. `Enter` starts a run on the
//! `BackgroundState` worker pool (`crate::full_search`) and hands the session
//! table focus at once; while the mode is open the table lists only the run's
//! matches, re-listed by session identity as results stream in. `Esc` stops a
//! running search, and a second `Esc` restores the ordinary list.

pub(crate) mod input;
pub(crate) mod render;
pub(crate) mod state;
#[cfg(test)]
mod tests;
