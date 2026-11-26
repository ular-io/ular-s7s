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

/// A single-line text input: the current value, a byte-offset cursor, and whether
/// the whole value is selected.
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
    /// Whole-value selection: the value was filled in by the app rather than
    /// typed, so the next edit replaces all of it and one `Backspace` clears it —
    /// the convention every graphical prefilled field follows. A cursor move
    /// drops the selection and keeps the text, so editing the existing value
    /// costs one key instead of being blocked. See [`TextInput::selected`].
    pub select_all: bool,
}

impl TextInput {
    pub(crate) fn new(value: String) -> Self {
        let cursor = value.len();
        TextInput {
            value,
            cursor,
            select_all: false,
        }
    }

    /// Prefilled value in the whole-value selection state, for a field the user
    /// is as likely to replace as to edit: the New Session folder prefill and the
    /// paths its dropdown writes back. A long absolute path would otherwise have
    /// to be erased one grapheme at a time before a different folder can be typed.
    pub(crate) fn selected(value: String) -> Self {
        let cursor = value.len();
        TextInput {
            value,
            cursor,
            select_all: true,
        }
    }

    /// Consumes a whole-value selection ahead of an edit, emptying the field so
    /// the edit applies to nothing. Returns whether a selection was consumed, so
    /// a deletion can stop there instead of deleting twice.
    fn take_selection(&mut self) -> bool {
        if !self.select_all {
            return false;
        }
        self.select_all = false;
        self.value.clear();
        self.cursor = 0;
        true
    }

    /// Drops a whole-value selection without changing the value, leaving the
    /// cursor at the requested edge of it (the graphical convention: ← collapses
    /// to the start, → to the end). Returns whether a selection was dropped.
    fn collapse_selection(&mut self, to_end: bool) -> bool {
        if !self.select_all {
            return false;
        }
        self.select_all = false;
        self.cursor = if to_end { self.value.len() } else { 0 };
        true
    }

    /// Inserts one typed scalar. The cursor lands directly after the inserted
    /// bytes and is deliberately NOT snapped forward to the next cluster
    /// boundary: snapping would step over a following combining mark and make
    /// that mark unreachable for editing.
    pub(crate) fn insert_char(&mut self, c: char) {
        self.take_selection();
        self.cursor = clamp_char_boundary(&self.value, self.cursor);
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Inserts one bracketed paste as a single edit, line breaks and tabs becoming
    /// spaces (see [`insert_paste_at`]).
    pub(crate) fn insert_paste(&mut self, text: &str) -> PasteOutcome {
        self.take_selection();
        insert_paste_at(&mut self.value, &mut self.cursor, text)
    }

    /// Inserts only the first pasted line, for fields where splicing lines
    /// together would change meaning rather than just formatting — the `!`
    /// terminal command, where a trailing `#` comment would swallow whatever a
    /// space-joined next line contained.
    pub(crate) fn insert_paste_first_line(&mut self, text: &str) -> PasteOutcome {
        self.take_selection();
        insert_paste_into(
            &mut self.value,
            &mut self.cursor,
            text,
            LineBreakPolicy::FirstLine,
        )
    }

    pub(crate) fn backspace(&mut self) {
        if self.take_selection() {
            return;
        }
        self.cursor = clamp_char_boundary(&self.value, self.cursor);
        if self.cursor == 0 {
            return;
        }
        let prev = prev_grapheme_boundary(&self.value, self.cursor);
        self.value.drain(prev..self.cursor);
        self.cursor = prev;
    }

    pub(crate) fn delete(&mut self) {
        if self.take_selection() {
            return;
        }
        self.cursor = clamp_char_boundary(&self.value, self.cursor);
        if self.cursor >= self.value.len() {
            return;
        }
        let next = next_grapheme_boundary(&self.value, self.cursor);
        self.value.drain(self.cursor..next);
    }

    pub(crate) fn move_left(&mut self) {
        if self.collapse_selection(false) {
            return;
        }
        self.cursor = prev_grapheme_boundary(&self.value, self.cursor);
    }

    pub(crate) fn move_right(&mut self) {
        if self.collapse_selection(true) {
            return;
        }
        self.cursor = next_grapheme_boundary(&self.value, self.cursor);
    }

    pub(crate) fn home(&mut self) {
        self.select_all = false;
        self.cursor = 0;
    }

    pub(crate) fn end(&mut self) {
        self.select_all = false;
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
#[path = "input_tests.rs"]
mod tests;
