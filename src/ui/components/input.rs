//! Single-line text input primitive with Unicode-safe cursor movement and
//! editing. The cursor is a byte offset kept on a `char` boundary so multi-byte
//! (e.g. CJK) input never splits a scalar. Shared by every editable field: the
//! rename modal, the profile form, the New Session folder input, and the Quick
//! Command window.

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

    pub(crate) fn insert_char(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub(crate) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.value, self.cursor);
        self.value.drain(prev..self.cursor);
        self.cursor = prev;
    }

    pub(crate) fn delete(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let next = next_char_boundary(&self.value, self.cursor);
        self.value.drain(self.cursor..next);
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = prev_char_boundary(&self.value, self.cursor);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = next_char_boundary(&self.value, self.cursor);
    }

    pub(crate) fn home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn end(&mut self) {
        self.cursor = self.value.len();
    }
}

/// Previous `char` boundary at or before `cursor` (byte offset).
pub(crate) fn prev_char_boundary(s: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let mut idx = cursor.saturating_sub(1);
    while !s.is_char_boundary(idx) {
        idx = idx.saturating_sub(1);
    }
    idx
}

/// Next `char` boundary after `cursor` (byte offset).
pub(crate) fn next_char_boundary(s: &str, cursor: usize) -> usize {
    if cursor >= s.len() {
        return s.len();
    }
    let mut idx = cursor.saturating_add(1);
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx.min(s.len())
}
