//! Bookmark CLI checks against disposable agent and s7s stores.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Store(PathBuf);

impl Store {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "s7s-cli-bookmarks-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join(".config/s7s")).unwrap();
        Self(root)
    }

    fn session(&self, profile_root: &str, id: &str, day: u8, folder: &str) -> PathBuf {
        let dir = self.0.join(profile_root).join("projects/test");
        std::fs::create_dir_all(&dir).unwrap();
        let record = json!({"type":"user", "uuid":"a", "parentUuid":null,
            "sessionId":id, "cwd":format!("/test/{folder}"),
            "timestamp":format!("2026-10-{day:02}T12:00:00Z"),
            "message":{"role":"user", "content":format!("Request for {id}")}});
        let path = dir.join(format!("{id}.jsonl"));
        std::fs::write(&path, format!("{record}\n")).unwrap();
        path
    }

    fn command(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_s7s"))
            .env("HOME", &self.0)
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .args(["session", "bookmark"])
            .args(args)
            .output()
            .unwrap()
    }

    fn path(&self) -> PathBuf {
        self.0.join(".config/s7s/bookmarks.json")
    }

    fn entries(&self) -> Vec<Value> {
        let data: Value = serde_json::from_slice(&std::fs::read(self.path()).unwrap()).unwrap();
        data["entries"].as_array().unwrap().clone()
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn failure(output: Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(message), "{error}");
}

#[test]
fn add_remove_are_idempotent_and_preserve_transcripts_and_other_bookmarks() {
    let store = Store::new();
    let transcript = store.session(".claude", "first-session", 10, "project-a");
    store.session(".claude", "second-session", 11, "project-b");
    let before = std::fs::read(&transcript).unwrap();
    assert_eq!(success(store.command(&["list"])), "No sessions matched.\n");
    success(store.command(&["remove", "first-session"]));
    assert!(!store.path().exists());

    for _ in 0..2 {
        success(store.command(&["add", "first-session", "--agent", "claude"]));
    }
    assert_eq!(
        store.entries(),
        [json!({"agent":"claude","profile":"builtin-claude","session":"first-session"})]
    );
    success(store.command(&["add", "second-session"]));
    for _ in 0..2 {
        success(store.command(&["remove", "first-session", "--profile", "builtin-claude"]));
    }
    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.entries()[0]["session"], "second-session");
    assert_eq!(std::fs::read(transcript).unwrap(), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(store.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn list_filters_bookmarks_before_limiting_and_preserves_activity_order() {
    let store = Store::new();
    store.session(".claude", "older-session", 10, "project-a");
    store.session(".claude", "newer-session", 11, "project-b");
    store.session(".claude", "unmarked-session", 12, "project-a");
    success(store.command(&["add", "newer-session"]));
    success(store.command(&["add", "older-session"]));

    let all = success(store.command(&["list", "--limit", "0"]));
    assert!(all.contains("2 bookmarked session(s)"), "{all}");
    assert!(all.find("newer-session").unwrap() < all.find("older-session").unwrap());
    assert!(all.contains("Request for newer-session"));
    assert!(!all.contains("unmarked-session"));
    assert!(!all.contains('♥'));
    assert!(!all.contains('\u{1b}'));

    let limited = success(store.command(&["list", "--limit", "1"]));
    assert!(limited.contains("showing 1"));
    assert!(limited.contains("newer-session"));
    assert!(!limited.contains("older-session"));
    let filtered = success(store.command(&[
        "list",
        "--folder",
        "project-a",
        "--folder",
        "absent",
        "--agent",
        "claude",
        "--agent",
        "codex",
        "--profile",
        "builtin-claude",
        "--limit",
        "1",
    ]));
    assert!(filtered.contains("older-session"));
    assert!(!filtered.contains("newer-session"));
    assert_eq!(
        success(store.command(&["list", "--agent", "codex"])),
        "No sessions matched.\n"
    );
    let unknown = store.command(&["list", "--profile", "unknown"]);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("matches no sessions"));
    assert_eq!(success(unknown), "No sessions matched.\n");

    // A temporarily unavailable transcript must not discard its bookmark.
    std::fs::remove_dir_all(store.0.join(".claude/projects")).unwrap();
    assert_eq!(success(store.command(&["list"])), "No sessions matched.\n");
    assert_eq!(store.entries().len(), 2);
}

#[test]
fn ambiguous_and_missing_targets_do_not_modify_bookmarks() {
    let store = Store::new();
    store.session(".claude", "shared-session", 10, "project-a");
    store.session("team", "shared-session", 11, "project-b");
    std::fs::write(
        store.0.join(".config/s7s/profiles.json"),
        json!({"version":1,"profiles":[{"id":"team","agent":"Claude","name":"Team",
            "path":store.0.join("team"),"active":true,"builtin":false}]})
        .to_string(),
    )
    .unwrap();
    failure(
        store.command(&["add", "shared-session"]),
        1,
        "matches 2 sessions",
    );
    failure(
        store.command(&["add", "missing-session"]),
        1,
        "no session found",
    );
    failure(
        store.command(&["add", "shared-session", "--profile", "missing"]),
        1,
        "does not exist",
    );
    failure(
        store.command(&["add", "shared-session", "--agent", "codex"]),
        1,
        "no session found",
    );
    assert!(!store.path().exists());

    success(store.command(&["add", "shared-session", "--profile", "team"]));
    success(store.command(&["add", "shared-session", "--profile", "builtin-claude"]));
    success(store.command(&["remove", "shared-session", "--profile", "team"]));
    assert_eq!(store.entries().len(), 1);
    assert_eq!(store.entries()[0]["profile"], "builtin-claude");
}

#[test]
fn invalid_storage_and_write_failures_are_reported_without_success_output() {
    let store = Store::new();
    store.session(".claude", "test-session", 10, "project-a");
    for data in ["invalid json", r#"{"version":999,"entries":[]}"#] {
        std::fs::write(store.path(), data).unwrap();
        for args in [
            vec!["add", "test-session"],
            vec!["remove", "test-session"],
            vec!["list"],
        ] {
            let output = store.command(&args);
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            assert!(!output.stderr.is_empty());
            assert_eq!(std::fs::read_to_string(store.path()).unwrap(), data);
        }
    }
    std::fs::remove_file(store.path()).unwrap();
    std::fs::create_dir(store.path()).unwrap();
    failure(
        store.command(&["add", "test-session"]),
        1,
        "cannot update bookmark",
    );
    assert!(store.path().is_dir());
}

#[test]
fn invalid_commands_and_options_exit_two() {
    let store = Store::new();
    for args in [
        vec![],
        vec!["add"],
        vec!["remove"],
        vec!["toggle", "id"],
        vec!["add", "id", "--agent", "unknown"],
        vec!["list", "--limit", "bad"],
        vec!["list", "unexpected"],
        vec!["list", "--agent", "unknown"],
    ] {
        let output = store.command(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    assert!(!store.path().exists());
}
