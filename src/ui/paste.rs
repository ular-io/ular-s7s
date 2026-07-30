//! Bracketed-paste routing: one terminal paste becomes at most one edit in the
//! focused editable field, and nothing else.
//!
//! Without bracketed paste the terminal decodes pasted text as ordinary key
//! events, so a pasted newline arrives as `KeyCode::Enter` — which submits the
//! rename/profile/New Session dialogs and *executes* the `!` terminal command.
//! `runtime` therefore enables the protocol and forwards `Event::Paste` here,
//! where the payload is sanitized to a single line and inserted as text only.
//! Execution and submission still require a physical Enter key event.
//!
//! The mode match is intentionally exhaustive: a new [`UiMode`] must make an
//! explicit routing decision rather than silently inherit "insert somewhere".

use crate::ui::components::input::MAX_PASTE_BYTES;
use crate::ui::{App, PasteOutcome, UiMode};

impl App {
    /// Routes one bracketed paste to the field that owns the caret in the current
    /// UI mode. Modes without an editable field ignore the paste entirely.
    pub fn on_paste(&mut self, text: &str) {
        match self.mode {
            UiMode::Keyword => self.paste_into_keyword(text),
            UiMode::Rename => self.paste_into_rename(text),
            UiMode::ProfileForm => self.paste_into_profile_form(text),
            UiMode::NewSession => self.paste_into_new_session(text),
            UiMode::QuickCommand => self.paste_into_quick(text),
            UiMode::FolderModal => self.paste_into_folder_query(text),
            // No editable field: a paste must not act as a key press.
            UiMode::Table
            | UiMode::AgentModal
            | UiMode::DeleteConfirm
            | UiMode::ProfileDeleteConfirm
            | UiMode::ProfileDirConfirm
            | UiMode::ProjectDirConfirm
            | UiMode::ThemeSelect
            | UiMode::Help
            | UiMode::Message => {}
        }
    }

    /// Reports what a paste had to drop. Silent when the whole payload landed.
    pub(crate) fn note_paste_outcome(&mut self, outcome: PasteOutcome) {
        if outcome.dropped_lines {
            self.status_msg =
                Some("Multi-line paste: only the first line was inserted".to_string());
        } else if outcome.truncated {
            self.status_msg = Some(format!(
                "Pasted text was truncated to {MAX_PASTE_BYTES} bytes"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MAX_PASTE_BYTES;
    use crate::ui::test_support::*;
    use crate::ui::{FormFocus, NewSessionFocus, RenameFocus, Screen, TerminalKind, UiMode};
    use crossterm::event::{KeyCode, KeyModifiers};

    /// Every mode that must ignore a paste, paired with a setup that enters it.
    #[test]
    fn paste_is_ignored_in_non_input_modes() {
        for mode in [
            UiMode::Table,
            UiMode::AgentModal,
            UiMode::DeleteConfirm,
            UiMode::ProfileDeleteConfirm,
            UiMode::ProfileDirConfirm,
            UiMode::ProjectDirConfirm,
            UiMode::ThemeSelect,
            UiMode::Help,
            UiMode::Message,
        ] {
            let mut app = app_with_session();
            app.mode = mode;
            app.on_paste("text\nwith breaks");
            assert_eq!(app.filter.keyword, "", "{mode:?} edited the keyword");
            assert!(app.terminal_request.is_none(), "{mode:?} queued a command");
            assert!(!app.should_quit, "{mode:?} quit");
        }
    }

    #[test]
    fn paste_reaches_the_keyword_box_and_recomputes() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::NONE));
        app.on_paste("hel");
        assert_eq!(app.filter.keyword, "hel");
        assert_eq!(app.keyword_cursor, 3);
        assert_eq!(app.filtered.len(), 1, "matching session stays visible");

        // Inserts at the cursor, not at the end.
        app.on_key_keyword(key(KeyCode::Home, KeyModifiers::NONE));
        app.on_paste("x");
        assert_eq!(app.filter.keyword, "xhel");
        assert_eq!(app.keyword_cursor, 1);
        // A non-matching keyword filters the list: the recompute really ran.
        assert!(app.filtered.is_empty());
    }

    #[test]
    fn keyword_paste_inserts_mode_switch_characters_literally() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::NONE));
        app.on_paste(":!/");
        assert_eq!(app.mode, UiMode::Keyword);
        assert_eq!(app.filter.keyword, ":!/");
    }

    #[test]
    fn paste_into_rename_does_not_submit_the_dialog() {
        let mut app = app_with_session();
        app.open_rename_modal();
        let modal = app.rename_modal.as_mut().expect("modal");
        modal.input.value.clear();
        modal.input.cursor = 0;
        app.on_paste("new\ntitle");
        let modal = app.rename_modal.as_ref().expect("still open");
        assert_eq!(modal.input.value, "new title");
        assert_eq!(app.mode, UiMode::Rename);
        assert!(app.pending_effect.is_none(), "rename must not be committed");
    }

    #[test]
    fn paste_is_ignored_while_rename_buttons_are_focused() {
        let mut app = app_with_session();
        app.open_rename_modal();
        {
            let modal = app.rename_modal.as_mut().expect("modal");
            modal.input.value.clear();
            modal.input.cursor = 0;
            modal.focus = RenameFocus::Buttons;
        }
        app.on_paste("nope");
        assert_eq!(app.rename_modal.as_ref().expect("modal").input.value, "");
    }

    #[test]
    fn paste_reaches_the_focused_profile_field_only() {
        let mut app = empty_app();
        app.screen = Screen::Profile;
        app.on_key_profile_table(key(KeyCode::Char('+'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::ProfileForm);
        {
            let form = app.profile_form.as_mut().expect("form");
            form.focus = FormFocus::Name;
            form.error = Some("stale".to_string());
        }
        app.on_paste("team\nprofile");
        {
            let form = app.profile_form.as_ref().expect("form");
            assert_eq!(form.name.value, "team profile");
            assert_eq!(form.path.value, "");
            assert!(form.error.is_none(), "editing clears the form error");
        }

        // Agent row owns no text input: the paste is dropped.
        app.profile_form.as_mut().expect("form").focus = FormFocus::Agent;
        app.on_paste("ignored");
        let form = app.profile_form.as_ref().expect("form");
        assert_eq!(form.name.value, "team profile");
        assert_eq!(form.path.value, "");
    }

    #[test]
    fn paste_reaches_the_new_session_folder_input_only() {
        let mut app = app_with_profiles();
        app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.mode, UiMode::NewSession);
        {
            let state = app.new_session.as_mut().expect("dialog");
            state.focus = NewSessionFocus::Folder;
            state.input.value.clear();
            state.input.cursor = 0;
            state.dropdown_open = false;
        }
        app.on_paste("/tmp/one\n/tmp/two");
        {
            let state = app.new_session.as_ref().expect("dialog");
            assert_eq!(state.input.value, "/tmp/one /tmp/two");
            assert!(state.dropdown_open, "on_input_edited ran");
        }
        assert!(
            app.new_session_request.is_none(),
            "paste must not start a session"
        );

        // Any other focus row ignores the paste.
        app.new_session.as_mut().expect("dialog").focus = NewSessionFocus::Buttons;
        app.on_paste("ignored");
        assert_eq!(
            app.new_session.as_ref().expect("dialog").input.value,
            "/tmp/one /tmp/two"
        );
    }

    #[test]
    fn paste_into_the_command_palette_refilters() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
        app.on_paste("refresh");
        let state = app.quick.as_ref().expect("palette");
        assert_eq!(state.input.value, "refresh");
        // The list was refiltered by the pasted query, not left at "all commands".
        assert!(
            state.items.iter().all(|i| i.spec().key == "refresh-all"),
            "palette matches recomputed: {:?}",
            state.items.iter().map(|i| i.spec().key).collect::<Vec<_>>()
        );
        assert_eq!(app.mode, UiMode::QuickCommand);
    }

    #[test]
    fn palette_paste_does_not_switch_modes() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char(':'), KeyModifiers::NONE));
        // `:`/`!` switch modes only as key presses on an empty input.
        app.on_paste("!");
        let state = app.quick.as_ref().expect("palette");
        assert_eq!(state.input.value, "!");
        assert_eq!(state.mode, crate::ui::quick::QuickMode::Palette);
    }

    #[test]
    fn terminal_paste_keeps_the_first_line_and_never_executes() {
        let mut app = app_with_cwd("/tmp");
        app.on_key_table(key(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::QuickCommand);
        app.on_paste("echo hi\nrm -rf /\n");
        {
            let state = app.quick.as_ref().expect("terminal window");
            assert_eq!(state.input.value, "echo hi");
            assert_eq!(state.term_typed, "echo hi", "history filter re-synced");
        }
        assert!(
            app.terminal_request.is_none(),
            "pasted newline must not run the command"
        );
        assert_eq!(
            app.status_msg.as_deref(),
            Some("Multi-line paste: only the first line was inserted")
        );

        // A real Enter is still required to execute.
        app.on_key_quick(key(KeyCode::Enter, KeyModifiers::NONE));
        let req = app.terminal_request.take().expect("explicit Enter runs");
        assert_eq!(req.command, "echo hi");
        assert_eq!(req.kind, TerminalKind::Command);
    }

    #[test]
    fn paste_reaches_the_folder_filter_query() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.mode, UiMode::FolderModal);
        app.on_paste("tm\np");
        assert_eq!(app.folder_query, "tm p");
    }

    #[test]
    fn oversized_paste_is_capped_and_reported() {
        let mut app = app_with_session();
        app.on_key_table(key(KeyCode::Char('/'), KeyModifiers::NONE));
        app.on_paste(&"a".repeat(MAX_PASTE_BYTES + 10));
        assert_eq!(app.filter.keyword.len(), MAX_PASTE_BYTES);
        assert!(app
            .status_msg
            .as_deref()
            .is_some_and(|m| m.starts_with("Pasted text was truncated")));
    }
}
