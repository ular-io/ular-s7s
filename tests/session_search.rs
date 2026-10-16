//! End-to-end CLI search checks against an isolated, disposable session store.

use serde_json::json;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Store(PathBuf);

impl Store {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "s7s-cli-search-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join(".config/s7s")).unwrap();
        Self(root)
    }

    fn session(&self, id: &str, day: u8, folder: &str, tool_text: &str) {
        let dir = self.0.join(".claude/projects/test");
        std::fs::create_dir_all(&dir).unwrap();
        let stamp = format!("2026-10-{day:02}T12:00:00Z");
        let records = [
            json!({"type":"user", "uuid":"a", "parentUuid":null,
                "sessionId":id, "cwd":format!("/test/{folder}"), "timestamp":stamp,
                "message":{"role":"user", "content":"request alpha"}}),
            json!({"type":"assistant", "uuid":"b", "parentUuid":"a",
                "timestamp":stamp, "message":{"role":"assistant", "content":[
                    {"type":"tool_use", "id":"t1", "name":"Bash",
                    "input":{"command":"test-command"}}]}}),
            json!({"type":"user", "uuid":"c", "parentUuid":"b",
                "timestamp":stamp, "message":{"role":"user", "content":[
                    {"type":"tool_result", "tool_use_id":"t1", "content":tool_text}]}}),
            json!({"type":"assistant", "uuid":"d", "parentUuid":"c",
                "timestamp":stamp, "message":{"role":"assistant", "content":[
                    {"type":"text", "text":"finished"}]}}),
        ];
        let text = records.iter().map(|r| format!("{r}\n")).collect::<String>();
        std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
    }

    fn search(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_s7s"))
            .env("HOME", &self.0)
            .env("XDG_CACHE_HOME", self.0.join("cache"))
            .args(["session", "search"])
            .args(args)
            .output()
            .unwrap()
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

#[test]
fn full_search_snippets_filters_order_and_limits() {
    let store = Store::new();
    store.session(
        "older-session",
        10,
        "project-a",
        "before\nneedle failure\nafter",
    );
    store.session(
        "newer-session",
        15,
        "project-b",
        "before\n\u{1b}[31mneedle failure\u{1b}[0m\nafter",
    );
    store.session("metadata-session", 16, "metadata-only", "unrelated output");

    let all = success(store.search(&["needle", "failure", "--full", "--limit", "0"]));
    assert!(all.contains("2 full-content match(es)"), "{all}");
    assert!(all.find("newer-session").unwrap() < all.find("older-session").unwrap());
    assert!(all.contains("[Match · Turn 1 · Tool Result ·"), "{all}");
    assert!(all.contains("      before\n      needle failure\n      after"));
    assert!(!all.contains('\u{1b}'));
    assert!(!all.contains("metadata-session"));

    let across_fields = success(store.search(&["alpha", "needle", "--full"]));
    assert!(across_fields.contains("2 full-content match(es)"));
    assert!(across_fields.contains("[Match · Turn 1 · User · 2 hit(s)]"));

    let limited = success(store.search(&["needle", "--full", "--limit", "1"]));
    assert!(limited.contains("showing 1"));
    assert!(limited.contains("newer-session"));
    assert!(!limited.contains("older-session"));

    let filtered = success(store.search(&[
        "--full",
        "needle",
        "--folder",
        "project-a",
        "--folder",
        "absent",
        "--agent",
        "claude",
        "--profile",
        "builtin-claude",
    ]));
    assert!(filtered.contains("older-session"));
    assert!(!filtered.contains("newer-session"));
    assert_eq!(
        success(store.search(&["needle", "--full", "--agent", "codex"])),
        "No sessions matched.\n"
    );
    assert_eq!(success(store.search(&["needle"])), "No sessions matched.\n");
    assert!(success(store.search(&["metadata-only"])).contains("metadata-session"));
    assert_eq!(
        success(store.search(&["metadata-only", "--full"])),
        "No sessions matched.\n"
    );

    let unknown = store.search(&["needle", "--full", "--profile", "unknown"]);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("matches no sessions"));
    assert_eq!(success(unknown), "No sessions matched.\n");
}

#[test]
fn full_search_rejects_invalid_queries_and_options() {
    let store = Store::new();
    let excess = (0..17)
        .map(|i| format!("token{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    for args in [
        vec!["--full"],
        vec!["--full", " \t "],
        vec!["--full", &excess],
        vec!["--full", "needle", "--limit", "bad"],
        vec!["--full", "needle", "--agent", "unknown"],
    ] {
        let output = store.search(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
    }
    let repeated = std::iter::repeat_n("NEEDLE", 17)
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        success(store.search(&["--full", &repeated])),
        "No sessions matched.\n"
    );
}
