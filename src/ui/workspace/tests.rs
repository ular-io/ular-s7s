use super::state::{WorkspaceDialog, WorkspaceField};
use crate::ui::test_support::*;
use crate::ui::{App, Focus, Screen, UiMode};
use crate::workspaces::{Workspace, WorkspaceStore};
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
    assert!(app.workspace.new_row);
    assert_eq!(app.workspace_pane_cursor(), 2);
    assert_eq!(app.active_workspace_name(), None);
    assert_eq!(app.filtered.len(), 3);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.workspace_pane_cursor(), 2, "the new row is last");

    press(&mut app, KeyCode::Home);
    assert!(!app.workspace.new_row);
    assert_eq!(app.workspace_pane_cursor(), 0);
    press(&mut app, KeyCode::End);
    assert_eq!(app.workspace_pane_cursor(), 2, "End goes to the new row");
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
    press(&mut app, KeyCode::Down); // Includes
    type_text(&mut app, "lea");
    assert_eq!(dialog(&app).matching, 1);
    assert_eq!(app.filtered.len(), 3, "the session list waits for Save");
    // Cancel drops the words.
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.workspaces.workspaces[0].includes, "");
    assert_eq!(app.filtered.len(), 3);

    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down); // Excludes
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
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Includes));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).cursor_on_search());
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert_eq!(app.active_workspace_name(), Some("Api"));
}

#[test]
fn tab_moves_between_fields_search_folders_and_buttons() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    let stops = |app: &App| {
        let d = dialog(app);
        (d.on_buttons, d.cursor)
    };
    let mut seen = vec![stops(&app)];
    for _ in 0..6 {
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
            (false, 4),
            (true, 4),
            (false, 0)
        ]
    );
    press(&mut app, KeyCode::BackTab);
    assert!(dialog(&app).on_buttons);
    press(&mut app, KeyCode::BackTab);
    assert_eq!(stops(&app), (false, 4), "BackTab lands on the first folder");

    // ↑/↓ reach the buttons after the last folder and stop there.
    for _ in 0..5 {
        press(&mut app, KeyCode::Down);
    }
    assert!(dialog(&app).on_buttons);
    press(&mut app, KeyCode::Up);
    assert_eq!(stops(&app), (false, 6), "back on the last folder");

    // With no folder shown, Tab goes from Search straight to the buttons.
    press(&mut app, KeyCode::BackTab);
    assert!(dialog(&app).cursor_on_search());
    type_text(&mut app, "zz");
    press(&mut app, KeyCode::Tab);
    assert!(dialog(&app).on_buttons);
}

#[test]
fn space_toggles_folders_by_full_path_in_the_draft() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    // Rows: Name, Includes, Excludes, Search, then one row per session folder.
    assert_eq!(dialog(&app).folders.len(), 3);
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
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

    // Enter toggles too; j/k move on folder rows.
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Enter);
    assert_eq!(dialog(&app).matching, 2, "multiple folders are allowed");
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char('k'));
    press(&mut app, KeyCode::Char(' '));
    assert!(dialog(&app).draft.folders.is_empty());
    assert_eq!(dialog(&app).matching, 3, "no folder means every folder");

    press(&mut app, KeyCode::Char(' '));
    save(&mut app);
    assert_eq!(app.workspaces.workspaces[0].folders, vec![folder.clone()]);
    assert_eq!(app.filtered.len(), 1);
    assert_eq!(app.sessions[app.filtered[0]].cwd, folder);
}

/// Opens "Api" and puts the dialog cursor on the folder search row.
fn on_folder_search(app: &mut App) {
    on_api_dialog(app);
    for _ in 0..3 {
        press(app, KeyCode::Down);
    }
    assert!(dialog(app).cursor_on_search());
}

fn visible_folder_names(app: &App) -> Vec<String> {
    dialog(app)
        .visible_folders()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn folder_search_row_takes_typing_and_arrows_leave_it() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    // Letter shortcuts (j/k/q/g) and space are text on this row.
    type_text(&mut app, "ro q");
    assert_eq!(dialog(&app).folder_query.value, "ro q");
    assert!(!app.quit_armed);
    assert!(visible_folder_names(&app).is_empty());
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(visible_folder_names(&app), vec!["/tmp/root".to_string()]);

    // ↓ leaves the row with the filter kept; space toggles the match.
    press(&mut app, KeyCode::Down);
    assert_eq!(
        dialog(&app).cursor_folder(),
        Some(&PathBuf::from("/tmp/root"))
    );
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(dialog(&app).draft.folders, vec![PathBuf::from("/tmp/root")]);

    // ↑ returns to the row with the query selected; → drops the selection,
    // and typing resumes at the end.
    press(&mut app, KeyCode::Up);
    assert!(dialog(&app).cursor_on_search());
    assert!(dialog(&app).folder_query.select_all);
    press(&mut app, KeyCode::Right);
    assert!(!dialog(&app).folder_query.select_all);
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(dialog(&app).folder_query.value, "rot");
    // Esc clears a query before it cancels the dialog.
    press(&mut app, KeyCode::Esc);
    assert!(dialog(&app).folder_query.value.is_empty());
    assert_eq!(dialog(&app).visible.len(), 3);
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    press(&mut app, KeyCode::Up);
    assert_eq!(dialog(&app).cursor_field(), Some(WorkspaceField::Excludes));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, UiMode::Table);
}

#[test]
fn arriving_on_the_folder_search_selects_the_query_so_typing_replaces_it() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "leaf");
    assert!(
        !dialog(&app).folder_query.select_all,
        "typing never selects"
    );
    press(&mut app, KeyCode::Up);
    assert!(!dialog(&app).folder_query.select_all);
    press(&mut app, KeyCode::Down);
    assert!(dialog(&app).folder_query.select_all);
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(dialog(&app).folder_query.value, "m");
    assert!(!dialog(&app).folder_query.select_all);
}

#[test]
fn folder_search_arrows_move_the_text_cursor() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "eaf");
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Down);
    // ← collapses the selection to the start; typing then inserts there.
    press(&mut app, KeyCode::Left);
    assert!(!dialog(&app).folder_query.select_all);
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(dialog(&app).folder_query.value, "leaf");
    for _ in 0..10 {
        press(&mut app, KeyCode::Left);
    }
    assert!(dialog(&app).cursor_on_search());
    press(&mut app, KeyCode::Delete);
    assert_eq!(dialog(&app).folder_query.value, "eaf");
    press(&mut app, KeyCode::End);
    for _ in 0..10 {
        press(&mut app, KeyCode::Right);
    }
    assert!(dialog(&app).cursor_on_search());
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(dialog(&app).folder_query.value, "leaf");
    assert_eq!(visible_folder_names(&app), vec!["/tmp/leaf".to_string()]);
}

#[test]
fn folder_search_enter_jumps_to_the_first_match_and_reopening_clears_it() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "leaf");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert_eq!(
        dialog(&app).cursor_folder(),
        Some(&PathBuf::from("/tmp/leaf"))
    );

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert!(dialog(&app).folder_query.value.is_empty());
    assert_eq!(dialog(&app).visible.len(), 3);
}

#[test]
fn paste_goes_to_the_text_row_under_the_cursor() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    app.on_paste("tmp/mid");
    assert_eq!(dialog(&app).folder_query.value, "tmp/mid");
    assert_eq!(visible_folder_names(&app), vec!["/tmp/middle".to_string()]);

    press(&mut app, KeyCode::Up); // Excludes
    app.on_paste("root");
    assert_eq!(dialog(&app).excludes.value, "root");
    assert_eq!(dialog(&app).matching, 2);

    // A folder row owns no text: the paste is dropped.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    app.on_paste("x");
    assert_eq!(dialog(&app).folder_query.value, "tmp/mid");
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
    on_api_dialog(&mut app);
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    ctrl(&mut app, 'd');
    press(&mut app, KeyCode::Delete);
    press(&mut app, KeyCode::Char('+'));
    press(&mut app, KeyCode::Char(':'));
    ctrl(&mut app, 'w');
    ctrl(&mut app, 'u');
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert!(app.pending_effect.is_none());
    assert_eq!(app.workspaces.workspaces.len(), 1);
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
    assert_eq!(state.items[0].label, "Open Workspace Api");
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
    type_text(&mut app, "leaf");
    assert_eq!(
        WorkspaceStore::load(&path).unwrap().workspaces[0].includes,
        "",
        "nothing is written before Save"
    );
    save(&mut app);

    let stored = WorkspaceStore::load(&path).expect("saved store");
    assert_eq!(stored.active.as_deref(), Some("ws-api"));
    assert_eq!(stored.workspaces[0].includes, "leaf");
    let restarted = WorkspaceStore::load_at_startup(&path).expect("saved store");
    assert_eq!(restarted.active, None, "a start opens All");
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
    // Dividers set the fixed rows apart from the stored workspaces.
    let pane: Vec<String> = text
        .lines()
        .skip(6)
        .take(5)
        .map(|l| l.chars().skip(1).take(22).collect::<String>())
        .collect();
    let rule = "─".repeat(22);
    assert_eq!(
        pane.iter().map(|l| l.trim_end()).collect::<Vec<_>>(),
        [
            " [ALL]",
            rule.as_str(),
            " Api",
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
fn edit_dialog_draws_fields_match_count_checked_folders_and_buttons() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    on_api_dialog(&mut app);
    let text = rendered(&app, 140, 40);
    assert!(text.contains(" Edit Workspace "), "{text}");
    assert!(text.contains("Name     Api"), "{text}");
    assert!(text.contains("Includes (none)"), "{text}");
    assert!(text.contains("Matches  1 of 3 sessions"), "{text}");
    assert!(text.contains("Folders · 1 selected"), "{text}");
    assert!(text.contains("[✓] root"), "{text}");
    assert!(text.contains("[ ] leaf"), "{text}");
    assert!(text.contains("Save"), "{text}");
    assert!(text.contains("Cancel"), "{text}");
    // The session list stays behind the dialog.
    assert!(text.contains("Session[Api: 1]"), "{text}");

    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('+'));
    let text = rendered(&app, 140, 40);
    assert!(text.contains(" New Workspace "), "{text}");
}

#[test]
fn edit_dialog_grows_with_the_folders_up_to_ninety_percent_of_the_terminal() {
    let mut app = app_with_workspace();
    on_api_dialog(&mut app);
    // Three folders: 13 rows of chrome plus three folder rows.
    let text = rendered(&app, 140, 40);
    assert_eq!(dialog_rows(&text).len(), 16, "{text}");

    // Thirty folders on a 40-row terminal: capped at 36 rows, scrolling.
    for i in 0..27 {
        let mut s = app.sessions[0].clone();
        s.id = format!("extra-{i}");
        s.cwd = PathBuf::from(format!("/tmp/extra{i:02}"));
        app.sessions.push(s);
    }
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Enter);
    assert_eq!(dialog(&app).folders.len(), 30);
    let text = rendered(&app, 140, 40);
    let rows = dialog_rows(&text);
    assert_eq!(rows.len(), 36, "{text}");
    assert!(rows.iter().any(|l| l.contains("Save")), "{text}");

    // Typing a query never changes the height, so the buttons stay put.
    for _ in 0..3 {
        press(&mut app, KeyCode::Down);
    }
    type_text(&mut app, "leaf");
    assert_eq!(dialog_rows(&rendered(&app, 140, 40)).len(), 36);
}

#[test]
fn dialog_folder_rows_end_with_a_session_count_even_when_narrow() {
    let mut app = app_with_workspace();
    app.sessions[1].cwd = PathBuf::from("/tmp/leaf");
    // A stored folder with no session left still shows its (0).
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/gone")];
    on_api_dialog(&mut app);
    // The dialog's right border is thick.
    for width in [140, 80] {
        let text = rendered(&app, width, 30);
        let leaf = text
            .lines()
            .find(|l| l.contains("[ ] leaf"))
            .expect("leaf row");
        assert!(leaf.contains("(2)  ┃"), "{width}: {leaf}");
        let gone = text
            .lines()
            .find(|l| l.contains("[✓] gone"))
            .expect("gone row");
        assert!(gone.contains("(0)  ┃"), "{width}: {gone}");
    }
}

#[test]
fn folder_search_row_renders_query_and_empty_result() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    let text = rendered(&app, 140, 30);
    assert!(
        !text.contains("type to filter"),
        "focused: no placeholder\n{text}"
    );
    assert!(text.contains("Type to filter folders"), "{text}");
    press(&mut app, KeyCode::Up);
    let text = rendered(&app, 140, 30);
    assert!(text.contains("Search   type to filter"), "{text}");
    press(&mut app, KeyCode::Down);
    type_text(&mut app, "zz");
    let text = rendered(&app, 140, 30);
    assert!(text.contains("Search   zz"), "{text}");
    assert!(text.contains("No matching folders"), "{text}");
    assert!(text.contains("0 of 3 folders"), "{text}");
    assert!(!text.contains("[ ] leaf"), "{text}");
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

#[test]
fn folder_search_row_paints_only_the_selected_text() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "ro");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Up);
    let mut terminal = Terminal::new(TestBackend::new(140, 30)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let buf = terminal.backend().buffer();
    let y = (0..buf.area.height)
        .find(|&y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains("Search   ro")
        })
        .expect("search row");
    let x = (0..buf.area.width)
        .find(|&x| buf[(x, y)].symbol() == "S" && buf[(x + 9, y)].symbol() == "r")
        .expect("search label");
    let selection_bg = app.theme.selection_bg;
    assert_ne!(buf[(x, y)].bg, selection_bg, "label is not highlighted");
    assert_eq!(buf[(x + 9, y)].bg, selection_bg, "query text is selected");
    assert_eq!(buf[(x + 10, y)].bg, selection_bg);
    assert_ne!(
        buf[(x + 11, y)].bg,
        selection_bg,
        "padding is not highlighted"
    );

    press(&mut app, KeyCode::Right);
    terminal
        .draw(|f| crate::ui::render::draw(f, &app))
        .expect("draw");
    let buf = terminal.backend().buffer();
    assert_ne!(buf[(x + 9, y)].bg, selection_bg, "→ drops the selection");
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
    for fixed in ["[ALL]", "[NEW WORKSPACE]"] {
        assert_eq!(
            cell_at(&buf, fixed, pane.clone()).fg,
            app.theme.key_hint,
            "{fixed}"
        );
    }
    assert_eq!(cell_at(&buf, "FOLDER", table).fg, muted);
}
