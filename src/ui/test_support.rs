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

/// Isolated durable bookmark storage for tests; never writes user configuration.
pub(crate) struct TempBookmarkStore {
    pub path: PathBuf,
}

impl TempBookmarkStore {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "s7s-bookmark-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        Self {
            path: root.join("bookmarks.json"),
        }
    }
}

impl Drop for TempBookmarkStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().expect("temp dir"));
    }
}

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
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hello".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: "hello".to_string(),
            assistant_blob: String::new(),
            title_hint: Some("hello".to_string()),
            title_fixed: false,
            context_source: None,
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
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hello".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: "hello".to_string(),
            assistant_blob: String::new(),
            title_hint: Some(name.to_string()),
            title_fixed: false,
            context_source: None,
        }
    };
    let mut app = App::new(
        Config::load(),
        test_profiles(),
        vec![make("s1.jsonl"), make("s2.jsonl")],
        "2 sessions".to_string(),
    );
    app.bookmarks_path = root.join("bookmarks.json");
    (app, root)
}

/// Three sessions forming a context chain: `root` <- `middle` <- `leaf`, each in
/// its own folder so folder filtering can hide one from another. The list order
/// is leaf, middle, root (indices 0, 1, 2).
pub(crate) fn app_with_context_chain() -> App {
    use crate::model::ContextSource;
    let session = |id: &str, source: Option<&str>| Session {
        agent: Agent::Codex,
        profile_id: "p1".to_string(),
        id: id.to_string(),
        source_path: None,
        cwd: PathBuf::from(format!("/tmp/{id}")),
        folder: id.to_string(),
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec![format!("{id} question")],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: format!("{id} question"),
        assistant_blob: String::new(),
        title_hint: Some(id.to_string()),
        title_fixed: false,
        context_source: source.map(|src| ContextSource {
            id: src.to_string(),
            agent: Agent::Codex,
            profile: "p1".to_string(),
        }),
    };
    App::new(
        Config::load(),
        test_profiles(),
        vec![
            session("leaf", Some("middle")),
            session("middle", Some("root")),
            session("root", None),
        ],
        "3 sessions · reparsed 0/0".to_string(),
    )
}

/// `app_with_context_chain` with distinct activity times: middle is the newest,
/// then root, then leaf, so latest-activity order differs from name order.
pub(crate) fn app_with_dated_folders() -> App {
    let mut app = app_with_context_chain();
    for s in &mut app.sessions {
        s.updated_at_ms = match s.id.as_str() {
            "middle" => 3,
            "root" => 2,
            _ => 1,
        };
    }
    app.rebuild_all_folders();
    app
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
            updated_at_ms: 0,
            ctime_ms: 0,
            size_bytes: 0,
            user_turns: vec!["hi".to_string()],
            user_turn_timestamps_ms: Vec::new(),
            search_blob: "hi".to_string(),
            assistant_blob: String::new(),
            title_hint: Some("hi".to_string()),
            title_fixed: false,
            context_source: None,
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
        updated_at_ms: 0,
        ctime_ms: 0,
        size_bytes: 0,
        user_turns: vec!["hi".to_string()],
        user_turn_timestamps_ms: Vec::new(),
        search_blob: "hi".to_string(),
        assistant_blob: String::new(),
        title_hint: None,
        title_fixed: false,
        context_source: None,
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
