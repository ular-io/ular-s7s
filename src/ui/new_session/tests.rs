//! New Session dialog tests: opening from each screen (ordinary and contextual),
//! profile/model/folder dropdown navigation and commit, focus rotation, launch
//! request construction, model default/last-selected resolution, and the
//! bare-name project-directory creation flow.

use crate::model::Agent;
use crate::models::LastSelection;
use crate::ui::new_session::state::is_bare_project_name;
use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};
use std::path::PathBuf;

/// Opens the new-session dialog from the Session view, then normalizes focus to the
/// (closed) Profile dropdown — the shared starting point for focus/dropdown flow tests.
/// The Session view itself now opens on the OK button; that behavior has its own
/// regression test (`ctrl_n_in_session_screen_focuses_ok_button`).
fn open_new_session_at_profile_focus(app: &mut App) {
    app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    app.new_session.as_mut().expect("new session dialog").focus = NewSessionFocus::Profile;
}

fn model_entry(value: &str) -> crate::models::ModelEntry {
    crate::models::ModelEntry {
        value: value.to_string(),
        label: value.to_string(),
        note: String::new(),
    }
}

fn profile_models(
    agent: Agent,
    values: &[&str],
    default_model: Option<&str>,
) -> crate::models::ProfileModels {
    crate::models::ProfileModels {
        agent,
        cli_version: None,
        models: values.iter().map(|v| model_entry(v)).collect(),
        default_model: default_model.map(str::to_string),
        last_selected: None,
    }
}

#[test]
fn new_session_folder_dropdown_selects_then_ok_starts() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.profile_selected = 1;

    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert_eq!(app.mode, UiMode::NewSession);
    {
        // Profile screen: starts with Folder focused (empty path) and dropdown open.
        let state = app.new_session.as_ref().expect("new session dialog");
        assert_eq!(state.focus, NewSessionFocus::Folder);
        assert!(state.dropdown_open);
        assert_eq!(state.folder_cursor, Some(0));
        assert_eq!(state.input.value, "");
        assert!(state.folders.contains(&PathBuf::from("/")));
    }

    // Down key navigates to "/tmp" option.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));

    // Enter key (dropdown open): commits selection and closes dropdown; does not trigger session launch yet.
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.new_session_request.is_none());
    {
        let state = app.new_session.as_ref().unwrap();
        assert!(!state.dropdown_open);
        assert_eq!(state.input.value, "/tmp");
    }

    // Down key (dropdown closed): moves focus from Folder to Buttons (OK focused). Enter launches session.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(state.ok_focused);
    }
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    let req = app
        .new_session_request
        .as_ref()
        .expect("new session request");
    assert_eq!(req.profile_id, "profile-x");
    assert_eq!(
        req.cwd,
        std::fs::canonicalize("/tmp").expect("canonicalize /tmp")
    );
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.new_session.is_none());
}

#[test]
fn new_session_space_selects_folder_and_keeps_dropdown_open() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;

    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    // Profile screen: starts with Folder focused (dropdown open at cursor 0). Down key moves cursor to 1 (/tmp).
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Char(' '), KeyModifiers::NONE));

    let state = app.new_session.as_ref().expect("new session dialog");
    assert!(state.dropdown_open); // Space key keeps dropdown open.
    assert_eq!(state.input.value, "/tmp"); // Synced back to input text box immediately.
                                           // Reordering post-selection must keep the cursor tracking same folder.
    let cursor_folder = state
        .folder_cursor
        .and_then(|c| state.ordered.get(c))
        .and_then(|&i| state.folders.get(i));
    assert_eq!(cursor_folder, Some(&PathBuf::from("/tmp")));
}

#[test]
fn new_session_right_does_not_complete_when_dropdown_open() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.profile_selected = 1;

    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    // Profile screen: starts with Folder focused (dropdown open at cursor 0). Down key moves cursor to 1 (/tmp).
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE));

    // → selection & completion removed: dropdown stays open and input text is unchanged.
    let state = app.new_session.as_ref().expect("new session dialog");
    assert!(state.dropdown_open);
    assert_eq!(state.input.value, "");
    assert_eq!(state.folder_cursor, Some(1));
}

#[test]
fn new_session_right_opens_both_dropdowns_when_closed() {
    let mut app = app_with_profiles();
    app.selected = 1; // directory populated -> focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );

    // → key (Profile closed): opens dropdown and places cursor on current selection.
    app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert!(state.dropdown_open);
        assert_eq!(state.profile_cursor, state.profile_idx);
    }

    // Esc closes it, then move to Folder focus and reopen using →.
    app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
    assert!(!app.new_session.as_ref().unwrap().dropdown_open);

    // → key (Folder closed): opens dropdown and highlights the first option.
    app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE));
    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.focus, NewSessionFocus::Folder);
    assert!(state.dropdown_open);
    assert_eq!(state.folder_cursor, Some(0));
}

#[test]
fn new_session_typing_autoopens_and_reorders_matches_first() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;

    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    app.on_key_new_session(key(KeyCode::Char('t'), KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Char('m'), KeyModifiers::NONE));

    let state = app.new_session.as_ref().expect("new session dialog");
    assert_eq!(state.input.value, "tm");
    assert!(state.dropdown_open); // Auto-opens dropdown on typing.
                                  // Matching option ("/tmp") at top, non-matching ("/") preserved at bottom.
    assert_eq!(state.match_count, 1);
    assert_eq!(state.ordered.len(), state.folders.len());
    assert_eq!(state.folders[state.ordered[0]], PathBuf::from("/tmp"));
    assert_eq!(state.folders[state.ordered[1]], PathBuf::from("/"));
}

#[test]
fn new_session_tab_toggles_focus_and_closes_dropdown() {
    let mut app = app_with_profiles();
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );

    // Tab key with dropdown open -> closes dropdown and moves focus to Model.
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.new_session.as_ref().unwrap().dropdown_open);
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Model);
        assert!(!state.dropdown_open);
    }

    // Shift+Tab key returns focus back to Profile.
    app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );
}

#[test]
fn new_session_tab_commits_dropdown_selection_before_moving_focus() {
    let mut app = app_with_profiles();
    app.selected = 1; // profile-x, /tmp — focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);

    // Profile dropdown: moves cursor to 0 (builtin-claude) then Tab ->
    // commits selection and shifts focus to Model.
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.profile_idx, 0);
        assert_eq!(state.focus, NewSessionFocus::Model);
        assert!(!state.dropdown_open);
    }

    // Folder dropdown: committed selection reflects on text box, shifting focus back via Shift+Tab.
    // Sort order by input "/tmp": ["/tmp" (match), "/"] - second item is "/".
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 0=/tmp)
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // cursor 1=/
    app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.input.value, "/");
    assert_eq!(state.focus, NewSessionFocus::Model);
    assert!(!state.dropdown_open);
}

#[test]
fn new_session_buttons_row_ok_and_cancel() {
    let mut app = app_with_profiles();
    app.selected = 1; // input "/tmp", focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);

    // Tab x 3: Profile -> Model -> Folder -> OK (first). Tab again -> Cancel.
    // Shift+Tab -> OK.
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(state.ok_focused); // OK is the initial button stop.
    }
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    assert!(!app.new_session.as_ref().unwrap().ok_focused); // -> Cancel
    app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert!(app.new_session.as_ref().unwrap().ok_focused); // returns to OK.

    // Shifts to Cancel then Enter -> closes dialog without starting.
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // OK -> Cancel
    assert!(!app.new_session.as_ref().unwrap().ok_focused);
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.new_session.is_none());
    assert!(app.new_session_request.is_none());
    assert_eq!(app.mode, UiMode::Table);

    // Reopens and Shift+Tab wraps back: Profile -> Cancel (the last stop).
    open_new_session_at_profile_focus(&mut app);
    app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(!state.ok_focused);
    }

    // Cycles forward via Tab to OK, then Enter -> launches session.
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Cancel -> Profile
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Folder -> OK
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(state.ok_focused);
    }
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    let req = app
        .new_session_request
        .as_ref()
        .expect("new session request");
    assert_eq!(req.profile_id, "profile-x");
}

#[test]
fn new_session_profile_dropdown_space_enter_up_flow() {
    let mut app = app_with_profiles();
    app.selected = 1; // s2: profile-x -> default profile_idx 1, focus normalized to Profile.
    open_new_session_at_profile_focus(&mut app);

    // Enter: opens dropdown placing cursor on the active profile selection.
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert!(state.dropdown_open);
        assert_eq!(state.profile_cursor, 1);
    }

    // Up key moves to top item, then Space: selects it while keeping dropdown open.
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Char(' '), KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert!(state.dropdown_open);
        assert_eq!(state.profile_idx, 0);
    }

    // Up key on top item (cursor 0): cycles to bottom item without closing dropdown.
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert!(state.dropdown_open); // Stays open.
        assert_eq!(state.profile_cursor, 1); // 0 -> bottom index.
    }

    // Down key on bottom item (cursor 1): cycles back to top (0), then Down key returns to 1.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.new_session.as_ref().unwrap().profile_cursor, 0);
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.new_session.as_ref().unwrap().profile_cursor, 1);

    // Enter: commits selection and closes dropdown (does not start session).
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    let state = app.new_session.as_ref().unwrap();
    assert!(!state.dropdown_open);
    assert_eq!(state.profile_idx, 1);
    assert!(app.new_session_request.is_none()); // Enter selection does not launch session.
}

#[test]
fn new_session_esc_closes_dropdown_first_then_dialog() {
    let mut app = app_with_profiles();
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);

    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // move cursor
    app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().expect("dialog stays open");
        assert!(!state.dropdown_open);
        assert_eq!(state.profile_idx, 1); // Esc key does not commit active cursor selection.
    }

    app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.new_session.is_none());
}

#[test]
fn new_session_folder_updown_wraps_around() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile; // initially focused on Folder (empty folders).
    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    // Profile screen: starts dropdown open (cursor 0). Up key cycles from top to bottom.
    assert_eq!(app.new_session.as_ref().unwrap().folder_cursor, Some(0));
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    let last = {
        let state = app.new_session.as_ref().unwrap();
        assert!(state.dropdown_open);
        state.ordered.len() - 1
    };
    assert_eq!(app.new_session.as_ref().unwrap().folder_cursor, Some(last));

    // Down key cycles from bottom back to top (0).
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.new_session.as_ref().unwrap().folder_cursor, Some(0));
}

#[test]
fn new_session_text_input_allows_plain_k_character() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;

    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    app.on_key_new_session(key(KeyCode::Char('k'), KeyModifiers::NONE));

    let state = app.new_session.as_ref().expect("new session dialog");
    assert_eq!(state.input.value, "k");
}

#[test]
fn profile_enter_no_longer_opens_new_session_dialog() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;

    app.on_key_profile_table(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::Table);
    assert!(app.new_session.is_none());
}

#[test]
fn ctrl_n_in_profile_screen_opens_dialog_with_selected_profile_and_empty_folder() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.profile_selected = 1;

    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, UiMode::NewSession);
    let state = app.new_session.as_ref().expect("new session dialog");
    assert_eq!(state.profile_idx, 1);
    assert_eq!(state.input.value, "");
    // Starts with Folder focused and dropdown open due to empty folders.
    assert_eq!(state.focus, NewSessionFocus::Folder);
    assert!(state.dropdown_open);
    assert_eq!(state.folder_cursor, Some(0));
}

#[test]
fn ctrl_n_in_session_screen_focuses_ok_button() {
    let mut app = app_with_profiles();
    // Selects the second session (s2: profile-x, /tmp).
    app.selected = 1;

    app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, UiMode::NewSession);
    let state = app.new_session.as_ref().expect("new session dialog");
    assert_eq!(state.profile_idx, 1); // profile-x
    assert_eq!(state.input.value, "/tmp");
    // Session view opens with the OK button focused for a quick start.
    assert_eq!(state.focus, NewSessionFocus::Buttons);
    assert!(state.ok_focused);
    assert!(!state.dropdown_open);
}

#[test]
fn ctrl_n_in_detail_screen_defaults_to_detail_session_profile() {
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);

    app.on_key_detail(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    assert_eq!(app.mode, UiMode::NewSession);
    let state = app.new_session.as_ref().expect("new session dialog");
    assert_eq!(state.profile_idx, 1);
    assert_eq!(state.input.value, "/tmp");
    // Detail view keeps the Profile dropdown focused (unlike the Session view).
    assert_eq!(state.focus, NewSessionFocus::Profile);
}

#[test]
fn ctrl_n_opens_ordinary_dialog_without_context() {
    let mut app = app_with_profiles();
    app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    let state = app.new_session.as_ref().expect("dialog");
    assert!(state.context.is_none());
}

#[test]
fn ctrl_shift_n_opens_contextual_dialog_with_selected_source() {
    let mut app = app_with_profiles();
    app.selected = 1; // s2: profile-x, /tmp

    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));

    assert_eq!(app.mode, UiMode::NewSession);
    let state = app.new_session.as_ref().expect("dialog");
    let ctx = state.context.as_ref().expect("context captured");
    assert_eq!(ctx.agent, Agent::Claude);
    assert_eq!(ctx.profile_id, "profile-x");
    assert_eq!(ctx.session_id, "s2");
    // Target defaults mirror ordinary New Session (source session's profile/cwd).
    assert_eq!(state.profile_idx, 1);
    assert_eq!(state.input.value, "/tmp");
}

#[test]
fn ctrl_shift_n_accepts_uppercase_char_form() {
    // Enhanced-keyboard terminals may report the chord as Char('N').
    let mut app = app_with_profiles();
    app.on_key_table(key(
        KeyCode::Char('N'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(app
        .new_session
        .as_ref()
        .is_some_and(|s| s.context.is_some()));
}

#[test]
fn ctrl_shift_n_without_focused_session_is_rejected() {
    let mut app = empty_app();
    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.new_session.is_none());
    assert_eq!(app.status_msg.as_deref(), Some("Select a session first"));
}

#[test]
fn profile_screen_ctrl_shift_n_is_not_captured_as_ordinary_new_session() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(app.new_session.is_none());
}

#[test]
fn detail_ctrl_shift_n_captures_detail_source_session() {
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    app.on_key_table(key(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.screen, Screen::Detail);

    app.on_key_detail(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));

    let ctx = app
        .new_session
        .as_ref()
        .and_then(|s| s.context.as_ref())
        .expect("context from detail source");
    assert_eq!(ctx.session_id, "s2");
}

#[test]
fn changing_target_profile_preserves_source_context() {
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    let before = app.new_session.as_ref().unwrap().context.clone().unwrap();

    // Switch the target profile via the dropdown (open -> up -> enter).
    app.new_session.as_mut().unwrap().focus = NewSessionFocus::Profile;
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    // Edit the folder input as well.
    let state = app.new_session.as_mut().unwrap();
    state.focus = NewSessionFocus::Folder;
    app.on_key_new_session(key(KeyCode::Char('x'), KeyModifiers::NONE));

    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.profile_idx, 0); // target changed
    assert_eq!(state.context.as_ref(), Some(&before)); // source immutable
}

#[test]
fn ok_transfers_context_into_request_and_cancel_discards_it() {
    // OK path.
    let mut app = app_with_profiles();
    app.selected = 1; // cwd /tmp exists on disk
    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(app.new_session.as_ref().unwrap().ok_focused);
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    let req = app.new_session_request.take().expect("request");
    let ctx = req.context.expect("context travels with the request");
    assert_eq!(ctx.session_id, "s2");
    assert_eq!(ctx.profile_id, "profile-x");

    // Cancel path.
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.new_session.is_none());
    assert!(app.new_session_request.is_none());
}

#[test]
fn contextual_ok_aborts_when_source_session_disappeared() {
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    // Source disappears while the dialog is open (delete/refresh race).
    app.sessions.retain(|s| s.id != "s2");
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.new_session_request.is_none());
    let state = app.new_session.as_ref().expect("dialog stays open");
    assert!(state
        .error
        .as_deref()
        .is_some_and(|e| e.contains("Source session not found")));
}

#[test]
fn quick_command_invokes_contextual_opener() {
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
    for c in "attach-session".chars() {
        app.on_key_quick(key(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::NewSession);
    let ctx = app
        .new_session
        .as_ref()
        .and_then(|s| s.context.as_ref())
        .expect("palette opens contextual dialog");
    assert_eq!(ctx.session_id, "s2");
}

#[test]
fn contextual_source_control_is_not_focusable() {
    let mut app = app_with_profiles();
    app.selected = 1;
    app.on_key_table(key(
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    let state = app.new_session.as_mut().expect("contextual dialog");
    state.focus = NewSessionFocus::Profile;

    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Model
    );
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Model
    );
}

#[test]
fn new_session_ctrl_n_p_no_longer_cycle_profile() {
    // Profile selection is unified into dropdown - Ctrl+N/P cycle shortcuts are removed.
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.profile_selected = 0;
    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    app.on_key_new_session(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    app.on_key_new_session(key(KeyCode::Char('p'), KeyModifiers::CONTROL));

    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.profile_idx, 0);
    assert_eq!(state.input.value, ""); // Ctrl key combinations are not written to the text input.
}

#[test]
fn new_session_enter_with_empty_folder_shows_error() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    // Closes open dropdown using Esc leaving folder input empty, moves focus to OK button via Down key, then Enter.
    app.on_key_new_session(key(KeyCode::Esc, KeyModifiers::NONE));
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert!(app.new_session.as_ref().unwrap().ok_focused);
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.mode, UiMode::NewSession);
    assert!(app.new_session_request.is_none());
    let state = app.new_session.as_ref().expect("dialog stays open");
    assert_eq!(state.error.as_deref(), Some("Select a folder first"));
}

#[test]
fn bare_project_name_detection() {
    assert!(is_bare_project_name("myproj"));
    assert!(is_bare_project_name("my.proj"));
    assert!(is_bare_project_name("my proj"));
    assert!(!is_bare_project_name("foo/bar"));
    assert!(!is_bare_project_name("./foo"));
    assert!(!is_bare_project_name("~/foo"));
    assert!(!is_bare_project_name("~foo"));
}

#[test]
fn new_session_bare_name_missing_opens_project_dir_confirm_and_cancel_returns() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    let name = format!("s7s-test-missing-project-{}", std::process::id());
    app.new_session.as_mut().unwrap().input.value = name.clone();
    app.confirm_new_session();

    assert_eq!(app.mode, UiMode::ProjectDirConfirm);
    assert!(app.dir_create_ok_focused);
    assert_eq!(
        app.project_dir_pending.as_deref(),
        Some(crate::config::projects_dir().join(&name).as_path())
    );
    assert!(app.new_session_request.is_none());

    // Cancel returns to the New Session dialog with the typed name kept.
    app.on_key_project_dir_confirm(key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, UiMode::NewSession);
    assert!(app.project_dir_pending.is_none());
    assert_eq!(app.new_session.as_ref().unwrap().input.value, name);
    assert!(app.new_session_request.is_none());
}

#[test]
fn project_dir_create_makes_folder_and_starts_session() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    // Pending path stands in for projects_dir()/<name> so the test never
    // touches the real user config directory.
    let dir = std::env::temp_dir().join(format!("s7s-test-project-create-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.project_dir_pending = Some(dir.clone());
    app.mode = UiMode::ProjectDirConfirm;
    app.dir_create_ok_focused = true;

    app.on_key_project_dir_confirm(key(KeyCode::Enter, KeyModifiers::NONE));

    assert!(dir.is_dir());
    let req = app.new_session_request.as_ref().expect("request issued");
    assert_eq!(req.cwd, std::fs::canonicalize(&dir).unwrap());
    assert_eq!(app.mode, UiMode::Table);
    assert!(app.new_session.is_none());
    assert!(app.project_dir_pending.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_session_missing_path_input_still_errors() {
    let mut app = app_with_profiles();
    app.screen = Screen::Profile;
    app.on_key_profile_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));

    // Path-form input (contains a separator) must keep the current error behavior
    // instead of offering project creation.
    app.new_session.as_mut().unwrap().input.value = "/definitely/missing/s7s-path".to_string();
    app.confirm_new_session();

    assert_eq!(app.mode, UiMode::NewSession);
    assert!(app.project_dir_pending.is_none());
    assert!(app.new_session_request.is_none());
    let state = app.new_session.as_ref().expect("dialog stays open");
    assert!(state
        .error
        .as_deref()
        .is_some_and(|e| e.starts_with("Cannot open path")));
}

#[test]
fn new_session_enter_uses_profile_selected_in_dropdown() {
    let mut app = app_with_profiles();
    app.selected = 1; // s2: profile-x, /tmp - focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);

    // Changes profile to builtin-claude (index 0) in dropdown, closes it, and launches via Enter on OK button.
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Profile -> Model
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Model -> Folder
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Folder -> Buttons(OK)
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start

    let req = app
        .new_session_request
        .as_ref()
        .expect("new session request");
    assert_eq!(req.profile_id, "builtin-claude");
    assert_eq!(
        req.cwd,
        std::fs::canonicalize("/tmp").expect("canonicalize /tmp")
    );
}

#[test]
fn new_session_updown_move_focus_when_dropdown_closed() {
    let mut app = app_with_profiles();
    app.selected = 1; // folder populated -> focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );

    // Down key: Profile -> Model -> Folder -> Buttons. Dropdown is not opened.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Model);
        assert!(!state.dropdown_open);
    }
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Folder
    );
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Buttons
    );
    // Down key at bottom wraps back to the top (rotation, like Tab).
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );

    // Up key at top wraps to the button row, focusing OK first.
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.focus, NewSessionFocus::Buttons);
        assert!(state.ok_focused);
    }

    // Up key: Buttons -> Folder -> Model -> Profile.
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Folder
    );
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Model
    );
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Profile
    );
}

#[test]
fn new_session_down_into_buttons_always_focuses_ok() {
    let mut app = app_with_profiles();
    app.selected = 1; // folder populated -> focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);

    // Moves focus to button row, moves to Cancel, climbs back to Folder, then moves Down again.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Profile -> Model
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Model -> Folder
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Folder -> Buttons(OK)
    app.on_key_new_session(key(KeyCode::Right, KeyModifiers::NONE)); // OK -> Cancel
    assert!(!app.new_session.as_ref().unwrap().ok_focused);
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // Buttons -> Folder
    assert_eq!(
        app.new_session.as_ref().unwrap().focus,
        NewSessionFocus::Folder
    );

    // Entering button row again -> focuses OK first, independent of the prior Cancel selection.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE));
    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.focus, NewSessionFocus::Buttons);
    assert!(state.ok_focused);
}

#[test]
fn new_session_model_dropdown_selects_and_passes_model() {
    let mut app = app_with_profiles();
    app.models.insert(
        "profile-x".to_string(),
        profile_models(Agent::Claude, &["opus", "fable", "sonnet"], Some("fable")),
    );
    app.selected = 1; // s2: profile-x, /tmp - focus normalized to Profile for this flow.
    open_new_session_at_profile_focus(&mut app);

    // Initial selection = default model from settings ("fable", index 2 including "Default").
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.model_options.len(), 4); // Default + 3
        assert_eq!(state.model_idx, 2);
        assert_eq!(state.model_options[2].value.as_deref(), Some("fable"));
    }

    // Selecting "opus" in Model dropdown then OK -> injected model configuration included in the request.
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 2)
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 1 (opus)
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
    assert_eq!(app.new_session.as_ref().unwrap().model_idx, 1);
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Model -> Folder
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // Folder -> Buttons(OK)
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start
    let req = app.new_session_request.as_ref().expect("request");
    assert_eq!(req.model.as_deref(), Some("opus"));
}

#[test]
fn new_session_default_model_passes_no_model() {
    let mut app = app_with_profiles();
    // If default model is absent from cache, initial selection points to "Default" (no injection).
    app.models.insert(
        "profile-x".to_string(),
        profile_models(Agent::Claude, &["opus", "sonnet"], None),
    );
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    assert_eq!(app.new_session.as_ref().unwrap().model_idx, 0);

    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Model
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Folder
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Buttons(OK)
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    let req = app.new_session_request.as_ref().expect("request");
    assert_eq!(req.model, None);
}

#[test]
fn new_session_missing_default_model_disables_ok_until_reselected() {
    let mut app = app_with_profiles();
    // Default model ("legacy") absent from options catalog -> placeholder item "missing" is selected.
    app.models.insert(
        "profile-x".to_string(),
        profile_models(Agent::Claude, &["opus", "sonnet"], Some("legacy")),
    );
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    {
        let state = app.new_session.as_ref().unwrap();
        assert_eq!(state.model_idx, 1);
        assert!(state.model_options[1].missing);
    }

    // Enter on OK while "missing" is selected must block execution and only show error message.
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // -> Model
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // -> Folder
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // -> OK
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.new_session_request.is_none());
    {
        let state = app.new_session.as_ref().expect("dialog stays open");
        assert!(state.error.as_deref().unwrap().contains("Model"));
    }

    // Re-selecting to "Default" (no injection) enables execution again.
    app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT)); // OK -> Folder
    app.on_key_new_session(key(KeyCode::BackTab, KeyModifiers::SHIFT)); // Folder -> Model
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0 (Default)
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Model -> Folder
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Folder -> OK
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE));
    let req = app.new_session_request.as_ref().expect("request");
    assert_eq!(req.model, None);
}

#[test]
fn new_session_profile_change_rebuilds_model_options() {
    let mut app = app_with_profiles();
    // Only builtin-claude has cached models. profile-x has no cache -> falls back to embedded Claude models
    // (fable / opus / sonnet / haiku).
    app.models.insert(
        "builtin-claude".to_string(),
        profile_models(Agent::Claude, &["opus"], Some("opus")),
    );
    app.selected = 1; // profile-x
    open_new_session_at_profile_focus(&mut app);
    assert_eq!(app.new_session.as_ref().unwrap().model_options.len(), 5);

    // Swapping profile to builtin-claude -> model options list is reconstructed.
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 1)
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 0
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.profile_idx, 0);
    assert_eq!(state.model_options.len(), 2); // Default + opus
    assert_eq!(state.model_idx, 1); // Initial selection default model "opus".
}

#[test]
fn new_session_last_selected_model_overrides_cli_default() {
    let mut app = app_with_profiles();
    let mut pm = profile_models(Agent::Claude, &["opus", "fable", "sonnet"], Some("fable"));
    pm.last_selected = Some(LastSelection::Model("sonnet".to_string()));
    app.models.insert("profile-x".to_string(), pm);
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    let state = app.new_session.as_ref().unwrap();
    // "sonnet" (index 3: Default + opus/fable/sonnet) beats CLI default "fable" (index 2).
    assert_eq!(state.model_idx, 3);
    assert_eq!(state.model_options[3].value.as_deref(), Some("sonnet"));
}

#[test]
fn new_session_last_selected_default_overrides_cli_default() {
    let mut app = app_with_profiles();
    // CLI default "fable" would normally select index 2, but a remembered "Default" pick
    // must select index 0 and never surface a placeholder.
    let mut pm = profile_models(Agent::Claude, &["opus", "fable"], Some("fable"));
    pm.last_selected = Some(LastSelection::Default);
    app.models.insert("profile-x".to_string(), pm);
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    assert_eq!(app.new_session.as_ref().unwrap().model_idx, 0);
}

#[test]
fn new_session_stale_last_selected_falls_back_to_cli_default() {
    let mut app = app_with_profiles();
    // Last pick "legacy" is no longer in the list -> skip it and use CLI default "opus".
    let mut pm = profile_models(Agent::Claude, &["opus", "sonnet"], Some("opus"));
    pm.last_selected = Some(LastSelection::Model("legacy".to_string()));
    app.models.insert("profile-x".to_string(), pm);
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    let state = app.new_session.as_ref().unwrap();
    assert_eq!(state.model_idx, 1); // Default + opus -> opus at 1
    assert_eq!(state.model_options[1].value.as_deref(), Some("opus"));
    assert!(state.model_options.iter().all(|o| !o.missing));
}

#[test]
fn new_session_launch_records_last_selected() {
    let mut app = app_with_profiles();
    app.models.insert(
        "profile-x".to_string(),
        profile_models(Agent::Claude, &["opus", "fable", "sonnet"], Some("fable")),
    );
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    // Pick "opus" and launch.
    app.on_key_new_session(key(KeyCode::Tab, KeyModifiers::NONE)); // Profile -> Model
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open (cursor 2 = fable)
    app.on_key_new_session(key(KeyCode::Up, KeyModifiers::NONE)); // cursor 1 = opus
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // select & close
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Folder
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> OK
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start
    assert_eq!(
        app.new_session_request.as_ref().unwrap().model.as_deref(),
        Some("opus")
    );
    // The pick is remembered in the in-memory catalog for next time.
    assert_eq!(
        app.models
            .for_profile(&app.profiles.profiles[1])
            .and_then(|m| m.last_selected.clone()),
        Some(LastSelection::Model("opus".to_string()))
    );
}

#[test]
fn new_session_launch_default_records_default_selection() {
    let mut app = app_with_profiles();
    app.models.insert(
        "profile-x".to_string(),
        profile_models(Agent::Claude, &["opus", "sonnet"], None),
    );
    app.selected = 1;
    open_new_session_at_profile_focus(&mut app);
    // Initial selection is Default (idx 0) because no CLI default; launch as-is.
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Model
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> Folder
    app.on_key_new_session(key(KeyCode::Down, KeyModifiers::NONE)); // -> OK
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // start
    assert_eq!(app.new_session_request.as_ref().unwrap().model, None);
    assert_eq!(
        app.models
            .for_profile(&app.profiles.profiles[1])
            .and_then(|m| m.last_selected.clone()),
        Some(LastSelection::Default)
    );
}
