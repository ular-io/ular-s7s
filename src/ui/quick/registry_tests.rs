//! Palette registry matching and ranking tests.

use super::registry::*;

fn spec_of(item: &QuickItem) -> &'static CommandSpec {
    item.spec().expect("registry command")
}

fn labels(items: &[QuickItem]) -> Vec<&str> {
    items.iter().map(|i| i.label.as_str()).collect()
}

#[test]
fn workspace_query_lists_close_then_every_open_row() {
    let names = ["AAA", "bbb", "ccc"];
    let items = build_workspace_items(WORKSPACE_QUERY, &names, true);
    assert_eq!(
        labels(&items),
        [
            "Close Workspace",
            "Open Workspace No Workspace",
            "Open Workspace AAA",
            "Open Workspace bbb",
            "Open Workspace ccc"
        ]
    );
    assert!(items.iter().all(|i| i.enabled));
    // Narrowing by a name keeps only that row (Close has no such word).
    let items = build_workspace_items("open workspace bb", &names, true);
    assert_eq!(labels(&items), ["Open Workspace bbb"]);
}

#[test]
fn close_workspace_is_disabled_and_last_while_all_is_open() {
    let items = build_workspace_items(WORKSPACE_QUERY, &["AAA"], false);
    assert_eq!(
        labels(&items),
        [
            "Open Workspace No Workspace",
            "Open Workspace AAA",
            "Close Workspace"
        ]
    );
    assert!(!items[2].enabled);
    assert_eq!(items[2].description(), "No workspace is open");
}

#[test]
fn empty_query_adds_no_workspace_rows() {
    assert!(build_workspace_items("  ", &["AAA"], true).is_empty());
}

#[test]
fn workspace_query_also_matches_the_workspace_window_command() {
    let items = build_items(WORKSPACE_QUERY, &[], |_| true);
    assert_eq!(labels(&items), ["Open Workspace Window"]);
}

#[test]
fn multi_word_and_matching() {
    let items = build_items("del prof", &[], |_| true);
    assert_eq!(items.len(), 1);
    assert_eq!(spec_of(&items[0]).label, "Delete Profile");
}

#[test]
fn synonym_matching() {
    let items = build_items("exit", &[], |_| true);
    assert!(items.iter().any(|i| spec_of(i).label == "Quit"));
    let items = build_items("update", &[], |_| true);
    assert!(items
        .iter()
        .any(|i| spec_of(i).label == "Refresh Usage & Sessions"));
}

#[test]
fn disabled_items_sort_below_enabled() {
    let items = build_items("session", &[], |id| id != CommandId::ResumeSession);
    let resume_pos = items
        .iter()
        .position(|i| spec_of(i).id == CommandId::ResumeSession)
        .unwrap();
    // Disabled "Resume Session" must sort below enabled matched items.
    assert!(items[..resume_pos].iter().all(|i| i.enabled));
    assert!(!items[resume_pos].enabled);
}

#[test]
fn empty_query_lists_recent_first_then_rest() {
    let history = vec!["exit-s7s".to_string(), "refresh-all".to_string()];
    let items = build_items("", &history, |_| true);
    assert_eq!(items.len(), COMMANDS.len());
    assert_eq!(spec_of(&items[0]).key, "exit-s7s");
    assert_eq!(spec_of(&items[1]).key, "refresh-all");
    // Remaining items preserve default registry order.
    assert_eq!(spec_of(&items[2]).key, COMMANDS[0].key);
}

#[test]
fn edit_config_matches_editor_and_settings_aliases() {
    for query in ["editor", "settings", "config"] {
        let items = build_items(query, &[], |_| true);
        assert!(
            items.iter().any(|i| spec_of(i).id == CommandId::EditConfig),
            "query {query:?} should match Edit Config"
        );
    }
}

#[test]
fn recent_but_disabled_still_sorts_below_enabled() {
    let history = vec!["toggle-tool-logs".to_string()];
    let items = build_items("", &history, |id| id != CommandId::ToggleToolLogs);
    assert!(!items.last().unwrap().enabled);
    assert_eq!(spec_of(items.last().unwrap()).key, "toggle-tool-logs");
}
