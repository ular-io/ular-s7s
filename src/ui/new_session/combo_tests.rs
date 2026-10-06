//! New Session combo rendering: name-only titles, the `▾` at each combo's inner
//! right end in its border's style, and values, notes, placeholder and the
//! Folder input cursor kept inside the value area left of the `▾`.

use crate::ui::test_support::*;
use crate::ui::*;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    backend::{Backend, TestBackend},
    buffer::Buffer,
    layout::Position,
    style::Modifier,
    Terminal,
};

/// Draws the screen at `width`×24; returns the buffer and the hardware cursor.
fn draw(app: &App, width: u16) -> (Buffer, Position) {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).expect("terminal");
    terminal
        .draw(|f| crate::ui::render::draw(f, app))
        .expect("draw");
    let cursor = terminal
        .backend_mut()
        .get_cursor_position()
        .expect("cursor");
    (terminal.backend().buffer().clone(), cursor)
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// A combo located by its title: the title row, the value row, the arrow
/// column, and the column of the combo's right border.
struct Combo {
    title_y: u16,
    value_y: u16,
    arrow_x: u16,
    right_x: u16,
}

fn combo(buf: &Buffer, title: &str) -> Combo {
    let title_y = (0..buf.area.height)
        .find(|&y| row(buf, y).contains(&format!(" {title} ")))
        .unwrap_or_else(|| panic!("{title} combo"));
    let value_y = title_y + 1;
    let corner = (0..buf.area.width)
        .rev()
        .find(|&x| matches!(buf[(x, title_y)].symbol(), "┐" | "┓"))
        .expect("right corner");
    let arrow_x = (0..buf.area.width)
        .find(|&x| buf[(x, value_y)].symbol() == "▾")
        .unwrap_or_else(|| panic!("{title} arrow:\n{}", row(buf, value_y)));
    Combo {
        title_y,
        value_y,
        arrow_x,
        right_x: corner,
    }
}

fn open_dialog() -> App {
    let mut app = app_with_profiles();
    app.theme = crate::theme::default_theme();
    app.selected = 1; // profile-x, /tmp
    app.on_key_table(key(KeyCode::Char('n'), KeyModifiers::CONTROL));
    let state = app.new_session.as_mut().expect("dialog");
    state.focus = NewSessionFocus::Profile;
    state.dropdown_open = false;
    app
}

#[test]
fn new_session_combos_title_the_name_and_align_the_arrow_at_the_inner_right_end() {
    let app = open_dialog();
    let (buf, _) = draw(&app, 80);
    let combos = ["Profile", "Model", "Folder"].map(|t| combo(&buf, t));
    for (title, c) in ["Profile", "Model", "Folder"].iter().zip(&combos) {
        let title_row = row(&buf, c.title_y);
        assert!(!title_row.contains('▾'), "{title}: {title_row}");
        // `▾`, the right padding cell, then the right border.
        assert_eq!(
            c.arrow_x + 2,
            c.right_x,
            "{title}: {}",
            row(&buf, c.value_y)
        );
        assert_eq!(buf[(c.arrow_x + 1, c.value_y)].symbol(), " ");
    }
    // One column for every arrow in the form.
    assert!(combos.iter().all(|c| c.arrow_x == combos[0].arrow_x));
}

#[test]
fn new_session_combo_arrow_takes_the_border_style() {
    let mut app = open_dialog();
    let th = crate::theme::default_theme();
    for focus in [
        NewSessionFocus::Profile,
        NewSessionFocus::Model,
        NewSessionFocus::Folder,
    ] {
        app.new_session.as_mut().unwrap().focus = focus;
        let (buf, _) = draw(&app, 80);
        for (title, f) in [
            ("Profile", NewSessionFocus::Profile),
            ("Model", NewSessionFocus::Model),
            ("Folder", NewSessionFocus::Folder),
        ] {
            let c = combo(&buf, title);
            let arrow = &buf[(c.arrow_x, c.value_y)];
            let border = &buf[(c.right_x, c.value_y)];
            assert_eq!(arrow.fg, border.fg, "{title}");
            if f == focus {
                assert_eq!(border.symbol(), "┃", "{title}");
                assert_eq!(arrow.fg, th.accent, "{title}");
                assert!(arrow.modifier.contains(Modifier::BOLD), "{title}");
            } else {
                assert_eq!(border.symbol(), "│", "{title}");
                assert_eq!(arrow.fg, th.dim, "{title}");
                assert!(!arrow.modifier.contains(Modifier::BOLD), "{title}");
            }
        }
    }
}

#[test]
fn new_session_long_values_and_the_folder_cursor_stay_left_of_the_arrow() {
    let mut app = open_dialog();
    app.profiles.profiles[1].name = "Team profile with a very long display name".repeat(2);
    app.usage.entry_mut("profile-x").phase = crate::usage::UsagePhase::NotLoggedIn;
    let state = app.new_session.as_mut().unwrap();
    state.model_options = vec![missing_model(
        "a-configured-model-name-that-is-not-listed-anymore",
        "a supplementary note long enough to run past the right edge of the box",
    )];
    state.model_idx = 0;
    let path = "/tmp/".to_string() + &"deeply/nested/folder/".repeat(6);
    state.input = TextInput::new(path);
    state.focus = NewSessionFocus::Folder;
    let th = crate::theme::default_theme();

    let (buf, cursor) = draw(&app, 80);
    for title in ["Profile", "Model", "Folder"] {
        let c = combo(&buf, title);
        // The gap cell before the `▾` is never written by a value.
        assert_eq!(
            buf[(c.arrow_x - 1, c.value_y)].symbol(),
            " ",
            "{title}: {}",
            row(&buf, c.value_y)
        );
    }
    // An invalid model is red, but its `▾` keeps the border style.
    let model = combo(&buf, "Model");
    assert_eq!(buf[(model.arrow_x, model.value_y)].fg, th.dim);
    // The cursor at the end of a long path stops left of the gap cell.
    let folder = combo(&buf, "Folder");
    assert_eq!(cursor.y, folder.value_y);
    assert!(cursor.x + 2 <= folder.arrow_x, "cursor {cursor:?}");
}

#[test]
fn new_session_open_dropdown_keeps_the_arrow_and_joins_in_accent() {
    let mut app = open_dialog();
    app.on_key_new_session(key(KeyCode::Enter, KeyModifiers::NONE)); // open Profile
    let (buf, _) = draw(&app, 80);
    let c = combo(&buf, "Profile");
    let th = crate::theme::default_theme();
    assert_eq!(buf[(c.arrow_x, c.value_y)].fg, th.accent);
    // The popup's `┣━┫` replaces the combo's bottom border in the same color.
    let join_y = c.value_y + 1;
    assert_eq!(
        buf[(c.right_x, join_y)].symbol(),
        "┫",
        "{}",
        row(&buf, join_y)
    );
    assert_eq!(buf[(c.right_x, join_y)].fg, th.accent);
}

/// A selected model that the profile's list no longer offers.
fn missing_model(label: &str, note: &str) -> crate::ui::new_session::state::ModelOption {
    crate::ui::new_session::state::ModelOption {
        value: Some(label.to_string()),
        label: label.to_string(),
        note: note.to_string(),
        missing: true,
    }
}
