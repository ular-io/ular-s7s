use super::WorkspacePane;
use crate::ui::test_support::*;
use crate::ui::{App, Screen, UiMode};
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
            Screen::Workspace => app.on_key_workspace(k),
        },
        UiMode::Keyword => app.on_key_keyword(k),
        UiMode::WorkspaceEdit => app.on_key_workspace_edit(k),
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
    app.refresh_workspace_folders();
    app
}

fn on_workspace_screen(app: &mut App) {
    app.enter_workspace_screen(WorkspacePane::List);
}

fn visible_ids(app: &App) -> Vec<String> {
    app.filtered
        .iter()
        .map(|&i| app.sessions[i].id.clone())
        .collect()
}

#[test]
fn screens_chain_profile_workspaces_sessions() {
    let mut app = app_with_workspace();
    app.screen = Screen::Profile;
    press(&mut app, KeyCode::Right);
    assert_eq!(app.screen, Screen::Workspace);
    assert_eq!(app.workspace.pane, WorkspacePane::List);

    // "All" has nothing to edit, so → skips the Detail pane.
    press(&mut app, KeyCode::Right);
    assert_eq!(app.workspace.pane, WorkspacePane::Sessions);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.screen, Screen::Session);

    // ← from the session list lands on the Workspaces screen's session pane.
    press(&mut app, KeyCode::Left);
    assert_eq!(app.screen, Screen::Workspace);
    assert_eq!(app.workspace.pane, WorkspacePane::Sessions);
    press(&mut app, KeyCode::Left);
    assert_eq!(app.workspace.pane, WorkspacePane::List);

    // With a real workspace open the Detail pane is reachable.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.workspace.pane, WorkspacePane::Detail);
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Left);
    assert_eq!(app.screen, Screen::Profile);
}

#[test]
fn list_cursor_is_the_scope_of_the_session_screen() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].includes = "leaf".into();
    on_workspace_screen(&mut app);
    assert_eq!(app.filtered.len(), 3, "All lists every session");

    press(&mut app, KeyCode::Down);
    assert_eq!(app.active_workspace_name(), Some("Api"));
    assert_eq!(visible_ids(&app), ["leaf"]);

    app.switch_screen(Screen::Session);
    assert_eq!(
        visible_ids(&app),
        ["leaf"],
        "the Session screen keeps the scope"
    );

    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Home);
    assert_eq!(app.active_workspace_name(), None);
    assert_eq!(app.filtered.len(), 3);
    press(&mut app, KeyCode::End);
    assert_eq!(app.active_workspace_name(), Some("Api"));
}

#[test]
fn plus_adds_a_workspace_with_its_name_in_edit() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Char('+'));
    assert_eq!(app.mode, UiMode::WorkspaceEdit);
    assert_eq!(app.active_workspace_name(), Some("New Workspace"));
    let edit = app.workspace.edit.as_ref().expect("edit");
    assert!(edit.in_list && edit.created);

    // The prefilled name is selected: typing replaces it.
    type_text(&mut app, "Web");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.active_workspace_name(), Some("Web"));
    assert_eq!(app.workspaces.workspaces.len(), 2);
    assert_eq!(app.status_msg.as_deref(), Some("Workspace added: Web"));
}

#[test]
fn esc_drops_an_unsaved_new_workspace_and_reopens_the_previous_scope() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char('+'));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.workspaces.workspaces.len(), 1);
    assert_eq!(app.active_workspace_name(), Some("Api"));
}

#[test]
fn duplicate_empty_and_reserved_names_are_rejected() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Char('+'));
    for name in ["api", "  ", "all"] {
        type_text(&mut app, name);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, UiMode::WorkspaceEdit, "{name:?} was accepted");
        let edit = app.workspace.edit.as_mut().unwrap();
        edit.input = crate::ui::TextInput::new(String::new());
    }
}

#[test]
fn include_and_exclude_words_filter_while_typing_and_esc_restores() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Down); // Includes
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "lea");
    assert_eq!(visible_ids(&app), ["leaf"], "applied before Enter");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.workspaces.workspaces[0].includes, "");
    assert_eq!(app.filtered.len(), 3);

    press(&mut app, KeyCode::Down); // Excludes
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "root middle");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.workspaces.workspaces[0].excludes, "root middle");
    assert_eq!(visible_ids(&app), ["leaf"]);
}

#[test]
fn detail_name_row_renames_in_place() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Enter);
    let edit = app.workspace.edit.as_ref().expect("edit");
    assert!(!edit.in_list);
    press(&mut app, KeyCode::Backspace);
    type_text(&mut app, "IS");
    // Names are not live: the stored name changes only on Enter.
    assert_eq!(app.active_workspace_name(), Some("Api"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.active_workspace_name(), Some("ApIS"));
}

#[test]
fn space_toggles_folders_by_full_path() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    // Rows: Name, Includes, Excludes, Search, then one row per session folder.
    assert_eq!(app.workspace.folders.len(), 3);
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    let folder = app.workspace.cursor_folder().cloned().expect("folder row");
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.workspaces.workspaces[0].folders, vec![folder.clone()]);
    assert_eq!(app.filtered.len(), 1);
    assert_eq!(app.sessions[app.filtered[0]].cwd, folder);
    assert_eq!(
        app.workspace.cursor_folder(),
        Some(&folder),
        "toggling does not reorder rows"
    );

    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.filtered.len(), 2, "multiple folders are allowed");
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Char(' '));
    assert!(app.workspaces.workspaces[0].folders.is_empty());
    assert_eq!(
        app.filtered.len(),
        3,
        "no folder selected means every folder"
    );
}

/// Opens "Api" and puts the Detail cursor on the folder search row.
fn on_folder_search(app: &mut App) {
    on_workspace_screen(app);
    press(app, KeyCode::Down);
    press(app, KeyCode::Right);
    for _ in 0..3 {
        press(app, KeyCode::Down);
    }
    assert!(app.workspace_search_focused());
}

fn visible_folder_names(app: &App) -> Vec<String> {
    app.workspace
        .visible_folders()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn folder_search_row_takes_typing_without_enter_and_arrows_leave_it() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    // Letter shortcuts (j/k/q/g) and space are text on this row.
    type_text(&mut app, "ro q");
    assert_eq!(app.workspace.folder_query.value, "ro q");
    assert!(!app.quit_armed);
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.workspace_search_focused());
    assert!(visible_folder_names(&app).is_empty());
    press(&mut app, KeyCode::Backspace);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(visible_folder_names(&app), vec!["/tmp/root".to_string()]);

    // ↓ leaves the row with the filter kept; space toggles the match.
    press(&mut app, KeyCode::Down);
    assert_eq!(
        app.workspace.cursor_folder(),
        Some(&PathBuf::from("/tmp/root"))
    );
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(
        app.workspaces.workspaces[0].folders,
        vec![PathBuf::from("/tmp/root")]
    );
    press(&mut app, KeyCode::Down);
    assert_eq!(
        app.workspace.cursor_folder(),
        Some(&PathBuf::from("/tmp/root")),
        "the cursor stays within the visible rows"
    );

    // ↑ returns to the row with the query selected; → drops the selection
    // without leaving the pane, and typing resumes at the end.
    press(&mut app, KeyCode::Up);
    assert!(app.workspace_search_focused());
    assert!(app.workspace.folder_query.select_all);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.workspace.pane, WorkspacePane::Detail);
    assert!(!app.workspace.folder_query.select_all);
    press(&mut app, KeyCode::Char('t'));
    assert_eq!(app.workspace.folder_query.value, "rot");
    press(&mut app, KeyCode::Esc);
    assert!(app.workspace.folder_query.value.is_empty());
    assert_eq!(app.workspace.visible.len(), 3);
    press(&mut app, KeyCode::Up);
    assert_eq!(
        app.workspace.cursor_field(),
        Some(super::state::WorkspaceField::Excludes)
    );
}

#[test]
fn arriving_on_the_folder_search_selects_the_query_so_typing_replaces_it() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "leaf");
    assert!(
        !app.workspace.folder_query.select_all,
        "typing never selects"
    );
    press(&mut app, KeyCode::Up);
    assert!(!app.workspace.folder_query.select_all);
    press(&mut app, KeyCode::Down);
    assert!(app.workspace.folder_query.select_all);
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.workspace.folder_query.value, "m");
    assert!(!app.workspace.folder_query.select_all);
}

#[test]
fn folder_search_arrows_move_the_text_cursor_and_never_the_pane() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "eaf");
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Down);
    // ← collapses the selection to the start; typing then inserts there.
    press(&mut app, KeyCode::Left);
    assert!(!app.workspace.folder_query.select_all);
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.workspace.folder_query.value, "leaf");
    for _ in 0..10 {
        press(&mut app, KeyCode::Left);
    }
    assert_eq!(app.workspace.pane, WorkspacePane::Detail);
    assert!(app.workspace_search_focused());
    press(&mut app, KeyCode::Delete);
    assert_eq!(app.workspace.folder_query.value, "eaf");
    press(&mut app, KeyCode::End);
    for _ in 0..10 {
        press(&mut app, KeyCode::Right);
    }
    assert_eq!(app.workspace.pane, WorkspacePane::Detail);
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.workspace.folder_query.value, "leaf");
    assert_eq!(visible_folder_names(&app), vec!["/tmp/leaf".to_string()]);

    // Off the row, ←/→ move between panes again.
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.workspace.pane, WorkspacePane::Sessions);
}

#[test]
fn folder_search_enter_jumps_to_the_first_match_and_scope_change_clears_it() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "leaf");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(
        app.workspace.cursor_folder(),
        Some(&PathBuf::from("/tmp/leaf"))
    );

    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Down);
    assert!(app.workspace.folder_query.value.is_empty());
    assert_eq!(app.workspace.visible.len(), 3);
}

#[test]
fn paste_on_the_folder_search_row_filters_folders() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    app.on_paste("tmp/mid");
    assert_eq!(app.workspace.folder_query.value, "tmp/mid");
    assert_eq!(visible_folder_names(&app), vec!["/tmp/middle".to_string()]);
}

#[test]
fn reopening_a_workspace_lists_its_folders_first() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.workspace.folders[0], PathBuf::from("/tmp/root"));
    assert_eq!(app.workspace.folders.len(), 3);
}

#[test]
fn all_is_neither_editable_nor_deletable() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    ctrl(&mut app, 'd');
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(
        app.status_msg.as_deref(),
        Some("The All workspace cannot be deleted")
    );
}

#[test]
fn ctrl_d_deletes_after_confirmation_and_keeps_the_row() {
    let mut app = app_with_workspace();
    app.workspaces
        .workspaces
        .push(Workspace::new("ws-web".into(), "Web".into()));
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    ctrl(&mut app, 'd');
    assert_eq!(app.mode, UiMode::WorkspaceDeleteConfirm);
    press(&mut app, KeyCode::Enter); // Cancel is focused first.
    assert_eq!(app.workspaces.workspaces.len(), 2);

    ctrl(&mut app, 'd');
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.workspaces.workspaces.len(), 1);
    assert_eq!(app.active_workspace_name(), Some("Web"));

    ctrl(&mut app, 'd');
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Enter);
    assert!(app.workspaces.workspaces.is_empty());
    assert_eq!(app.active_workspace_name(), None);
}

#[test]
fn sessions_pane_takes_the_session_list_keys() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Right);
    assert_eq!(app.workspace.pane, WorkspacePane::Sessions);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.selected, 1);
    press(&mut app, KeyCode::End);
    assert_eq!(app.selected, 2);
    ctrl(&mut app, 'd');
    assert_eq!(
        app.mode,
        UiMode::DeleteConfirm,
        "ctrl+d targets the session here"
    );
}

#[test]
fn search_from_a_workspace_pane_confirms_into_the_session_pane() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Char('/'));
    assert_eq!(app.mode, UiMode::Keyword);
    type_text(&mut app, "middle");
    assert_eq!(visible_ids(&app), ["middle"]);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.mode, UiMode::Table);
    assert_eq!(app.screen, Screen::Workspace);
    assert_eq!(app.workspace.pane, WorkspacePane::Sessions);
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

    // From the Workspaces screen too, the palette lands on the Session screen.
    on_workspace_screen(&mut app);
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
    assert_eq!(app.active_workspace_name(), None);
    assert_eq!(app.filtered.len(), 3);
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
fn edits_are_saved_and_the_open_workspace_survives_a_restart() {
    let root = TempBookmarkStore::new();
    let path = root.path.with_file_name("workspaces.json");
    let mut app = app_with_workspace();
    app.workspaces_path = Some(path.clone());
    // The fixture's workspace exists only in memory; a real one was loaded or saved.
    let api = app.workspaces.workspaces[0].clone();
    WorkspaceStore::commit(&path, &[crate::workspaces::WorkspaceChange::Upsert(api)]).unwrap();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    type_text(&mut app, "leaf");
    press(&mut app, KeyCode::Enter);

    let stored = WorkspaceStore::load(&path).expect("saved store");
    assert_eq!(stored.active.as_deref(), Some("ws-api"));
    assert_eq!(stored.workspaces[0].includes, "leaf");
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
fn workspace_screen_draws_three_panes_with_checked_folders() {
    let mut app = app_with_workspace();
    app.workspaces.workspaces[0].folders = vec![PathBuf::from("/tmp/root")];
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Right);
    let text = rendered(&app, 140, 24);
    assert!(text.contains(" Workspaces "), "{text}");
    assert!(text.contains(" Detail "), "{text}");
    assert!(text.contains("Session[Api: 1]"), "{text}");
    assert!(text.contains("[✓] root"), "{text}");
    assert!(text.contains("[ ] leaf"), "{text}");
    assert!(text.contains("Folders · 1 selected"), "{text}");

    // The list pane is capped at 24 cells: the Detail border starts right after.
    let row = text
        .lines()
        .find(|l| l.contains("Api") && l.contains("Name"))
        .expect("first list row beside the Name row");
    assert_eq!(row.chars().nth(23), Some('│'));
}

#[test]
fn folder_search_row_renders_query_and_empty_result() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    let text = rendered(&app, 140, 24);
    assert!(
        !text.contains("type to filter"),
        "focused: no placeholder\n{text}"
    );
    assert!(text.contains("Type to filter folders"), "{text}");
    press(&mut app, KeyCode::Up);
    let text = rendered(&app, 140, 24);
    assert!(text.contains("Search   type to filter"), "{text}");
    press(&mut app, KeyCode::Down);
    type_text(&mut app, "zz");
    let text = rendered(&app, 140, 24);
    assert!(text.contains("Search   zz"), "{text}");
    assert!(text.contains("No matching folders"), "{text}");
    assert!(text.contains("0 of 3 folders"), "{text}");
    assert!(!text.contains("[ ] leaf"), "{text}");
}

#[test]
fn folder_search_row_paints_only_the_selected_text() {
    let mut app = app_with_workspace();
    on_folder_search(&mut app);
    type_text(&mut app, "ro");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Up);
    let mut terminal = Terminal::new(TestBackend::new(140, 24)).expect("terminal");
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

#[test]
fn all_workspace_detail_is_locked() {
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    let text = rendered(&app, 140, 24);
    assert!(text.contains("All sessions · cannot be edited"), "{text}");
    assert!(text.contains("Folders · all folders"), "{text}");
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

#[test]
fn panes_without_focus_fade_like_the_session_screen() {
    use ratatui::style::Modifier;
    let mut app = app_with_workspace();
    on_workspace_screen(&mut app);
    press(&mut app, KeyCode::Down);
    assert_eq!(app.workspace.pane, WorkspacePane::List);
    let draw = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).expect("terminal");
        terminal
            .draw(|f| crate::ui::render::draw(f, app))
            .expect("draw");
        terminal.backend().buffer().clone()
    };
    let (list, detail, sessions) = (0..24, 24..64, 64..140);
    let muted = app.theme.muted;

    // List focused: Detail and the session table fade, and Detail's cursor row
    // keeps only the weak reversed signal.
    let buf = draw(&app);
    let list_row = cell_at(&buf, "Api", list.clone());
    assert_eq!(list_row.bg, app.theme.selection_bg);
    let name_value = cell_at(&buf, "Api", detail.clone());
    assert_eq!(name_value.fg, muted);
    assert!(name_value.modifier.contains(Modifier::REVERSED));
    assert_eq!(cell_at(&buf, "FOLDER", sessions.clone()).fg, muted);

    // Sessions focused: the list's open workspace and Detail fade instead.
    app.workspace.pane = WorkspacePane::Sessions;
    let buf = draw(&app);
    let list_row = cell_at(&buf, "Api", list.clone());
    assert_eq!(list_row.fg, muted);
    assert_eq!(list_row.bg, app.theme.selection_inactive_bg);
    assert!(list_row.modifier.contains(Modifier::REVERSED));
    assert_eq!(cell_at(&buf, "Api", detail.clone()).fg, muted);
    assert_eq!(
        cell_at(&buf, "FOLDER", sessions.clone()).fg,
        app.theme.accent
    );
}
