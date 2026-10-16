//! Raw-transcript prefilter: rules out a session by plain substring search over
//! the bytes the detailed parser would read, before paying for JSON decoding.
//!
//! The prefilter may only produce false positives (the detailed match decides).
//! A token takes part only when its parsed form is guaranteed to appear
//! verbatim in the raw JSONL:
//!
//! - ASCII matches after ASCII-lowercasing the raw text. `"` and `\` are
//!   escaped by every JSON writer, and `<`, `>`, `&` by Go's default encoder,
//!   so tokens holding them are skipped.
//! - Non-ASCII characters must have no case (Hangul, CJK, symbols): byte
//!   comparison cannot fold case outside ASCII. All three agents were observed
//!   writing non-ASCII as raw UTF-8, never `\uXXXX`; both the NFC and the NFD
//!   form are tried.
//! - A JSON document embedded as a string (tool arguments) is decoded by the
//!   detailed parsers, so its own `\uXXXX` or `\/` escapes appear as `\\u` or
//!   `\\/` in the raw text. When one is present, the affected tokens skip the
//!   prefilter for that file.
//!
//! A token found in the in-memory `search_blob` (user turns, title, folder)
//! passes too: the detailed load falls back to those user turns when the
//! transcript cannot be parsed in full.

use super::query::Query;
use crate::model::{Agent, Session};
use unicode_normalization::UnicodeNormalization;

/// Whether `session` may contain every query token. `true` whenever the raw
/// source cannot be read: the detailed load then decides on its own.
pub(crate) fn may_match(session: &Session, query: &Query) -> bool {
    let pending: Vec<&str> = query
        .tokens
        .iter()
        .map(String::as_str)
        .filter(|t| eligible(t) && !session.search_blob.contains(t))
        .collect();
    if pending.is_empty() {
        return true;
    }
    let Some(mut raw) = read_raw(session) else {
        return true;
    };
    raw.make_ascii_lowercase();
    let nested_unicode = raw.contains("\\\\u");
    let nested_slash = raw.contains("\\\\/");
    pending.iter().all(|token| {
        (nested_unicode && !token.is_ascii())
            || (nested_slash && token.contains('/'))
            || raw.contains(token)
            || (!token.is_ascii() && raw.contains(token.nfd().collect::<String>().as_str()))
    })
}

fn eligible(token: &str) -> bool {
    token.chars().all(|c| {
        if c.is_ascii() {
            !c.is_ascii_control() && !matches!(c, '"' | '\\' | '<' | '>' | '&')
        } else {
            c.to_lowercase().eq(std::iter::once(c)) && c.to_uppercase().eq(std::iter::once(c))
        }
    })
}

/// The text the agent's detailed parser reads: the Claude JSONL, the Codex
/// logical rollout (rewind history and compressed files included), or the
/// Antigravity transcript.
fn read_raw(session: &Session) -> Option<String> {
    let source = session.source_path.as_deref()?;
    match session.agent {
        Agent::Claude => std::fs::read_to_string(source).ok(),
        Agent::Codex => crate::parser::codex::segments::read_rollout(source).ok(),
        Agent::Antigravity => {
            let path = crate::session_context::antigravity::transcript_path(source, &session.id)?;
            std::fs::read_to_string(path).ok()
        }
    }
}
