//! Shared unit-test fixtures for the `ui` feature modules: key-event helpers and
//! `App` builders (empty, single-session, deletable-sessions, custom-cwd, and the
//! multi-profile setup). Kept in one place so the per-feature `tests` submodules
//! (`session`, `detail`, `profile`, `new_session`, `overlays`, `quick`) reuse the
//! same setup instead of each rebuilding it.

use crate::config::Config;
use crate::model::{Agent, Session};
use crate::ui::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

pub(crate) fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

/// Returns an empty mock ProfileStore for unit testing (prevents disk scanning / file saving).
pub(crate) fn test_profiles() -> crate::profile::ProfileStore {
    crate::profile::ProfileStore {
        profiles: Vec::new(),
    }
}

pub(crate) fn empty_app() -> App {
    App::new(
        Config::load(),
        test_profiles(),
        Vec::new(),
        "0 sessions · reparsed 0/0".to_string(),
    )
}

pub(crate) fn app_with_session() -> App {
    App::new(
        Config::load(),
        test_profiles(),
        vec![Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: "session-1".to_string(),
            source_path: None,
            cwd: PathBuf::from("/tmp"),
            folder: "tmp".to_string(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hello".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: "hello".to_string(),
            assistant_blob: String::new(),
            title_hint: Some("hello".to_string()),
            title_fixed: false,
        }],
        "1 sessions · reparsed 0/0".to_string(),
    )
}

/// Instantiates App with two sessions linked to temporary source files (for testing deletion).
pub(crate) fn app_with_two_deletable_sessions() -> (App, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "s7s-ui-delete-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("temp dir");
    let make = |name: &str| {
        let path = root.join(name);
        std::fs::write(&path, "{}").expect("write source");
        Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: name.to_string(),
            source_path: Some(path),
            cwd: PathBuf::from("/tmp"),
            folder: "tmp".to_string(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hello".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: "hello".to_string(),
            assistant_blob: String::new(),
            title_hint: Some(name.to_string()),
            title_fixed: false,
        }
    };
    let app = App::new(
        Config::load(),
        test_profiles(),
        vec![make("s1.jsonl"), make("s2.jsonl")],
        "2 sessions".to_string(),
    );
    (app, root)
}

pub(crate) fn app_with_cwd(cwd: &str) -> App {
    App::new(
        Config::load(),
        test_profiles(),
        vec![Session {
            agent: Agent::Codex,
            profile_id: String::new(),
            id: "session-1".to_string(),
            source_path: None,
            cwd: PathBuf::from(cwd),
            folder: "x".to_string(),
            mtime_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hi".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: "hi".to_string(),
            assistant_blob: String::new(),
            title_hint: Some("hi".to_string()),
            title_fixed: false,
        }],
        "1 sessions · reparsed 0/0".to_string(),
    )
}

/// Returns App with a built-in Claude profile, one custom profile, and one session per profile.
pub(crate) fn app_with_profiles() -> App {
    use crate::profile::{Profile, ProfileStore};
    let profiles = ProfileStore {
        profiles: vec![
            Profile {
                id: "builtin-claude".to_string(),
                agent: Agent::Claude,
                name: "Claude".to_string(),
                path: PathBuf::from("/tmp"),
                oauth_token: None,
                active: true,
                shortcut: Some(1),
                builtin: true,
            },
            Profile {
                id: "profile-x".to_string(),
                agent: Agent::Claude,
                name: "Team".to_string(),
                path: PathBuf::from("/tmp/team"),
                oauth_token: None,
                active: true,
                shortcut: Some(2),
                builtin: false,
            },
        ],
    };
    let session = |pid: &str, id: &str, cwd: &str| Session {
        agent: Agent::Claude,
        profile_id: pid.to_string(),
        id: id.to_string(),
        source_path: None,
        cwd: PathBuf::from(cwd),
        folder: PathBuf::from(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd.to_string()),
        mtime_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["hi".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: "hi".to_string(),
        assistant_blob: String::new(),
        title_hint: None,
        title_fixed: false,
    };
    App::new(
        Config::load(),
        profiles,
        vec![
            session("builtin-claude", "s1", "/"),
            session("profile-x", "s2", "/tmp"),
        ],
        "2 sessions · reparsed 0/0".to_string(),
    )
}
