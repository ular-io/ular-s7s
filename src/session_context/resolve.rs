//! Exact source-session resolution across agent and profile boundaries.
//!
//! Account-safety principle (same as rename): resolution succeeds only when
//! exactly one session matches, and a requested-but-missing profile is an error —
//! never a silent fallback to another profile.

use crate::model::{Agent, Session};

/// Resolution constraints. `session_id` must be the full ID.
pub struct Query<'a> {
    pub session_id: &'a str,
    pub agent: Option<Agent>,
    pub profile_id: Option<&'a str>,
}

/// One ambiguous candidate (shown so the caller can disambiguate).
#[derive(Debug, Clone)]
pub struct Candidate {
    pub agent: Agent,
    pub profile_id: String,
    pub title: String,
}

#[derive(Debug)]
pub enum ResolveError {
    NotFound,
    Ambiguous(Vec<Candidate>),
}

/// Parses an `--agent` value. The CLI layer validates values up front, but this
/// stays lenient for reuse.
pub fn parse_agent(s: &str) -> Option<Agent> {
    match s.trim().to_ascii_lowercase().as_str() {
        "claude" => Some(Agent::Claude),
        "codex" => Some(Agent::Codex),
        "antigravity" | "agy" => Some(Agent::Antigravity),
        _ => None,
    }
}

/// Finds exactly one session matching the query. Zero matches -> `NotFound`;
/// more than one -> `Ambiguous` with every candidate's agent/profile.
pub fn resolve<'s>(sessions: &'s [Session], q: &Query) -> Result<&'s Session, ResolveError> {
    let mut matches: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.id == q.session_id)
        .filter(|s| q.agent.is_none_or(|a| s.agent == a))
        .filter(|s| q.profile_id.is_none_or(|p| s.profile_id == p))
        .collect();
    match matches.len() {
        0 => Err(ResolveError::NotFound),
        1 => Ok(matches.remove(0)),
        _ => Err(ResolveError::Ambiguous(
            matches
                .iter()
                .map(|s| Candidate {
                    agent: s.agent,
                    profile_id: s.profile_id.clone(),
                    title: s.title(),
                })
                .collect(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn session(agent: Agent, profile: &str, id: &str) -> Session {
        Session {
            agent,
            profile_id: profile.to_string(),
            id: id.to_string(),
            source_path: None,
            cwd: PathBuf::from("/tmp/demo"),
            folder: "demo".to_string(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["질문".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: String::new(),
            assistant_blob: String::new(),
            title_hint: None,
            title_fixed: false,
        }
    }

    #[test]
    fn unique_id_resolves() {
        let sessions = vec![
            session(Agent::Claude, "builtin-claude", "aaa"),
            session(Agent::Codex, "builtin-codex", "bbb"),
        ];
        let q = Query {
            session_id: "bbb",
            agent: None,
            profile_id: None,
        };
        let s = resolve(&sessions, &q).expect("resolve");
        assert_eq!(s.agent, Agent::Codex);
    }

    #[test]
    fn unknown_id_is_not_found() {
        let sessions = vec![session(Agent::Claude, "builtin-claude", "aaa")];
        let q = Query {
            session_id: "zzz",
            agent: None,
            profile_id: None,
        };
        assert!(matches!(
            resolve(&sessions, &q),
            Err(ResolveError::NotFound)
        ));
    }

    #[test]
    fn duplicate_id_requires_disambiguation() {
        // Same ID visible through two profiles (e.g. shared config folder).
        let sessions = vec![
            session(Agent::Claude, "builtin-claude", "aaa"),
            session(Agent::Claude, "profile-team", "aaa"),
        ];
        let q = Query {
            session_id: "aaa",
            agent: None,
            profile_id: None,
        };
        match resolve(&sessions, &q) {
            Err(ResolveError::Ambiguous(c)) => assert_eq!(c.len(), 2),
            other => panic!("expected ambiguity, got {other:?}"),
        }
        // Profile constraint disambiguates.
        let q = Query {
            session_id: "aaa",
            agent: None,
            profile_id: Some("profile-team"),
        };
        assert_eq!(resolve(&sessions, &q).unwrap().profile_id, "profile-team");
    }

    #[test]
    fn agent_constraint_filters() {
        let sessions = vec![
            session(Agent::Claude, "builtin-claude", "aaa"),
            session(Agent::Codex, "builtin-codex", "aaa"),
        ];
        let q = Query {
            session_id: "aaa",
            agent: Some(Agent::Codex),
            profile_id: None,
        };
        assert_eq!(resolve(&sessions, &q).unwrap().agent, Agent::Codex);
    }

    #[test]
    fn parse_agent_values() {
        assert_eq!(parse_agent("claude"), Some(Agent::Claude));
        assert_eq!(parse_agent("Codex"), Some(Agent::Codex));
        assert_eq!(parse_agent("antigravity"), Some(Agent::Antigravity));
        assert_eq!(parse_agent("agy"), Some(Agent::Antigravity));
        assert_eq!(parse_agent("gpt"), None);
    }
}
