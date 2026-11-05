use super::*;

const DIR: &str = "/Users/x/.config/s7s/probe";

/// Shape of claude 2.1.318 `~/.claude.json`: two-space indent, no trailing newline,
/// a nested object that also has a `projects` key, non-ASCII text.
const CLAUDE: &str = r#"{
  "numStartups": 5900,
  "cachedStats": {
    "projects": {}
  },
  "projects": {
    "/Users/x": {
      "hasTrustDialogAccepted": true,
      "lastSessionFirstPrompt": "세션 이름 바꿔줘",
      "lastCost": 0.25
    }
  },
  "theme": "light"
}"#;

/// Shape of agy 1.5.1 `settings.json`: two-space indent, trailing newline.
const AGY: &str = "{\n  \"colorScheme\": \"solarized light\",\n  \"trustedWorkspaces\": [\n    \"/Users/x/Script\"\n  ]\n}\n";

/// Shape of codex 0.166.0 `config.toml`, with a comment that must survive.
const CODEX: &str = "model = \"gpt-6\"\n# keep me\n\n[projects.\"/Users/x\"]\ntrust_level = \"trusted\"\n\n[tui]\nscreen_reader_detection_done = true\n";

/// The edit only inserts `inserted`: removing it once gives back the original bytes.
fn assert_only_inserted(original: &str, updated: &str, inserted: &str) {
    assert_eq!(updated.len(), original.len() + inserted.len());
    let at = updated.find(inserted).expect("inserted text present");
    assert_eq!(
        format!("{}{}", &updated[..at], &updated[at + inserted.len()..]),
        original
    );
}

#[test]
fn claude_record_is_spliced_into_top_level_projects() {
    let updated = claude_with_trust(CLAUDE, DIR).unwrap();
    assert_only_inserted(
        CLAUDE,
        &updated,
        "\n    \"/Users/x/.config/s7s/probe\": {\n      \"hasTrustDialogAccepted\": true\n    },",
    );
    let parsed: serde_json::Value = serde_json::from_str(&updated).unwrap();
    assert_eq!(parsed["projects"][DIR]["hasTrustDialogAccepted"], true);
    // The nested `projects` key was not the one edited.
    assert_eq!(parsed["cachedStats"]["projects"], serde_json::json!({}));
    assert!(updated.contains("세션 이름 바꿔줘"));
}

#[test]
fn claude_existing_record_or_unknown_layout_is_left_alone() {
    let trusted = claude_with_trust(CLAUDE, DIR).unwrap();
    assert_eq!(claude_with_trust(&trusted, DIR), None);
    // A record that is not trusted is the CLI's (or the user's) decision.
    let declined = CLAUDE.replace("\"/Users/x\"", &format!("\"{DIR}\""));
    let declined = declined.replace("true", "false");
    assert_eq!(claude_with_trust(&declined, DIR), None);
    assert_eq!(claude_with_trust("{\"numStartups\": 1}", DIR), None);
    assert_eq!(claude_with_trust("not json", DIR), None);
}

#[test]
fn claude_empty_projects_gets_a_single_member() {
    let text = "{\n  \"projects\": {}\n}";
    let updated = claude_with_trust(text, DIR).unwrap();
    assert_eq!(
        updated,
        "{\n  \"projects\": {\n    \"/Users/x/.config/s7s/probe\": {\n      \"hasTrustDialogAccepted\": true\n    }\n  }\n}"
    );
}

#[test]
fn agy_workspace_is_added_once() {
    let updated = agy_with_trust(AGY, DIR).unwrap();
    assert_only_inserted(AGY, &updated, "\n    \"/Users/x/.config/s7s/probe\",");
    assert_eq!(agy_with_trust(&updated, DIR), None);
    let empty = "{\n  \"trustedWorkspaces\": []\n}\n";
    assert_eq!(
        agy_with_trust(empty, DIR).unwrap(),
        "{\n  \"trustedWorkspaces\": [\n    \"/Users/x/.config/s7s/probe\"\n  ]\n}\n"
    );
    assert_eq!(agy_with_trust("{\"editor\": \"zed\"}\n", DIR), None);
}

#[test]
fn codex_table_is_appended_and_comments_survive() {
    let updated = codex_with_trust(CODEX, DIR).unwrap();
    assert_only_inserted(
        CODEX,
        &updated,
        "\n[projects.\"/Users/x/.config/s7s/probe\"]\ntrust_level = \"trusted\"\n",
    );
    assert!(updated.starts_with(CODEX));
    assert_eq!(codex_with_trust(&updated, DIR), None);
    // No trailing newline, an empty file, and a key that needs escaping.
    let bare = codex_with_trust("model = \"gpt-6\"", DIR).unwrap();
    assert!(bare.starts_with("model = \"gpt-6\"\n\n[projects."));
    assert!(codex_with_trust("", DIR).unwrap().starts_with("[projects."));
    let odd = "/tmp/a \"b\" \\c";
    let escaped = codex_with_trust(CODEX, odd).unwrap();
    let parsed: toml::Table = escaped.parse().unwrap();
    assert_eq!(
        parsed["projects"][odd]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn codex_existing_or_inline_projects_is_left_alone() {
    let untrusted = format!("[projects.\"{DIR}\"]\ntrust_level = \"untrusted\"\n");
    assert_eq!(codex_with_trust(&untrusted, DIR), None);
    // An inline `projects` table cannot be extended by a header: leave it.
    let inline = "projects = { \"/Users/x\" = { trust_level = \"trusted\" } }\n";
    assert_eq!(codex_with_trust(inline, DIR), None);
    assert_eq!(codex_with_trust("not = = toml", DIR), None);
}

#[test]
fn record_write_keeps_mode_and_symlink_and_runs_once() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = std::env::temp_dir().join(format!("s7s-trust-record-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let real = root.join("real.json");
    let link = root.join(".claude.json");
    fs::write(&real, CLAUDE).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&real, &link).unwrap();

    add_record(&link, DIR, claude_with_trust).unwrap();
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    let written = fs::read_to_string(&real).unwrap();
    assert_eq!(Some(written.clone()), claude_with_trust(CLAUDE, DIR));
    let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);

    let modified = fs::metadata(&real).unwrap().modified().unwrap();
    add_record(&link, DIR, claude_with_trust).unwrap();
    assert_eq!(fs::metadata(&real).unwrap().modified().unwrap(), modified);
    // No temp file left behind.
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    assert!(add_record(&root.join("missing.json"), DIR, claude_with_trust).is_err());
    fs::remove_dir_all(&root).unwrap();
}
