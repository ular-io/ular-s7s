use super::*;

/// `e` + COMBINING ACUTE ACCENT: one grapheme, two scalars, three bytes.
const COMBINING: &str = "e\u{301}";
/// Family emoji joined by ZERO WIDTH JOINER: one grapheme, 25 bytes.
const FAMILY: &str = "👨‍👩‍👧‍👦";

fn input(value: &str) -> TextInput {
    TextInput::new(value.to_string())
}

/// A prefilled path is selected, so one key replaces or clears all of it.
#[test]
fn a_selected_value_is_replaced_by_the_first_edit() {
    let mut i = TextInput::selected("/Users/dev/projects/demo".to_string());
    assert!(i.select_all);

    i.insert_char('u');

    assert_eq!(i.value, "u");
    assert_eq!(i.cursor, 1);
    assert!(!i.select_all);
}

#[test]
fn backspace_on_a_selected_value_clears_it_once() {
    let mut i = TextInput::selected("/Users/dev/projects/demo".to_string());

    i.backspace();

    assert_eq!(i.value, "");
    assert_eq!(i.cursor, 0);
    assert!(!i.select_all);
    // A second Backspace has nothing left to take, so the field stays empty.
    i.backspace();
    assert_eq!(i.value, "");
}

#[test]
fn delete_on_a_selected_value_clears_it() {
    let mut i = TextInput::selected("demo".to_string());
    i.delete();
    assert_eq!(i.value, "");
    assert!(!i.select_all);
}

#[test]
fn paste_replaces_a_selected_value_instead_of_appending() {
    let mut i = TextInput::selected("/old/path".to_string());

    let outcome = i.insert_paste("/new/path");

    assert_eq!(i.value, "/new/path");
    assert_eq!(outcome.inserted, "/new/path".len());
}

/// The edit flow: a cursor move drops the selection and keeps the text, so an
/// existing path can be corrected without retyping it.
#[test]
fn a_cursor_move_collapses_the_selection_to_that_edge() {
    let value = "/Users/dev/projects/demo";

    let mut right = TextInput::selected(value.to_string());
    right.move_right();
    assert_eq!(right.value, value);
    assert_eq!(right.cursor, value.len());
    assert!(!right.select_all);

    let mut left = TextInput::selected(value.to_string());
    left.move_left();
    assert_eq!(left.value, value);
    assert_eq!(left.cursor, 0);
    assert!(!left.select_all);

    let mut home = TextInput::selected(value.to_string());
    home.home();
    assert_eq!(home.value, value);
    assert_eq!(home.cursor, 0);
    assert!(!home.select_all);

    let mut end = TextInput::selected(value.to_string());
    end.end();
    assert_eq!(end.value, value);
    assert_eq!(end.cursor, value.len());
    assert!(!end.select_all);
}

#[test]
fn a_typed_value_is_never_selected() {
    let mut i = input("");
    i.insert_char('a');
    assert!(!i.select_all);
}

#[test]
fn movement_treats_combining_sequence_as_one_grapheme() {
    let mut i = input(COMBINING);
    assert_eq!(i.cursor, COMBINING.len());
    i.move_left();
    assert_eq!(i.cursor, 0);
    i.move_right();
    assert_eq!(i.cursor, COMBINING.len());
}

#[test]
fn backspace_and_delete_remove_the_whole_combining_sequence() {
    let mut i = input(COMBINING);
    i.backspace();
    assert_eq!(i.value, "");
    assert_eq!(i.cursor, 0);

    let mut i = input(COMBINING);
    i.home();
    i.delete();
    assert_eq!(i.value, "");
    assert_eq!(i.cursor, 0);
}

#[test]
fn movement_and_deletion_keep_zwj_emoji_intact() {
    let mut i = input(&format!("a{FAMILY}b"));
    i.home();
    i.move_right();
    assert_eq!(i.cursor, 1);
    i.move_right();
    assert_eq!(i.cursor, 1 + FAMILY.len());
    i.move_left();
    assert_eq!(i.cursor, 1);

    // Delete at the cluster start removes the entire emoji, not one scalar.
    i.delete();
    assert_eq!(i.value, "ab");

    let mut i = input(&format!("a{FAMILY}"));
    i.backspace();
    assert_eq!(i.value, "a");
}

#[test]
fn inserting_a_combining_mark_keeps_the_cursor_after_typed_text() {
    let mut i = input("");
    i.insert_char('e');
    i.insert_char('\u{301}');
    assert_eq!(i.value, COMBINING);
    assert_eq!(i.cursor, COMBINING.len());
    // One grapheme now: a single backspace clears the field.
    i.backspace();
    assert_eq!(i.value, "");
}

#[test]
fn insert_before_an_existing_mark_does_not_skip_it() {
    // Cursor between the base char and its combining mark stays there, so the
    // mark remains reachable instead of being jumped over.
    let mut i = input(COMBINING);
    i.move_left();
    i.insert_char('x');
    assert_eq!(i.value, format!("xe\u{301}"));
    assert_eq!(i.cursor, 1);
}

#[test]
fn paste_normalizes_line_breaks_tabs_and_control_characters() {
    let mut i = input("");
    // CRLF, lone CR, LF and tab become single spaces; BEL is dropped outright.
    let outcome = i.insert_paste("a\r\nb\rc\nd\te\u{7}f");
    assert_eq!(i.value, "a b c d ef");
    assert_eq!(i.cursor, i.value.len());
    assert_eq!(outcome.inserted, "a b c d ef".len());
    assert!(!outcome.truncated);
    assert!(!outcome.dropped_lines);
}

#[test]
fn paste_inserts_at_the_cursor_in_one_edit() {
    let mut i = input("ac");
    i.home();
    i.move_right();
    let outcome = i.insert_paste("b");
    assert_eq!(i.value, "abc");
    assert_eq!(i.cursor, 2);
    assert_eq!(outcome.inserted, 1);
}

#[test]
fn first_line_policy_keeps_only_the_first_line() {
    let mut i = input("");
    let outcome = i.insert_paste_first_line("echo hi\nrm -rf /");
    assert_eq!(i.value, "echo hi");
    assert!(outcome.dropped_lines);

    // A trailing newline alone drops nothing meaningful.
    let mut i = input("");
    let outcome = i.insert_paste_first_line("echo hi\n");
    assert_eq!(i.value, "echo hi");
    assert!(!outcome.dropped_lines);
}

#[test]
fn paste_is_capped_on_a_grapheme_boundary() {
    let mut i = input("");
    let huge = FAMILY.repeat(MAX_PASTE_BYTES); // far beyond the cap
    let outcome = i.insert_paste(&huge);
    assert!(outcome.truncated);
    assert!(i.value.len() <= MAX_PASTE_BYTES);
    // No partial cluster survived the cut.
    assert_eq!(i.value.len() % FAMILY.len(), 0);
    assert_eq!(
        i.value.graphemes(true).count(),
        i.value.len() / FAMILY.len()
    );
}

#[test]
fn paste_of_only_control_characters_changes_nothing() {
    let mut i = input("keep");
    let outcome = i.insert_paste("\u{1b}\u{7}");
    assert_eq!(i.value, "keep");
    assert_eq!(outcome.inserted, 0);
}

#[test]
fn boundary_helpers_handle_offsets_inside_a_cluster() {
    // Offset 1 sits inside the `e`+mark cluster: prev/next still resolve to
    // the enclosing cluster edges instead of splitting it.
    assert_eq!(prev_grapheme_boundary(COMBINING, 1), 0);
    assert_eq!(next_grapheme_boundary(COMBINING, 1), COMBINING.len());
    assert_eq!(prev_grapheme_boundary(COMBINING, 0), 0);
    assert_eq!(
        next_grapheme_boundary(COMBINING, COMBINING.len()),
        COMBINING.len()
    );
    // Byte offsets inside a multi-byte scalar clamp back to a char boundary.
    assert_eq!(next_grapheme_boundary("한글", 1), "한".len());
}
