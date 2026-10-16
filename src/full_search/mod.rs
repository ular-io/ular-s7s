//! Full-content session search: user text, every assistant message, tool calls,
//! and tool results, beyond the list index the keyword filter reads.
//!
//! A session is read through the shared detailed loader
//! (`session_context::load`), so the content, active-branch rules (rewinds,
//! compaction, Codex segments and compressed rollouts, the Antigravity DB step
//! bound), redaction, and turn numbering are the ones Detail and
//! `s7s session show` use. Before that, `raw` rules out sessions whose
//! transcript cannot contain the query, which keeps most of the store from
//! being decoded at all. `worker` runs both on a bounded thread pool and
//! streams one result per session; nothing is cached between searches.

mod query;
mod raw;
mod snippet;
mod worker;

pub use query::Query;
pub use snippet::SnippetLine;
pub use worker::{spawn, SearchResult};

use crate::model::Session;
use crate::session_context::{self, ContextEntryKind, ContextTurn};
use query::fold;

/// Where in a turn the snippet was taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchField {
    User,
    Entry(ContextEntryKind),
    /// The turn's last assistant text (Detail's answer).
    Answer,
}

impl MatchField {
    pub fn label(self) -> &'static str {
        match self {
            MatchField::User => "User",
            MatchField::Entry(kind) => kind.heading(),
            MatchField::Answer => "Answer",
        }
    }
}

/// The match of one session: where its best snippet is and how many texts
/// (user text, work entries, answers) hold at least one token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMatch {
    /// 0-based turn index, aligned with Detail and `session show --turn`.
    pub turn: usize,
    pub field: MatchField,
    pub lines: Vec<SnippetLine>,
    pub hits: usize,
}

/// Searches one session; `None` when some token occurs nowhere in it.
pub fn search_session(session: &Session, query: &Query) -> Option<SessionMatch> {
    if !raw::may_match(session, query) {
        return None;
    }
    match_turns(&session_context::load(session).turns, query)
}

/// Matches `query` over parsed turns. Every token must occur somewhere in the
/// session, not necessarily in one text; the snippet comes from the first text
/// holding the most distinct tokens.
pub(crate) fn match_turns(turns: &[ContextTurn], query: &Query) -> Option<SessionMatch> {
    let mut seen = 0;
    let mut hits = 0;
    let mut best: Option<(u32, usize, MatchField, &str)> = None;
    for (index, turn) in turns.iter().enumerate() {
        let texts = std::iter::once((MatchField::User, turn.user.as_str()))
            .chain(
                turn.entries
                    .iter()
                    .map(|e| (MatchField::Entry(e.kind), e.text.as_str())),
            )
            .chain(
                turn.last_assistant_text
                    .as_deref()
                    .map(|text| (MatchField::Answer, text)),
            );
        for (field, text) in texts {
            let found = query.found_in(&fold(text));
            if found == 0 {
                continue;
            }
            hits += 1;
            seen |= found;
            let count = found.count_ones();
            if best.is_none_or(|(most, ..)| count > most) {
                best = Some((count, index, field, text));
            }
        }
    }
    if seen != query.all() {
        return None;
    }
    let (_, turn, field, text) = best?;
    Some(SessionMatch {
        turn,
        field,
        lines: snippet::build(text, query),
        hits,
    })
}

#[cfg(test)]
mod tests;
