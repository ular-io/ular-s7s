use super::*;
use crate::profile::Profile;
use crate::session_workspace::WorkspaceStore;
use crate::ui::test_support::{app_with_session, TempBookmarkStore};
use clap::Parser;

#[derive(Parser)]
struct TestCli {
    #[command(subcommand)]
    command: super::super::SessionCommand,
}

fn session(agent: Agent, profile: &str, id: &str) -> Session {
    let mut s = app_with_session().sessions.remove(0);
    s.agent = agent;
    s.profile_id = profile.to_string();
    s.id = id.to_string();
    s
}

fn profiles() -> ProfileStore {
    ProfileStore {
        profiles: [(Agent::Claude, "claude-a"), (Agent::Codex, "codex-a")]
            .into_iter()
            .map(|(agent, id)| Profile {
                agent,
                id: id.to_string(),
                name: id.to_string(),
                path: PathBuf::from("/unused"),
                oauth_token: None,
                active: true,
                shortcut: None,
                builtin: false,
            })
            .collect(),
    }
}

fn args(to: &Path, ids: &[&str], dry_run: bool) -> ChangeFolderArgs {
    ChangeFolderArgs {
        session_ids: ids.iter().map(|id| id.to_string()).collect(),
        to: to.to_path_buf(),
        agent: None,
        profile: None,
        dry_run,
    }
}

#[test]
fn command_requires_ids_and_destination_and_accepts_multiple_ids() {
    for input in [
        vec!["s7s", "change-folder", "--to", "/tmp"],
        vec!["s7s", "change-folder", "a"],
        vec![
            "s7s",
            "change-folder",
            "a",
            "--to",
            "/tmp",
            "--agent",
            "bad",
        ],
    ] {
        assert!(TestCli::try_parse_from(input).is_err());
    }
    let cli = TestCli::try_parse_from([
        "s7s",
        "change-folder",
        "a",
        "b",
        "--to",
        "./target",
        "--dry-run",
        "--agent",
        "codex",
        "--profile",
        "codex-a",
    ])
    .unwrap();
    let super::super::SessionCommand::ChangeFolder(a) = cli.command else {
        panic!("expected folder change");
    };
    assert_eq!(a.session_ids, ["a", "b"]);
    assert!(a.dry_run);
    assert_eq!(a.profile.as_deref(), Some("codex-a"));
}

#[test]
fn batch_preserves_unrelated_mappings_and_transcripts() {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("session_workspaces.json");
    let folder = root.path.parent().unwrap();
    let transcript = folder.join("transcript.jsonl");
    std::fs::write(&transcript, "historical conversation\n").unwrap();
    let mut index = vec![
        session(Agent::Claude, "claude-a", "a"),
        session(Agent::Codex, "codex-a", "b"),
    ];
    index[0].source_path = Some(transcript.clone());
    crate::session_workspace::record(&store, "other", "untouched", Path::new("/old")).unwrap();
    run_in_index(
        &args(folder, &["a", "b", "a"], false),
        &profiles(),
        &index,
        &store,
    )
    .unwrap();
    let saved = WorkspaceStore::load(&store);
    let folder = std::fs::canonicalize(folder).unwrap();
    assert_eq!(saved.cwd("claude-a", "a"), Some(folder.as_path()));
    assert_eq!(saved.cwd("codex-a", "b"), Some(folder.as_path()));
    assert_eq!(saved.cwd("other", "untouched"), Some(Path::new("/old")));
    assert_eq!(
        targets(&args(&folder, &["a", "b", "a"], false), &index)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        std::fs::read_to_string(transcript).unwrap(),
        "historical conversation\n"
    );
}

#[test]
fn dry_run_neither_creates_a_store_nor_modifies_an_existing_one() {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("session_workspaces.json");
    let folder = root.path.parent().unwrap();
    let index = vec![session(Agent::Claude, "claude-a", "a")];
    let args = args(folder, &["a"], true);
    run_in_index(&args, &profiles(), &index, &store).unwrap();
    assert!(!store.exists());
    assert!(!folder.join(".session_workspaces.json.lock").exists());
    crate::session_workspace::record(&store, "claude-a", "a", Path::new("/old")).unwrap();
    let before = std::fs::read(&store).unwrap();
    run_in_index(&args, &profiles(), &index, &store).unwrap();
    assert_eq!(std::fs::read(store).unwrap(), before);
}

#[test]
fn invalid_batch_never_writes_its_valid_prefix() {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("session_workspaces.json");
    let folder = root.path.parent().unwrap();
    let index = vec![
        session(Agent::Claude, "claude-a", "a"),
        session(Agent::Antigravity, "agy-a", "unsupported"),
        session(Agent::Codex, "codex-a", "ambiguous"),
        session(Agent::Claude, "claude-a", "ambiguous"),
    ];
    crate::session_workspace::record(&store, "claude-a", "a", Path::new("/old")).unwrap();
    let before = std::fs::read(&store).unwrap();
    for id in ["missing", "unsupported", "ambiguous"] {
        let err = run_in_index(
            &args(folder, &["a", id], false),
            &profiles(),
            &index,
            &store,
        );
        assert!(err.is_err(), "{id}");
        assert_eq!(std::fs::read(&store).unwrap(), before);
    }
}

#[test]
fn profile_and_agent_constraints_disambiguate_without_fallback() {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("session_workspaces.json");
    let folder = root.path.parent().unwrap();
    let index = vec![
        session(Agent::Claude, "claude-a", "shared"),
        session(Agent::Codex, "codex-a", "shared"),
    ];
    let mut args = args(folder, &["shared"], false);
    args.profile = Some("absent".to_string());
    assert!(run_in_index(&args, &profiles(), &index, &store).is_err());
    assert!(!store.exists());
    args.profile = Some("claude-a".to_string());
    args.agent = Some("codex".to_string());
    assert!(run_in_index(&args, &profiles(), &index, &store).is_err());
    assert!(!store.exists());
    args.profile = Some("codex-a".to_string());
    run_in_index(&args, &profiles(), &index, &store).unwrap();
    let saved = WorkspaceStore::load(&store);
    assert!(saved.cwd("codex-a", "shared").is_some());
    assert!(saved.cwd("claude-a", "shared").is_none());
}

#[test]
fn bad_destination_and_corrupt_store_are_rejected_without_overwriting() {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("session_workspaces.json");
    let folder = root.path.parent().unwrap();
    let index = vec![session(Agent::Claude, "claude-a", "a")];
    for to in [folder.join("missing"), folder.join("file")] {
        if to.file_name().unwrap() == "file" {
            std::fs::write(&to, "not a directory").unwrap();
        }
        assert!(run_in_index(&args(&to, &["a"], false), &profiles(), &index, &store).is_err());
        assert!(!store.exists());
    }
    for content in ["broken json", "{\"version\":2,\"profiles\":{}}"] {
        std::fs::write(&store, content).unwrap();
        assert!(run_in_index(&args(folder, &["a"], false), &profiles(), &index, &store).is_err());
        assert_eq!(std::fs::read_to_string(&store).unwrap(), content);
    }
}

#[test]
fn shared_change_rejects_antigravity_even_when_the_ui_adapter_is_bypassed() {
    let root = TempBookmarkStore::new();
    let store = root.path.with_file_name("session_workspaces.json");
    let folder = root.path.parent().unwrap();
    let claude = session(Agent::Claude, "claude-a", "a");
    let agy = session(Agent::Antigravity, "agy-a", "b");
    assert!(crate::session_folder::change(&[&claude, &agy], folder, &store).is_err());
    assert!(!store.exists());
}
