//! Composite filter: keyword (body + title + folder) AND agent AND folder AND profile.
//!
//! All conditions are joined with AND. Empty conditions are treated as "match all".
//! Keywords are matched against the precomputed `search_blob` (user turns + title + folder name) per
//! session, avoiding runtime overhead during search. Tokens not found there fall back to the
//! `assistant_blob` (each turn's last assistant answer), then to a partial match of the session id
//! (if the token is at least [`ID_SEARCH_MIN_LEN`] long). A multi-token query can be satisfied by
//! finding some tokens in the user body/title and others in past assistant answers.

use crate::model::{Agent, Session};
use crate::normalize;
use std::collections::HashSet;

/// Minimum token length (in bytes) to attempt session id matching.
/// Prevents false positives where short hex tokens accidentally match part of a UUID.
const ID_SEARCH_MIN_LEN: usize = 5;

/// Current filter state.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Raw keyword string. All whitespace-separated tokens must be present in the body or title (AND).
    pub keyword: String,
    /// Selected agents. If empty, matches all.
    pub agents: HashSet<Agent>,
    /// Selected folders. If empty, matches all.
    pub folders: HashSet<String>,
    /// Selected profile IDs. If empty, matches all (configured via header number keys `1..5`).
    pub profile_ids: HashSet<String>,
}

impl Filter {
    /// Returns true if the session satisfies all active filter conditions.
    pub fn matches(&self, s: &Session) -> bool {
        // Agent filter
        if !self.agents.is_empty() && !self.agents.contains(&s.agent) {
            return false;
        }
        // Folder filter
        if !self.folders.is_empty() && !self.folders.contains(&s.folder) {
            return false;
        }
        // Profile filter
        if !self.profile_ids.is_empty() && !self.profile_ids.contains(&s.profile_id) {
            return false;
        }
        // Keyword filter (AND tokens) - search_blob first, falls back to session id matching
        if !self.keyword.trim().is_empty() {
            let needle = normalize::nfc_lower(&self.keyword);
            for token in needle.split_whitespace() {
                if s.search_blob.contains(token) {
                    continue;
                }
                // Assistant answers are a secondary target: a token found only in a
                // past answer still matches (AND semantics across tokens preserved).
                if s.assistant_blob.contains(token) {
                    continue;
                }
                if token.len() >= ID_SEARCH_MIN_LEN && s.id.to_ascii_lowercase().contains(token) {
                    continue;
                }
                return false;
            }
        }
        true
    }

    /// Returns true if at least one filter condition is active.
    pub fn is_active(&self) -> bool {
        !self.keyword.trim().is_empty()
            || !self.agents.is_empty()
            || !self.folders.is_empty()
            || !self.profile_ids.is_empty()
    }

    /// Brief description of active filters for the table title `sessions[<describe>: N]`.
    /// The profile ID is resolved to its display name via `resolve`. Returns empty string if no filters are active.
    pub fn describe_with(&self, resolve: impl Fn(&str) -> Option<String>) -> String {
        let mut parts = Vec::new();
        if !self.keyword.trim().is_empty() {
            parts.push(self.keyword.trim().to_string());
        }
        if !self.agents.is_empty() {
            let mut a: Vec<&str> = self.agents.iter().map(|x| x.key()).collect();
            a.sort_unstable();
            parts.push(a.join("+"));
        }
        if !self.folders.is_empty() {
            parts.push(format!("{} folders", self.folders.len()));
        }
        if !self.profile_ids.is_empty() {
            let mut names: Vec<String> = self
                .profile_ids
                .iter()
                .map(|id| resolve(id).unwrap_or_else(|| id.clone()))
                .collect();
            names.sort_unstable();
            parts.push(names.join("+"));
        }
        parts.join(", ")
    }
}

/// Filters the session list and returns indices of matched sessions, preserving original order.
pub fn apply(sessions: &[Session], filter: &Filter) -> Vec<usize> {
    sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| filter.matches(s))
        .map(|(i, _)| i)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, blob: &str) -> Session {
        Session {
            agent: Agent::Claude,
            profile_id: String::new(),
            id: id.to_string(),
            source_path: None,
            cwd: "/tmp/demo".into(),
            folder: "demo".to_string(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: Vec::new(),
            user_turn_timestamps_ms: Vec::new(),
            search_blob: blob.to_string(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        }
    }

    fn keyword(kw: &str) -> Filter {
        Filter {
            keyword: kw.to_string(),
            ..Filter::default()
        }
    }

    #[test]
    fn keyword_matches_session_id_when_token_is_long_enough() {
        let s = session("019f36e8-9157-7c63-bee8-8937a6314982", "본문 텍스트");
        assert!(keyword("019f3").matches(&s));
        assert!(keyword("8937a6314982").matches(&s));
        // AND combination of body token and id token also matches
        assert!(keyword("본문 019f36e8").matches(&s));
    }

    #[test]
    fn keyword_shorter_than_threshold_does_not_match_session_id() {
        let s = session("019f36e8-9157-7c63-bee8-8937a6314982", "본문 텍스트");
        // Tokens of 4 chars or less do not match session ID (prevention of false positives).
        assert!(!keyword("019f").matches(&s));
        assert!(!keyword("bee8").matches(&s));
        // Matches search_blob regardless of length.
        assert!(keyword("본문").matches(&s));
    }

    #[test]
    fn keyword_id_match_is_ascii_case_insensitive() {
        let s = session("019F36E8-9157-7C63-BEE8-8937A6314982", "");
        assert!(keyword("019f36e8").matches(&s));
    }

    #[test]
    fn keyword_matches_assistant_blob() {
        let mut s = session("id", "질문 본문");
        s.assistant_blob = "sqlite 전환은 필요 없습니다".to_string();
        // Token present only in the assistant answer still matches.
        assert!(keyword("sqlite").matches(&s));
        // Token in neither blob nor id fails.
        assert!(!keyword("존재하지않는키워드").matches(&s));
    }

    #[test]
    fn keyword_and_across_user_and_assistant_blobs() {
        let mut s = session("id", "질문 본문");
        s.assistant_blob = "최종 답변 텍스트".to_string();
        // One token from the user body, one from the assistant answer: AND holds.
        assert!(keyword("질문 답변").matches(&s));
        // Both tokens must be found somewhere; a missing one fails the whole query.
        assert!(!keyword("질문 없는토큰").matches(&s));
    }
}
