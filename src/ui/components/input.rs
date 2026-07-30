//! Single-line text input primitive with grapheme-safe cursor movement, editing,
//! and paste insertion. The cursor is a byte offset kept on a `char` boundary so
//! slicing stays cheap; movement and deletion step over whole extended grapheme
//! clusters so a combining sequence or a ZWJ emoji is never split in half.
//!
//! Shared by every editable field: the rename modal, the profile form, the New
//! Session folder input, and the Quick Command window use [`TextInput`], while the
//! session keyword box and the folder-filter query keep their own string and
//! cursor and reach the same editing contract through [`insert_paste_at`] and the
//! boundary helpers.

use unicode_segmentation::UnicodeSegmentation;

/// Upper bound on the bytes one paste may insert into a single-line field.
/// A multi-megabyte paste would otherwise be re-measured for display width on
/// every frame and, for rename, written into agent session metadata.
pub(crate) const MAX_PASTE_BYTES: usize = 4096;

/// How a multi-line paste collapses into a single-line field. Not part of the
/// public surface: callers pick a behavior by choosing an insertion method, so a
/// new field cannot accidentally opt into the wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineBreakPolicy {
    /// Line breaks and tabs become single spaces; all pasted text is kept.
    Space,
    /// Only the first line is kept.
    FirstLine,
}

/// What one paste actually did, so the caller can report a surprise to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PasteOutcome {
    /// Bytes inserted into the field (0 = nothing changed).
    pub inserted: usize,
    /// Text was dropped because it exceeded [`MAX_PASTE_BYTES`].
    pub truncated: bool,
    /// Lines after the first were dropped ([`LineBreakPolicy::FirstLine`]).
    pub dropped_lines: bool,
}

/// A single-line text input: the current value plus a byte-offset cursor.
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
}

impl TextInput {
    pub(crate) fn new(value: String) -> Self {
        let cursor = value.len();
        TextInput { value, cursor }
    }

    /// Inserts one typed scalar. The cursor lands directly after the inserted
    /// bytes and is deliberately NOT snapped forward to the next cluster
    /// boundary: snapping would step over a following combining mark and make
    /// that mark unreachable for editing.
    pub(crate) fn insert_char(&mut self, c: char) {
        self.cursor = clamp_char_boundary(&self.value, self.cursor);
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Inserts one bracketed paste as a single edit, line breaks and tabs becoming
    /// spaces (see [`insert_paste_at`]).
    pub(crate) fn insert_paste(&mut self, text: &str) -> PasteOutcome {
        insert_paste_at(&mut self.value, &mut self.cursor, text)
    }

    /// Inserts only the first pasted line, for fields where splicing lines
    /// together would change meaning rather than just formatting — the `!`
    /// terminal command, where a trailing `#` comment would swallow whatever a
    /// space-joined next line contained.
    pub(crate) fn insert_paste_first_line(&mut self, text: &str) -> PasteOutcome {
        insert_paste_into(
            &mut self.value,
            &mut self.cursor,
            text,
            LineBreakPolicy::FirstLine,
        )
    }

    pub(crate) fn backspace(&mut self) {
        self.cursor = clamp_char_boundary(&self.value, self.cursor);
        if self.cursor == 0 {
            return;
        }
        let prev = prev_grapheme_boundary(&self.value, self.cursor);
        self.value.drain(prev..self.cursor);
        self.cursor = prev;
    }

    pub(crate) fn delete(&mut self) {
        self.cursor = clamp_char_boundary(&self.value, self.cursor);
        if self.cursor >= self.value.len() {
            return;
        }
        let next = next_grapheme_boundary(&self.value, self.cursor);
        self.value.drain(self.cursor..next);
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = prev_grapheme_boundary(&self.value, self.cursor);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = next_grapheme_boundary(&self.value, self.cursor);
    }

    pub(crate) fn home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn end(&mut self) {
        self.cursor = self.value.len();
    }
}

/// Inserts sanitized paste text into `value` at `cursor` as one edit, advancing
/// the cursor to the end of the inserted text. Line breaks and tabs become spaces.
///
/// Used directly by fields that keep their string and cursor outside a
/// [`TextInput`] (the session keyword box, the folder filter query).
pub(crate) fn insert_paste_at(value: &mut String, cursor: &mut usize, text: &str) -> PasteOutcome {
    insert_paste_into(value, cursor, text, LineBreakPolicy::Space)
}

fn insert_paste_into(
    value: &mut String,
    cursor: &mut usize,
    text: &str,
    policy: LineBreakPolicy,
) -> PasteOutcome {
    let sanitized = sanitize_single_line(text, policy);
    if sanitized.text.is_empty() {
        return PasteOutcome {
            inserted: 0,
            truncated: sanitized.truncated,
            dropped_lines: sanitized.dropped_lines,
        };
    }
    *cursor = clamp_char_boundary(value, *cursor);
    value.insert_str(*cursor, &sanitized.text);
    *cursor += sanitized.text.len();
    PasteOutcome {
        inserted: sanitized.text.len(),
        truncated: sanitized.truncated,
        dropped_lines: sanitized.dropped_lines,
    }
}

/// Sanitized paste payload: single-line, control-character free, length capped.
struct Sanitized {
    text: String,
    truncated: bool,
    dropped_lines: bool,
}

/// Flattens pasted text into something a single-line field can hold:
/// CRLF and lone CR normalize to LF, line breaks and tabs become spaces (or the
/// text is cut at the first line break), remaining control characters are
/// discarded, and the result is capped at [`MAX_PASTE_BYTES`] on a grapheme
/// boundary. All other Unicode text is preserved as pasted — no normalization,
/// so a pasted rename title keeps the exact bytes the agent CLI stored.
fn sanitize_single_line(text: &str, policy: LineBreakPolicy) -> Sanitized {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let (body, dropped_lines) = match policy {
        LineBreakPolicy::Space => (normalized.as_str(), false),
        LineBreakPolicy::FirstLine => match normalized.split_once('\n') {
            // Trailing-newline-only pastes drop nothing the user can see.
            Some((first, rest)) => (first, !rest.trim().is_empty()),
            None => (normalized.as_str(), false),
        },
    };

    let mut out = String::with_capacity(body.len());
    for ch in body.chars() {
        match ch {
            '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    let mut truncated = false;
    if out.len() > MAX_PASTE_BYTES {
        let cut = grapheme_boundary_at_or_before(&out, MAX_PASTE_BYTES);
        out.truncate(cut);
        truncated = true;
    }

    Sanitized {
        text: out,
        truncated,
        dropped_lines,
    }
}

/// Byte offsets of every extended grapheme-cluster boundary in `s`, `s.len()` included.
fn boundaries(s: &str) -> impl Iterator<Item = usize> + '_ {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
}

/// Clamps a byte offset into `s` onto a `char` boundary (defensive: cursors are
/// only moved through these helpers, but stored offsets outlive edits).
fn clamp_char_boundary(s: &str, cursor: usize) -> usize {
    let mut idx = cursor.min(s.len());
    while !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Grapheme-cluster boundary immediately before `cursor` (byte offset).
pub(crate) fn prev_grapheme_boundary(s: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(s, cursor);
    boundaries(s)
        .take_while(|&b| b < cursor)
        .last()
        .unwrap_or(0)
}

/// Grapheme-cluster boundary immediately after `cursor` (byte offset).
pub(crate) fn next_grapheme_boundary(s: &str, cursor: usize) -> usize {
    let cursor = clamp_char_boundary(s, cursor);
    boundaries(s).find(|&b| b > cursor).unwrap_or(s.len())
}

/// Greatest grapheme-cluster boundary at or before `limit` (byte offset).
fn grapheme_boundary_at_or_before(s: &str, limit: usize) -> usize {
    boundaries(s)
        .take_while(|&b| b <= limit.min(s.len()))
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `e` + COMBINING ACUTE ACCENT: one grapheme, two scalars, three bytes.
    const COMBINING: &str = "e\u{301}";
    /// Family emoji joined by ZERO WIDTH JOINER: one grapheme, 25 bytes.
    const FAMILY: &str = "👨‍👩‍👧‍👦";

    fn input(value: &str) -> TextInput {
        TextInput::new(value.to_string())
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
}
