use super::*;
use crate::model::{Agent, Session};
use std::collections::HashMap;
use std::path::PathBuf;

mod compressed;
mod index;
mod titles;
mod workspace;

fn claude_session(id: &str, title_hint: Option<&str>) -> Session {
    Session {
        agent: Agent::Claude,
        profile_id: String::new(),
        id: id.to_string(),
        source_path: None,
        cwd: "/tmp/demo".into(),
        folder: "demo".to_string(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["첫 질문".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: String::new(),
        assistant_blob: String::new(),
        title_hint: title_hint.map(str::to_string),
        title_fixed: false,
        context_source: None,
    }
}
