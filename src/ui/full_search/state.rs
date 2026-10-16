//! Full-search state: the query being edited and the last submitted run.

use crate::full_search::SessionMatch;
use crate::ui::refresh::SessionKey;
use crate::ui::TextInput;
use std::collections::HashMap;

/// Present while full-search mode is open. The session table then lists only
/// the run's matches (under the ordinary filters and workspace scope).
pub(crate) struct FullSearchState {
    pub(crate) input: TextInput,
    /// The prompt owns input (`UiMode::Keyword`); otherwise the session table
    /// does and the prompt only shows the run's status.
    pub(crate) editing: bool,
    /// `None` until the first query is submitted.
    pub(crate) run: Option<SearchRun>,
    /// Session selected when the mode opened, restored on exit when the
    /// cursor is not on a session the full list still shows.
    pub(crate) origin: Option<SessionKey>,
}

/// One submitted query and the matches streamed in for it so far.
pub(crate) struct SearchRun {
    pub(crate) query: String,
    pub(crate) phase: RunPhase,
    /// Sessions searched so far and in total.
    pub(crate) scanned: usize,
    pub(crate) total: usize,
    pub(crate) matches: HashMap<SessionKey, SessionMatch>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunPhase {
    Running,
    /// Cancelled by `Esc`; the matches found so far stay listed.
    Stopped,
    Done,
}

impl SearchRun {
    pub(crate) fn new(query: String, total: usize) -> Self {
        SearchRun {
            query,
            phase: if total == 0 {
                RunPhase::Done
            } else {
                RunPhase::Running
            },
            scanned: 0,
            total,
            matches: HashMap::new(),
        }
    }

    /// Prompt status, e.g. `[Stopped · 3 matches found · Esc: Back]`. The key
    /// hint is left out while the prompt is being edited, where `Esc` closes
    /// the whole mode instead.
    pub(crate) fn status(&self, with_key_hint: bool) -> String {
        let found = match self.matches.len() {
            1 => "1 match found".to_string(),
            n => format!("{n} matches found"),
        };
        let (phase, key) = match self.phase {
            RunPhase::Running => (
                format!("Searching {}/{}", self.scanned, self.total),
                "Esc: Stop",
            ),
            RunPhase::Stopped => ("Stopped".to_string(), "Esc: Back"),
            RunPhase::Done if self.scanned < self.total => (
                format!("Done · {}/{} searched", self.scanned, self.total),
                "Esc: Back",
            ),
            RunPhase::Done => ("Done".to_string(), "Esc: Back"),
        };
        if with_key_hint {
            format!("[{phase} · {found} · {key}]")
        } else {
            format!("[{phase} · {found}]")
        }
    }
}
