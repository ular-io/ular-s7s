use super::state::{WorkspaceDialog, WorkspaceField};
use crate::ui::test_support::*;
use crate::ui::{App, Focus, Screen, UiMode};
use crate::workspaces::{Workspace, WorkspaceScope, WorkspaceStore};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use std::path::PathBuf;

fn press(app: &mut App, code: KeyCode) {
    send(app, code, KeyModifiers::NONE);
}

fn ctrl(app: &mut App, c: char) {
    send(app, KeyCode::Char(c), KeyModifiers::CONTROL);
}

/// Routes like `runtime::dispatch_event` for the modes these tests reach.
fn send(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let k = key(code, modifiers);
    match app.mode {
        UiMode::Table => match app.screen {
            Screen::Session => app.on_key_table(k),
            Screen::Profile => app.on_key_profile_table(k),
            Screen::Detail => app.on_key_detail(k),
        },
        UiMode::Keyword => app.on_key_keyword(k),
        UiMode::WorkspaceEdit => app.on_key_workspace_dialog(k),
        UiMode::WorkspaceDeleteConfirm => app.on_key_workspace_delete_confirm(k),
        UiMode::QuickCommand => app.on_key_quick(k),
        UiMode::DeleteConfirm => app.on_key_delete_confirm(k),
        UiMode::Message => app.on_key_message(k),
        mode => panic!("unexpected mode {mode:?}"),
    }
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

/// Three sessions in /tmp/{leaf,middle,root} plus one stored workspace "Api".
fn app_with_workspace() -> App {
    let mut app = app_with_context_chain();
    app.workspaces
        .workspaces
        .push(Workspace::new("ws-api".into(), "Api".into()));
    app.recompute();
    app
}

/// `←` from the session list: the workspace pane, focused.
fn on_pane(app: &mut App) {
    press(app, KeyCode::Left);
    assert_eq!(app.focus, Focus::Workspaces);
}

fn dialog(app: &App) -> &WorkspaceDialog {
    app.workspace.dialog.as_ref().expect("edit dialog")
}

/// Opens "Api" in the pane and its edit dialog, cursor on Name.
fn on_api_dialog(app: &mut App) {
    on_pane(app);
    press(app, KeyCode::Down);
    press(app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert_eq!(dialog(app).cursor_field(), Some(WorkspaceField::Name));
}

/// Tabs to the button row and presses Save.
fn save(app: &mut App) {
    while !dialog(app).on_buttons {
        press(app, KeyCode::Tab);
    }
    if !dialog(app).save_focused {
        press(app, KeyCode::Left);
    }
    press(app, KeyCode::Enter);
}

fn visible_ids(app: &App) -> Vec<String> {
    app.filtered
        .iter()
        .map(|&i| app.sessions[i].id.clone())
        .collect()
}

fn on_unassigned(app: &mut App) {
    app.open_workspace_pane();
    press(app, KeyCode::End);
    press(app, KeyCode::Up);
    assert_eq!(
        app.workspace_pane_cursor(),
        app.workspaces.workspaces.len() + 1
    );
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
}

#[test]
fn unassigned_is_the_complement_of_all_saved_workspace_conditions() {
    let mut app = app_with_workspace();
    // One selected folder can contain both matched and unmatched sessions.
    let folder = app.sessions[0].cwd.clone();
    for session in &mut app.sessions {
        session.cwd = folder.clone();
    }
    app.workspaces.workspaces[0].folders = vec![folder];
    app.workspaces.workspaces[0].includes = "leaf middle root".into();
    app.workspaces.workspaces[0].excludes = "middle root".into();
    // Includes are AND, so this first scope covers nothing.
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["leaf", "middle", "root"]);

    app.workspaces.workspaces[0].includes.clear();
    let mut other = Workspace::new("other".into(), "Other".into());
    other.includes = "middle".into();
    app.workspaces.upsert(other);
    app.invalidate_workspace_membership();
    app.recompute();
    assert_eq!(visible_ids(&app), ["root"]);
    assert!(rendered(&app, 140, 24).contains("Session[No Workspace: 1]"));

    app.filter.keyword = "absent".into();
    app.recompute();
    assert!(app.filtered.is_empty());
    app.filter = crate::filter::Filter::default();
    app.recompute();
    assert_eq!(visible_ids(&app), ["root"]);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
}

#[test]
fn unassigned_handles_empty_stores_and_unrestricted_workspaces() {
    let mut app = app_with_context_chain();
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["leaf", "middle", "root"]);
    press(&mut app, KeyCode::Down);
    assert!(app.workspace.new_row);
    assert_eq!(app.workspace_pane_cursor(), 2);
    assert_eq!(app.workspaces.active, WorkspaceScope::All);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    app.workspaces
        .upsert(Workspace::new("all".into(), "Any".into()));
    app.invalidate_workspace_membership();
    app.recompute();
    assert!(app.filtered.is_empty());

    let mut empty = empty_app();
    on_unassigned(&mut empty);
    assert!(empty.filtered.is_empty());
    let text = rendered(&empty, 80, 24);
    let pane: Vec<String> = text
        .lines()
        .skip(6)
        .take(4)
        .map(|l| l.chars().skip(1).take(22).collect::<String>())
        .collect();
    assert_eq!(
        pane.iter().map(|l| l.trim_end()).collect::<Vec<_>>(),
        [
            " [ALL]",
            " [NO WORKSPACE]",
            "─".repeat(22).as_str(),
            " [NEW WORKSPACE]"
        ],
        "{text}"
    );
}

#[test]
fn unassigned_follows_the_last_saved_workspace_and_scrolls_into_view() {
    let mut app = app_with_context_chain();
    for i in 0..30 {
        app.workspaces.upsert(Workspace::new(
            format!("ws-{i}"),
            format!("Workspace {i:02}"),
        ));
    }
    on_unassigned(&mut app);
    let text = rendered(&app, 80, 12);
    assert!(text.contains("[NO WORKSPACE]"), "{text}");
    assert!(!text.contains("[ALL]"), "{text}");
    assert!(app.workspace.list_scroll.get() > 0);

    press(&mut app, KeyCode::Up);
    assert_eq!(app.active_workspace_name(), Some("Workspace 29"));
    press(&mut app, KeyCode::Down);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    press(&mut app, KeyCode::Down);
    assert!(app.workspace.new_row);
    assert!(rendered(&app, 80, 12).contains("[NEW WORKSPACE]"));

    press(&mut app, KeyCode::Home);
    let text = rendered(&app, 80, 12);
    assert!(text.contains("[ALL]"), "{text}");
    assert_eq!(app.workspace.list_scroll.get(), 0);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.workspace_pane_cursor(), 0);
}

#[test]
fn unassigned_cannot_be_edited_or_deleted_and_new_draft_cancel_keeps_it() {
    let mut app = app_with_workspace();
    on_unassigned(&mut app);
    for code in [KeyCode::Enter, KeyCode::Delete] {
        press(&mut app, code);
        assert_eq!(app.mode, UiMode::Table);
    }
    ctrl(&mut app, 'd');
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.workspaces.workspaces.len(), 1);
    press(&mut app, KeyCode::Char('+'));
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    type_text(&mut app, "New Scope");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    assert_eq!(app.workspace_pane_cursor(), 2);
    press(&mut app, KeyCode::Right);
    on_pane(&mut app);
    assert_eq!(app.workspace_pane_cursor(), 2);
}

#[test]
fn workspace_save_and_delete_invalidate_unassigned_membership() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "leaf".into();
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["middle", "root"]);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    let includes = &mut app.workspace.dialog.as_mut().unwrap().includes;
    includes.select_all = true;
    type_text(&mut app, "middle");
    save(&mut app);
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["leaf", "root"]);
    press(&mut app, KeyCode::Up);
    ctrl(&mut app, 'd');
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Enter);
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["leaf", "middle", "root"]);
}

#[test]
fn session_scan_replaces_membership_even_when_the_session_count_is_unchanged() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "leaf".into();
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["middle", "root"]);
    let mut sessions = app.sessions.clone();
    sessions[0].search_blob = "renamed".into();
    sessions[1].search_blob = "leaf".into();
    sessions.reverse();
    app.apply_session_scan(
        crate::scan::ScanResult {
            sessions,
            scanned_files: 3,
            reparsed_files: 2,
        },
        None,
    );
    assert_eq!(visible_ids(&app), ["root", "leaf"]);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
}

#[test]
fn unassigned_folder_counts_and_bookmark_filters_use_the_same_scope() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "leaf".into();
    let root = app.sessions[2].clone();
    app.bookmarks.set(&root, true);
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["root", "middle"]);
    app.filter.bookmarked_only = true;
    app.filter.folders.insert("root".into());
    app.recompute();
    app.open_folder_modal();
    assert_eq!(app.folder_counts.get("root"), Some(&1));
    assert!(!app.folder_counts.contains_key("leaf"));
    assert!(!app.folder_counts.contains_key("middle"));
}

#[test]
fn unassigned_context_jump_and_back_restore_the_scope() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "middle".into();
    on_unassigned(&mut app);
    press(&mut app, KeyCode::Right);
    ctrl(&mut app, 'o');
    assert_eq!(app.current().unwrap().id, "middle");
    assert_eq!(app.workspaces.active, WorkspaceScope::All);
    app.return_to_jump_origin();
    assert_eq!(app.current().unwrap().id, "leaf");
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
}

#[test]
fn unassigned_palette_opens_and_close_returns_to_all() {
    let mut app = app_with_context_chain();
    ctrl(&mut app, 'w');
    assert_eq!(
        app.quick.as_ref().unwrap().items[0].label,
        "Open Workspace No Workspace"
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    assert_eq!(app.focus, Focus::Table);
    ctrl(&mut app, 'w');
    assert_eq!(
        app.quick.as_ref().unwrap().items[0].label,
        "Close Workspace"
    );
    assert!(app.quick.as_ref().unwrap().items[0].enabled);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.workspaces.active, WorkspaceScope::All);
}

#[test]
fn reload_keeps_unassigned_open_and_recalculates_changed_workspace_rules() {
    let root = TempBookmarkStore::new();
    let path = root.path.with_file_name("workspaces.json");
    let mut app = app_with_workspace();
    app.workspaces_path = Some(path.clone());
    app.workspaces.workspaces[0].includes = "leaf".into();
    on_unassigned(&mut app);
    assert_eq!(visible_ids(&app), ["middle", "root"]);
    let mut changed = app.workspaces.workspaces[0].clone();
    changed.includes = "middle".into();
    WorkspaceStore::commit(
        &path,
        &[crate::workspaces::WorkspaceChange::Upsert(changed)],
    )
    .unwrap();
    app.reload_shared_stores();
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    assert_eq!(visible_ids(&app), ["leaf", "root"]);
}

fn names(app: &App) -> Vec<String> {
    app.workspaces
        .workspaces
        .iter()
        .map(|w| w.name.clone())
        .collect()
}

#[test]
fn left_opens_the_pane_right_and_esc_close_it_and_profile_returns_to_it() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    assert_eq!(app.screen, Screen::Session);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.focus, Focus::Table);
    on_pane(&mut app);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, Focus::Table);
    assert_eq!(app.mode, UiMode::Table);

    on_pane(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Left);
    assert_eq!(app.screen, Screen::Profile);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.focus, Focus::Workspaces, "→ retraces ← from the pane");
    assert_eq!(app.workspace_pane_cursor(), 1, "on the open workspace");
}

#[test]
fn pane_cursor_is_the_scope_and_the_new_row_shows_all() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "leaf".into();
    on_pane(&mut app);
    assert_eq!(app.filtered.len(), 3, "All lists every session");

    press(&mut app, KeyCode::Down);
    assert_eq!(app.active_workspace_name(), Some("Api"));
    assert_eq!(visible_ids(&app), ["leaf"]);
    assert_eq!(app.mode, UiMode::Table, "moving opens no dialog");

    press(&mut app, KeyCode::Down);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    assert_eq!(visible_ids(&app), ["middle", "root"]);
    press(&mut app, KeyCode::Down);
    assert!(app.workspace.new_row);
    assert_eq!(app.workspace_pane_cursor(), 3);
    assert_eq!(app.active_workspace_name(), None);
    assert_eq!(app.filtered.len(), 3);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.workspace_pane_cursor(), 3, "the new row is last");

    press(&mut app, KeyCode::Home);
    assert!(!app.workspace.new_row);
    assert_eq!(app.workspace_pane_cursor(), 0);
    press(&mut app, KeyCode::End);
    assert_eq!(app.workspace_pane_cursor(), 3, "End goes to the new row");
    press(&mut app, KeyCode::Up);
    assert_eq!(app.workspaces.active, WorkspaceScope::Unassigned);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.active_workspace_name(), Some("Api"));

    // Closing the pane keeps the scope for the session list.
    press(&mut app, KeyCode::Right);
    assert_eq!(visible_ids(&app), ["leaf"]);
}

#[test]
fn enter_opens_the_edit_dialog_over_the_session_screen_and_esc_cancels() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table, "Enter on All does nothing");

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert_eq!(app.screen, Screen::Session);
    assert!(!dialog(&app).created);
    assert_eq!(dialog(&app).name.value, "Api");

    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.workspace.dialog.is_none());
    assert_eq!(app.focus, Focus::Workspaces);
    assert_eq!(app.active_workspace_name(), Some("Api"));
}

#[test]
fn new_row_enter_opens_a_new_workspace_that_exists_only_once_saved() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert!(dialog(&app).created);
    assert_eq!(app.workspaces.workspaces.len(), 1, "not added before Save");
    assert_eq!(app.active_workspace_name(), None);

    // The suggested name is selected: typing replaces it.
    type_text(&mut app, "Web");
    assert_eq!(dialog(&app).name.value, "Web");
    save(&mut app);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.focus, Focus::Workspaces);
    assert_eq!(names(&app), ["Api", "Web"]);
    assert_eq!(app.active_workspace_name(), Some("Web"));
    assert!(!app.workspace.new_row);
    assert_eq!(
        app.workspace_pane_cursor(),
        2,
        "the pane cursor lands on it"
    );
    assert_eq!(app.status_msg.as_deref(), Some("Workspace added: Web"));
}

#[test]
fn plus_opens_a_new_workspace_from_any_row_and_cancel_keeps_the_pane() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    press(&mut app, KeyCode::Down); // On "Api": `+` does not need the new row.
    press(&mut app, KeyCode::Char('+'));
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert!(dialog(&app).created);
    assert_eq!(app.active_workspace_name(), Some("Api"), "scope unchanged");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.workspaces.workspaces.len(), 1);
    assert_eq!(app.active_workspace_name(), Some("Api"));

    // The Cancel button does the same.
    press(&mut app, KeyCode::Char('+'));
    press(&mut app, KeyCode::BackTab);
    assert!(dialog(&app).on_buttons);
    assert!(dialog(&app).save_focused, "Save is focused first");
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.workspaces.workspaces.len(), 1);
}

#[test]
fn the_pane_lists_workspaces_by_name_as_text() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    for name in ["\u{D55C}\u{AE00}", "beta", "Alpha", "10x"] {
        press(&mut app, KeyCode::Char('+'));
        type_text(&mut app, name);
        save(&mut app);
    }
    assert_eq!(
        names(&app),
        ["10x", "Alpha", "Api", "beta", "\u{D55C}\u{AE00}"]
    );
    assert_eq!(app.active_workspace_name(), Some("10x"));
    assert_eq!(app.workspace_pane_cursor(), 1);

    // A rename moves the workspace to its new place; the cursor follows it.
    press(&mut app, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut app, KeyCode::Backspace);
    }
    type_text(&mut app, "zeta");
    save(&mut app);
    assert_eq!(
        names(&app),
        ["Alpha", "Api", "beta", "zeta", "\u{D55C}\u{AE00}"]
    );
    assert_eq!(app.active_workspace_name(), Some("zeta"));
    assert_eq!(app.workspace_pane_cursor(), 4);
}

#[test]
fn duplicate_empty_and_reserved_names_are_refused_with_the_dialog_open() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    press(&mut app, KeyCode::Char('+'));
    for (name, reason) in [
        ("api", "already exists"),
        ("  ", "cannot be empty"),
        ("all", "reserved"),
        ("no workspace", "reserved"),
    ] {
        let d = app.workspace.dialog.as_mut().unwrap();
        d.name = crate::ui::TextInput::new(String::new());
        type_text(&mut app, name);
        save(&mut app);
        assert_eq!(app.mode, UiMode::WorkspaceEdit, "{name:?} was accepted");
        let d = dialog(&app);
        assert!(
            d.error.as_deref().is_some_and(|e| e.contains(reason)),
            "{name:?}: {:?}",
            d.error
        );
        assert_eq!(d.cursor_field(), Some(WorkspaceField::Name));
        assert!(!d.on_buttons, "back on the Name row");
    }
    // The next edit clears the reason.
    type_text(&mut app, "x");
    assert!(dialog(&app).error.is_none());
    assert_eq!(app.workspaces.workspaces.len(), 1);
}

#[test]
fn include_and_exclude_words_update_the_match_count_and_apply_on_save() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    assert_eq!(dialog(&app).matching, 3);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down); // Includes
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Includes));
    type_text(&mut app, "lea");
    assert_eq!(dialog(&app).matching, 1);
    assert_eq!(app.filtered.len(), 3, "the session list waits for Save");
    // Cancel drops the words.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.workspaces.workspaces[0].includes, "");
    assert_eq!(app.filtered.len(), 3);

    press(&mut app, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut app, KeyCode::Down);
    }
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Excludes));
    type_text(&mut app, "root middle");
    assert_eq!(dialog(&app).matching, 1);
    save(&mut app);
    assert_eq!(app.workspaces.workspaces[0].excludes, "root middle");
    assert_eq!(visible_ids(&app), ["leaf"]);
    assert_eq!(app.status_msg.as_deref(), Some("Workspace saved: Api"));
}

#[test]
fn the_name_is_edited_in_place_and_saved_only_with_save() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "IS");
    assert_eq!(app.active_workspace_name(), Some("Api"));
    save(&mut app);
    assert_eq!(app.active_workspace_name(), Some("ApIS"));
}

#[test]
fn enter_on_a_text_row_moves_on_and_never_saves() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    type_text(&mut app, "X");
    press(&mut app, KeyCode::Enter);
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Folders));
    assert!(
        dialog(&app).folder_list.is_none(),
        "moving on opens nothing"
    );
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Excludes));
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).on_buttons);
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert_eq!(app.active_workspace_name(), Some("Api"));
}

#[test]
fn tab_and_arrows_move_through_the_rows_and_the_buttons() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    let stops = |app: &App| {
        let d = dialog(app);
        (d.on_buttons, d.cursor)
    };
    let mut seen = vec![stops(&app)];
    for _ in 0..5 {
        press(&mut app, KeyCode::Tab);
        seen.push(stops(&app));
    }
    assert_eq!(
        seen,
        [
            (false, 0),
            (false, 1),
            (false, 2),
            (false, 3),
            (true, 3),
            (false, 0)
        ]
    );
    press(&mut app, KeyCode::BackTab);
    assert!(dialog(&app).on_buttons, "BackTab wraps to the buttons");
    press(&mut app, KeyCode::BackTab);
    assert_eq!(stops(&app), (false, 3));

    // ↑/↓ stop at either end instead of wrapping.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    assert!(dialog(&app).on_buttons);
    press(&mut app, KeyCode::Up);
    assert_eq!(stops(&app), (false, 3));
    for _ in 0..5 {
        press(&mut app, KeyCode::Up);
    }
    assert_eq!(stops(&app), (false, 0));
}

#[test]
fn the_closed_folder_combo_opens_only_with_enter_or_space() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    press(&mut app, KeyCode::Down);
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Folders));
    // Typing, arrows, and paste change nothing on a closed combo.
    type_text(&mut app, "xq");
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Backspace);
    app.on_paste("x");
    assert!(dialog(&app).folder_list.is_none());
    assert!(!app.quit_armed);
    assert_eq!(dialog(&app).name.value, "Api");
    assert_eq!(dialog(&app).includes.value, "");
    assert!(dialog(&app).folder_query.value.is_empty());

    // Enter and space open the checklist on [ALL FOLDERS]; Esc closes only it.
    press(&mut app, KeyCode::Enter);
    assert_eq!(dialog(&app).folder_list, Some(0));
    press(&mut app, KeyCode::Esc);
    assert!(dialog(&app).folder_list.is_none());
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(dialog(&app).folder_list, Some(0));
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).folder_list.is_none());
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Folders));
    // Esc on the closed combo cancels the dialog.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, UiMode::Table);
}

/// Opens "Api" and its folder checklist, cursor on `[ALL FOLDERS]`.
fn on_folder_list(app: &mut App) {
    on_api_dialog(app);
    press(app, KeyCode::Down);
    press(app, KeyCode::Enter);
    assert_eq!(dialog(app).folder_list, Some(0));
}

fn visible_folder_names(app: &App) -> Vec<String> {
    dialog(app)
        .visible_folders()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn space_toggles_folders_and_closing_keeps_the_selection() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    // Rows: [ALL FOLDERS], then one row per session folder.
    assert_eq!(dialog(&app).folders.len(), 3);
    assert_eq!(dialog(&app).list_rows(), 4);
    press(&mut app, KeyCode::Down);
    let folder = dialog(&app).cursor_folder().cloned().expect("folder row");
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(dialog(&app).draft.folders, vec![folder.clone()]);
    assert_eq!(dialog(&app).matching, 1);
    assert!(
        app.workspaces.workspaces[0].folders.is_empty(),
        "the store waits for Save"
    );
    assert_eq!(
        dialog(&app).cursor_folder(),
        Some(&folder),
        "toggling does not reorder rows"
    );
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(dialog(&app).matching, 2, "multiple folders are allowed");

    // Enter closes without touching the selection, even on an unchecked row.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).folder_list.is_none());
    assert_eq!(dialog(&app).draft.folders.len(), 2);

    // Space on [ALL FOLDERS] clears the selection: every folder again.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char(' '));
    assert!(dialog(&app).draft.folders.is_empty());
    assert_eq!(dialog(&app).matching, 3);

    // Esc closes and keeps the toggles too.
    press(&mut app, KeyCode::Down);
    let folder = dialog(&app).cursor_folder().cloned().expect("folder row");
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Esc);
    assert!(dialog(&app).folder_list.is_none());
    save(&mut app);
    assert_eq!(app.workspaces.workspaces[0].folders, vec![folder.clone()]);
    assert_eq!(app.filtered.len(), 1);
    assert_eq!(app.sessions[app.filtered[0]].cwd, folder);
}

#[test]
fn unchecking_the_last_folder_returns_to_every_folder() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(dialog(&app).draft.folders.len(), 1, "that folder alone");
    press(&mut app, KeyCode::Char(' '));
    assert!(dialog(&app).draft.folders.is_empty());
    assert_eq!(dialog(&app).matching, 3, "no folder means every folder");
}

#[test]
fn reopening_the_checklist_lists_the_selection_first() {
    let mut app = app_with_workspace();
    for s in &mut app.sessions {
        s.updated_at_ms = match s.id.as_str() {
            "leaf" => 1,
            "middle" => 3,
            _ => 2,
        };
    }
    on_folder_list(&mut app);
    let order = ["/tmp/middle", "/tmp/root", "/tmp/leaf"];
    assert_eq!(visible_folder_names(&app), order);
    for _ in 0..3 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(visible_folder_names(&app), order, "kept while open");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        visible_folder_names(&app),
        ["/tmp/leaf", "/tmp/middle", "/tmp/root"]
    );
}

#[test]
fn checklist_search_takes_typing_while_space_still_toggles() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    // Letter shortcuts are text in the open checklist.
    type_text(&mut app, "jkq");
    assert_eq!(dialog(&app).folder_query.value, "jkq");
    assert!(!app.quit_armed);
    assert!(visible_folder_names(&app).is_empty());
    assert_eq!(dialog(&app).folder_list, Some(0), "no match: [ALL FOLDERS]");
    for _ in 0..3 {
        press(&mut app, KeyCode::Backspace);
    }
    assert_eq!(dialog(&app).visible.len(), 3);

    // A query puts the cursor on the first match, so space toggles it.
    type_text(&mut app, "ro");
    assert_eq!(visible_folder_names(&app), ["/tmp/root"]);
    assert_eq!(
        dialog(&app).cursor_folder(),
        Some(&PathBuf::from("/tmp/root"))
    );
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(dialog(&app).draft.folders, vec![PathBuf::from("/tmp/root")]);
    assert_eq!(dialog(&app).folder_query.value, "ro", "space is no text");

    // ←/→/Home/End move the query's text cursor.
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Char('o'));
    assert_eq!(dialog(&app).folder_query.value, "roo");
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Delete);
    assert_eq!(dialog(&app).folder_query.value, "r");

    // Esc clears the query with the cursor kept on its folder, then closes.
    type_text(&mut app, "oo");
    press(&mut app, KeyCode::Esc);
    assert!(dialog(&app).folder_query.value.is_empty());
    assert_eq!(dialog(&app).visible.len(), 3);
    assert_eq!(
        dialog(&app).cursor_folder(),
        Some(&PathBuf::from("/tmp/root"))
    );
    assert!(dialog(&app).folder_list.is_some());
    type_text(&mut app, "leaf");
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).folder_list.is_none());
    assert_eq!(dialog(&app).visible.len(), 3, "closing drops the query");
    // Reopening starts with an empty query.
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).folder_query.value.is_empty());
    assert_eq!(dialog(&app).draft.folders, vec![PathBuf::from("/tmp/root")]);
}

#[test]
fn tab_closes_the_checklist_and_moves_on() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Tab);
    assert!(dialog(&app).folder_list.is_none());
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Includes));
    assert_eq!(dialog(&app).draft.folders.len(), 1);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::BackTab);
    assert!(dialog(&app).folder_list.is_none());
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Name));
}

#[test]
fn paste_goes_to_the_text_input_under_the_cursor() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    app.on_paste("tmp/mid");
    assert_eq!(dialog(&app).folder_query.value, "tmp/mid");
    assert_eq!(visible_folder_names(&app), ["/tmp/middle"]);

    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down); // Excludes
    app.on_paste("root");
    assert_eq!(dialog(&app).excludes.value, "root");
    assert_eq!(dialog(&app).matching, 2);

    // The closed combo owns no text: the paste is dropped.
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Up);
    app.on_paste("x");
    assert_eq!(dialog(&app).includes.value, "");
    assert!(dialog(&app).folder_query.value.is_empty());
}

#[test]
fn reopening_a_workspace_lists_its_folders_first() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    on_api_dialog(&mut app);
    assert_eq!(dialog(&app).folders[0], PathBuf::from("/tmp/root"));
    assert_eq!(dialog(&app).folders.len(), 3);
}

#[test]
fn selected_folders_come_first_each_group_by_latest_activity() {
    let mut app = app_with_workspace();
    for s in &mut app.sessions {
        s.updated_at_ms = match s.id.as_str() {
            "leaf" => 1,
            "middle" => 3,
            _ => 2,
        };
    }
    // Selected oldest first, plus a stored folder with no session left.
    app.workspaces.workspaces[0].folders = vec![
        PathBuf::from("/tmp/gone"),
        PathBuf::from("/tmp/leaf"),
        PathBuf::from("/tmp/root"),
    ];
    on_api_dialog(&mut app);
    assert_eq!(
        visible_folder_names(&app),
        vec!["/tmp/root", "/tmp/leaf", "/tmp/gone", "/tmp/middle"]
    );
}

#[test]
fn all_and_the_new_row_are_not_deletable() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    ctrl(&mut app, 'd');
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(
        app.status_msg.as_deref(),
        Some("The All workspace cannot be deleted")
    );
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Delete);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(
        app.status_msg.as_deref(),
        Some("Select a workspace to delete")
    );
}

#[test]
fn ctrl_d_in_the_pane_deletes_after_confirmation_and_keeps_the_row() {
    let mut app = app_with_workspace();
    app.workspaces
        .workspaces
        .push(Workspace::new("ws-web".into(), "Web".into()));
    on_pane(&mut app);
    press(&mut app, KeyCode::Down);
    ctrl(&mut app, 'd');
    assert_eq!(app.mode, UiMode::WorkspaceDeleteConfirm);
    press(&mut app, KeyCode::Enter); // Cancel is focused first.
    assert_eq!(app.workspaces.workspaces.len(), 2);
    assert_eq!(app.focus, Focus::Workspaces);

    ctrl(&mut app, 'd');
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.workspaces.workspaces.len(), 1);
    assert_eq!(app.active_workspace_name(), Some("Web"));

    press(&mut app, KeyCode::Delete);
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.workspaces.workspaces.is_empty());
    assert_eq!(app.active_workspace_name(), None);
}

#[test]
fn the_dialog_neither_deletes_nor_opens_other_windows() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    for open in [true, false] {
        ctrl(&mut app, 'd');
        press(&mut app, KeyCode::Delete);
        press(&mut app, KeyCode::Char('+'));
        press(&mut app, KeyCode::Char(':'));
        ctrl(&mut app, 'w');
        ctrl(&mut app, 'u');
        assert_eq!(app.mode, UiMode::WorkspaceEdit);
        assert_eq!(dialog(&app).folder_list.is_some(), open);
        assert!(app.pending_effect.is_none());
        assert_eq!(app.workspaces.workspaces.len(), 1);
        // Then the same on the closed combo.
        press(&mut app, KeyCode::Enter);
    }
}

#[test]
fn search_from_the_pane_returns_on_esc_and_closes_it_on_enter() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(app.mode, UiMode::Keyword);
    type_text(&mut app, "middle");
    assert_eq!(visible_ids(&app), ["middle"]);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.focus, Focus::Workspaces);
    assert_eq!(app.filtered.len(), 3);

    press(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "middle");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.focus, Focus::Table);
    assert_eq!(visible_ids(&app), ["middle"]);
}

#[test]
fn ctrl_w_palette_opens_and_closes_workspaces_on_the_session_screen() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "root".into();
    ctrl(&mut app, 'w');
    let state = app.quick.as_ref().expect("palette");
    assert_eq!(state.input.value, "open workspace ");
    assert_eq!(state.items[0].label, "Open Workspace No Workspace");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.active_workspace_name(), Some("Api"));
    assert_eq!(visible_ids(&app), ["root"]);

    // From the pane too, the palette lands on the session list.
    on_pane(&mut app);
    ctrl(&mut app, 'w');
    let labels: Vec<&str> = app
        .quick
        .as_ref()
        .unwrap()
        .items
        .iter()
        .map(|i| i.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "Close Workspace",
            "Open Workspace No Workspace",
            "Open Workspace Api",
            "Open Workspace Window"
        ]
    );
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.focus, Focus::Table);
    assert_eq!(app.active_workspace_name(), None);
    assert_eq!(app.filtered.len(), 3);
}

#[test]
fn open_workspace_window_shows_the_pane() {
    let mut app = app_with_workspace();
    app.switch_screen(Screen::Profile);
    ctrl(&mut app, 'w');
    type_text(&mut app, "window");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.screen, Screen::Session);
    assert_eq!(app.focus, Focus::Workspaces);
}

#[test]
fn context_jump_closes_a_workspace_hiding_the_source_and_back_restores_it() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "leaf".into();
    app.set_active_workspace(Some(0));
    assert_eq!(visible_ids(&app), ["leaf"]);
    ctrl(&mut app, 'o');
    assert_eq!(app.active_workspace_name(), None);
    assert_eq!(app.current().map(|s| s.id.as_str()), Some("middle"));

    app.return_to_jump_origin();
    assert_eq!(app.active_workspace_name(), Some("Api"));
    assert_eq!(app.current().map(|s| s.id.as_str()), Some("leaf"));
}

#[test]
fn saves_reach_the_file_and_a_restart_opens_all() {
    let root = TempBookmarkStore::new();
    let path = root.path.with_file_name("workspaces.json");
    let mut app = app_with_workspace();
    app.workspaces_path = Some(path.clone());
    // The fixture's workspace exists only in memory; a real one was loaded or saved.
    let api = app.workspaces.workspaces[0].clone();
    WorkspaceStore::commit(&path, &[crate::workspaces::WorkspaceChange::Upsert(api)]).unwrap();
    on_api_dialog(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    type_text(&mut app, "leaf");
    assert_eq!(
        WorkspaceStore::load(&path).unwrap().workspaces[0].includes,
        "",
        "nothing is written before Save"
    );
    save(&mut app);

    let stored = WorkspaceStore::load(&path).expect("saved store");
    assert_eq!(stored.active.id(), Some("ws-api"));
    assert_eq!(stored.workspaces[0].includes, "leaf");
    let restarted = WorkspaceStore::load_at_startup(&path).expect("saved store");
    assert_eq!(
        restarted.active,
        crate::workspaces::WorkspaceScope::All,
        "a start opens All"
    );
    assert_eq!(restarted.workspaces[0].includes, "leaf");
}

fn rendered(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, app))
        .expect("draw");
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn session_screen_draws_the_pane_only_while_focused_and_drops_a_narrow_prompt() {
    let mut app = app_with_workspace();
    let text = rendered(&app, 140, 24);
    assert!(!text.contains("[NEW WORKSPACE]"), "{text}");

    on_pane(&mut app);
    let text = rendered(&app, 140, 24);
    assert!(text.contains(" Workspaces "), "{text}");
    assert!(text.contains("[NEW WORKSPACE]"), "{text}");
    assert!(text.contains(" Prompt "), "{text}");
    // No Workspace follows the stored rows, above the new-workspace divider.
    let pane: Vec<String> = text
        .lines()
        .skip(6)
        .take(6)
        .map(|l| l.chars().skip(1).take(22).collect::<String>())
        .collect();
    let rule = "─".repeat(22);
    assert_eq!(
        pane.iter().map(|l| l.trim_end()).collect::<Vec<_>>(),
        [
            " [ALL]",
            rule.as_str(),
            " Api",
            " [NO WORKSPACE]",
            rule.as_str(),
            " [NEW WORKSPACE]"
        ],
        "{text}"
    );
    // The pane is 24 cells wide (its focused border is thick); the session
    // table's border starts right after.
    let row = text
        .lines()
        .find(|l| l.contains("[NEW WORKSPACE]"))
        .expect("new row");
    assert_eq!(row.chars().nth(23), Some('┃'), "{row}");
    assert_eq!(row.chars().nth(24), Some('│'), "{row}");

    // Too narrow for a 40-cell Prompt beside the pane: the table takes it.
    let text = rendered(&app, 100, 24);
    assert!(text.contains("[NEW WORKSPACE]"), "{text}");
    assert!(!text.contains(" Prompt "), "{text}");
    press(&mut app, KeyCode::Esc);
    let text = rendered(&app, 100, 24);
    assert!(text.contains(" Prompt "), "{text}");
}

/// Rows of the dialog frame (`┏` to `┗`) in a rendered screen.
fn dialog_rows(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let top = lines
        .iter()
        .position(|l| l.contains('┏'))
        .expect("dialog top");
    let bottom = lines
        .iter()
        .rposition(|l| l.contains('┗'))
        .expect("dialog bottom");
    lines[top..=bottom].to_vec()
}

#[test]
fn edit_dialog_stacks_the_fields_with_the_folder_combo_second() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    on_api_dialog(&mut app);
    let text = rendered(&app, 140, 40);
    assert!(text.contains(" Edit Workspace "), "{text}");
    let rows = dialog_rows(&text);
    assert_eq!(rows.len(), 19, "{text}");
    // One column of titled boxes: the focused Name box is thick, the rest plain.
    let at = |needle: &str| rows.iter().position(|l| l.contains(needle));
    let name = at("┏ Name ━").expect("name box");
    assert_eq!(at("┌ Folders ─"), Some(name + 3), "{text}");
    assert_eq!(at("┌ Includes · all words ─"), Some(name + 6), "{text}");
    assert_eq!(at("┌ Excludes · any word ─"), Some(name + 9), "{text}");
    assert!(rows[name + 4].contains("│ root "), "{text}");
    assert!(text.contains("│ (none) "), "{text}");
    assert!(text.contains("Matches  1 of 3 sessions"), "{text}");
    // One blank row under the title, and like every form no divider and no
    // column separator.
    let frame = rows[0].find('┏').expect("frame left");
    let blank: String = rows[1].chars().skip(frame + 1).take(20).collect();
    assert_eq!(blank.trim(), "", "{text}");
    assert!(!text.contains('┠') && !text.contains('┴'), "{text}");
    // Matches, the notice line (empty until a Save is refused), a blank row,
    // then the buttons.
    let matches = at("Matches  1 of 3 sessions").expect("matches");
    let left = rows[matches].chars().position(|c| c == '┃').unwrap();
    let inside = |r: usize| -> String { rows[r].chars().skip(left + 1).take(50).collect() };
    assert_eq!(inside(matches + 1).trim(), "", "{text}");
    assert_eq!(inside(matches + 2).trim(), "", "{text}");
    assert!(rows[matches + 3].contains("Save"), "{text}");
    // The checklist is not drawn while the combo is closed.
    assert!(!text.contains("[ALL FOLDERS]"), "{text}");
    assert!(text.contains("Save"), "{text}");
    assert!(text.contains("Cancel"), "{text}");
    // The session list stays behind the dialog.
    assert!(text.contains("Session[Api: 1]"), "{text}");

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('+'));
    let text = rendered(&app, 140, 40);
    assert!(text.contains(" New Workspace "), "{text}");
    assert!(text.contains("│ All folders "), "{text}");
}

#[test]
fn folder_combo_lists_whole_names_beside_a_dim_folder_count() {
    let mut app = app_with_workspace();
    for s in &mut app.sessions {
        s.updated_at_ms = match s.id.as_str() {
            "leaf" => 1,
            "middle" => 3,
            _ => 2,
        };
    }
    app.workspaces.workspaces[0].folders = vec![
        PathBuf::from("/tmp/leaf"),
        PathBuf::from("/tmp/middle"),
        PathBuf::from("/tmp/root"),
    ];
    on_api_dialog(&mut app);
    let summary = |width| {
        let (names, count) = super::render::folder_summary(dialog(&app), width);
        (names, count.unwrap_or_default())
    };
    let count = "3 folders".to_string();
    // Checklist order, and the count even when every name fits.
    assert_eq!(summary(40), ("middle, root, leaf".into(), count.clone()));
    // Cut at a name boundary, leaving room for `, …` and one gap cell.
    assert_eq!(summary(25), ("middle, root, …".into(), count.clone()));
    assert_eq!(summary(15), ("…".into(), count.clone()));
    assert_eq!(summary(9), (String::new(), count.clone()));

    // Rendered: names in the default color, the count dim at the right edge.
    let buf = draw_buffer(&app);
    let names = cell_at(&buf, "middle, root, leaf", 0..140);
    assert_ne!(names.fg, app.theme.soft_dim().fg.unwrap());
    let text = rendered(&app, 140, 24);
    let row = text
        .lines()
        .find(|l| l.contains("│ middle, root, leaf "))
        .expect("combo value");
    // The count ends one gap cell left of the `▾`, which sits just inside
    // the right padding.
    assert!(row.contains("3 folders ▾ │"), "{row}");
    assert_eq!(
        cell_at(&buf, "3 folders", 0..140).fg,
        app.theme.soft_dim().fg.unwrap()
    );

    // One folder is its name alone; none is every folder.
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        super::render::folder_summary(dialog(&app), 40),
        ("root".into(), None)
    );
    app.workspaces.workspaces[0].folders.clear();
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        super::render::folder_summary(dialog(&app), 40),
        ("All folders".into(), None)
    );
}

/// The `▾` sits at the inner right end of the `Folders` combo in the border's
/// style, and on an 80-column terminal the name summary and count stay left of
/// it, leaving the one gap cell.
#[test]
fn folder_combo_arrow_follows_focus_and_the_summary_stays_left_of_it() {
    use ratatui::style::Modifier;
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![
        PathBuf::from("/tmp/leaf"),
        PathBuf::from("/tmp/middle"),
        PathBuf::from("/tmp/root"),
    ];
    on_api_dialog(&mut app);
    let arrow = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|f| crate::ui::render::draw(f, app))
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let y = (0..buf.area.height)
            .find(|&y| (0..buf.area.width).any(|x| buf[(x, y)].symbol() == "▾"))
            .expect("arrow row");
        let row: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        let x = row.iter().position(|s| *s == "▾").unwrap();
        // `▾`, the right padding, then the combo's right border.
        assert_eq!(row[x + 1], " ", "{}", row.concat());
        assert!(matches!(row[x + 2], "│" | "┃"), "{}", row.concat());
        // The count ends at the gap cell before the `▾`.
        assert!(row.concat().contains("3 folders ▾"), "{}", row.concat());
        let title_y = y - 1;
        assert!(
            (0..buf.area.width).all(|x| buf[(x, title_y)].symbol() != "▾"),
            "no arrow in the title"
        );
        buf[(x as u16, y)].clone()
    };
    let unfocused = arrow(&app);
    assert_eq!(unfocused.fg, app.theme.dim);
    assert!(!unfocused.modifier.contains(Modifier::BOLD));
    press(&mut app, KeyCode::Down); // Name -> Folders
    let focused = arrow(&app);
    assert_eq!(focused.fg, app.theme.accent);
    assert!(focused.modifier.contains(Modifier::BOLD));
}

#[test]
fn folder_checklist_joins_the_combo_and_resolves_the_cursor_path() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    on_folder_list(&mut app);
    let text = rendered(&app, 140, 40);
    let lines: Vec<&str> = text.lines().collect();
    // The popup's top edge replaces the combo's bottom border.
    let top = lines
        .iter()
        .position(|l| l.contains("┣━") && l.contains("━┫"))
        .expect("joined popup edge");
    assert!(lines[top - 2].contains("┏ Folders ━"), "{text}");
    assert!(lines[top + 1].contains(" Search "), "{text}");
    assert!(lines[top + 2].contains("┠─"), "{text}");
    assert!(lines[top + 3].contains("[ ] [ALL FOLDERS]"), "{text}");
    assert!(
        lines[top + 4].contains("[✓] root"),
        "selection first
{text}"
    );
    assert!(text.contains("[ ] leaf"), "{text}");
    assert!(
        text.contains("Every folder: no folder restriction"),
        "{text}"
    );
    assert!(
        text.contains("space toggle"),
        "status bar
{text}"
    );

    press(&mut app, KeyCode::Down);
    let text = rendered(&app, 140, 40);
    assert!(text.contains(" /tmp/root "), "{text}");
}

#[test]
fn checklist_rows_end_with_a_session_count_even_when_narrow() {
    let mut app = app_with_workspace();
    app.sessions[1].cwd = PathBuf::from("/tmp/leaf");
    // A stored folder with no session left still shows its (0).
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/gone")];
    on_folder_list(&mut app);
    // The popup's right border is thick, inside the dialog's padding.
    for width in [140, 80] {
        let text = rendered(&app, width, 30);
        let leaf = text
            .lines()
            .find(|l| l.contains("[ ] leaf"))
            .expect("leaf row");
        assert!(leaf.contains("(2) ┃"), "{width}: {leaf}");
        let gone = text
            .lines()
            .find(|l| l.contains("[✓] gone"))
            .expect("gone row");
        assert!(gone.contains("(0) ┃"), "{width}: {gone}");
        let all = text
            .lines()
            .find(|l| l.contains("[ ] [ALL FOLDERS]"))
            .expect("all row");
        assert!(all.contains("(3) ┃"), "{width}: {all}");
    }
}

#[test]
fn checklist_search_renders_query_and_empty_result() {
    let mut app = app_with_workspace();
    on_folder_list(&mut app);
    type_text(&mut app, "zz");
    let text = rendered(&app, 140, 30);
    assert!(text.contains("Search   zz"), "{text}");
    assert!(text.contains("[✓] [ALL FOLDERS]"), "{text}");
    assert!(text.contains("No matching folders"), "{text}");
    assert!(!text.contains("[ ] leaf"), "{text}");
}

#[test]
fn checklist_stops_at_the_terminal_bottom_and_scrolls() {
    let mut app = app_with_workspace();
    for i in 0..27 {
        let mut s = app.sessions[0].clone();
        s.id = format!("extra-{i}");
        s.cwd = PathBuf::from(format!("/tmp/extra{i:02}"));
        app.sessions.push(s);
    }
    on_folder_list(&mut app);
    assert_eq!(dialog(&app).folders.len(), 30);
    let text = rendered(&app, 140, 24);
    let lines: Vec<&str> = text.lines().collect();
    // Ends on the last terminal row: borders, search, divider, and footer
    // leave nine checklist rows.
    assert!(lines[23].contains('┗') && lines[23].contains('┛'), "{text}");
    let list_rows = lines
        .iter()
        .filter(|l| l.contains("[ ] ") || l.contains("[✓] "))
        .count();
    assert_eq!(list_rows, 9, "{text}");
    // The dialog's bottom border keeps its `━` on both sides of the popup,
    // whichever popup row crosses it.
    let bottom = lines.iter().position(|l| l.contains('┗')).expect("dialog");
    assert!(bottom < 23, "{text}");
    let row: Vec<char> = lines[bottom].chars().collect();
    let left = row.iter().position(|&c| c == '┗').unwrap();
    let right = row.iter().position(|&c| c == '┛').unwrap();
    assert_eq!(row[left + 1], '━', "{}", lines[bottom]);
    assert_eq!(row[right - 1], '━', "{}", lines[bottom]);

    // The cursor scrolls the rows; typing never resizes the popup.
    for _ in 0..3 {
        press(&mut app, KeyCode::PageDown);
    }
    rendered(&app, 140, 24);
    assert!(dialog(&app).folder_scroll.get() > 0);
    type_text(&mut app, "extra0");
    let text = rendered(&app, 140, 24);
    assert!(
        text.lines().nth(23).is_some_and(|l| l.contains('┛')),
        "{text}"
    );
}

#[test]
fn a_refused_save_shows_its_reason_on_the_notice_line() {
    let mut app = app_with_workspace();
    on_pane(&mut app);
    press(&mut app, KeyCode::Char('+'));
    type_text(&mut app, "api");
    save(&mut app);
    let text = rendered(&app, 140, 30);
    assert!(
        text.contains("A workspace named 'api' already exists"),
        "{text}"
    );
}

/// Buffer cell at the first character of `needle` inside columns `cols`,
/// searched below the header.
fn cell_at<'a>(
    buf: &'a ratatui::buffer::Buffer,
    needle: &str,
    cols: std::ops::Range<u16>,
) -> &'a ratatui::buffer::Cell {
    // Skips the five-row header, whose shortcut labels may contain `needle`.
    for y in 5..buf.area.height {
        let row: Vec<&str> = cols.clone().map(|x| buf[(x, y)].symbol()).collect();
        for x in 0..row.len() {
            if row[x..].concat().starts_with(needle) {
                let x = cols.start + x as u16;
                return &buf[(x, y)];
            }
        }
    }
    panic!("{needle:?} not rendered");
}

fn draw_buffer(app: &App) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(140, 24)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, app))
        .expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn panes_without_focus_fade_like_the_session_screen() {
    let mut app = app_with_workspace();
    let muted = app.theme.muted;

    // Session screen, workspace pane focused: the session table fades.
    on_pane(&mut app);
    press(&mut app, KeyCode::Down);
    let buf = draw_buffer(&app);
    let (pane, table) = (0..24, 24..140);
    assert_eq!(
        cell_at(&buf, "Api", pane.clone()).bg,
        app.theme.selection_bg
    );
    // Unselected fixed rows take the key-hint color; workspace names do not.
    for fixed in ["[ALL]", "[NO WORKSPACE]", "[NEW WORKSPACE]"] {
        assert_eq!(
            cell_at(&buf, fixed, pane.clone()).fg,
            app.theme.key_hint,
            "{fixed}"
        );
    }
    assert_eq!(cell_at(&buf, "FOLDER", table).fg, muted);
}
