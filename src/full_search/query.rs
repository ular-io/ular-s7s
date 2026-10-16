//! Query tokens and the normalization/case folding shared by content matching,
//! snippet highlighting, and the raw-transcript prefilter.

use unicode_normalization::UnicodeNormalization;

/// Tokens beyond this count are ignored; matched tokens are tracked as bits.
const MAX_TOKENS: usize = 16;

/// A parsed full-search query: whitespace-separated tokens, every one of which
/// must occur somewhere in the session (AND), matching the keyword filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Folded (`fold`) tokens, deduplicated, in input order.
    pub(crate) tokens: Vec<String>,
}

impl Query {
    /// `None` when the text holds no token.
    pub fn parse(text: &str) -> Option<Query> {
        let mut tokens: Vec<String> = Vec::new();
        for token in fold(text).split_whitespace() {
            if tokens.len() < MAX_TOKENS && !tokens.iter().any(|t| t == token) {
                tokens.push(token.to_string());
            }
        }
        (!tokens.is_empty()).then_some(Query { tokens })
    }

    /// Parses a CLI query without silently dropping tokens beyond the bit limit.
    /// The interactive parser retains its existing bounded-query behavior.
    pub fn parse_complete(text: &str) -> Result<Query, String> {
        let query =
            Self::parse(text).ok_or_else(|| "query holds no valid search token.".to_string())?;
        if fold(text)
            .split_whitespace()
            .any(|token| !query.tokens.iter().any(|t| t == token))
        {
            return Err(format!(
                "full search accepts at most {MAX_TOKENS} distinct query tokens."
            ));
        }
        Ok(query)
    }

    /// Bit `i` is set when token `i` occurs in the already folded `haystack`.
    pub(crate) fn found_in(&self, haystack: &str) -> u32 {
        self.tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| haystack.contains(t.as_str()))
            .fold(0, |bits, (i, _)| bits | (1 << i))
    }

    /// The bit set with every token present.
    pub(crate) fn all(&self) -> u32 {
        (1u32 << self.tokens.len()) - 1
    }
}

/// NFC normalization followed by per-character lowercasing. Per character, not
/// `str::to_lowercase`, so `fold_mapped` can map folded offsets back to the
/// displayed text; the two differ only in the context-dependent final sigma.
pub(crate) fn fold(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.nfc() {
        out.extend(c.to_lowercase());
    }
    out
}

/// Folds an already NFC-normalized `display` text, also returning for every
/// folded byte the byte offset of the display character it came from.
pub(crate) fn fold_mapped(display: &str) -> (String, Vec<usize>) {
    let mut folded = String::with_capacity(display.len());
    let mut origin = Vec::with_capacity(display.len());
    for (at, c) in display.char_indices() {
        for lower in c.to_lowercase() {
            folded.push(lower);
            origin.extend(std::iter::repeat_n(at, lower.len_utf8()));
        }
    }
    (folded, origin)
}

/// Display byte ranges of every occurrence of the query tokens in `display`,
/// sorted and merged.
pub(crate) fn highlight_ranges(display: &str, query: &Query) -> Vec<(usize, usize)> {
    let (folded, origin) = fold_mapped(display);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for token in &query.tokens {
        for (start, matched) in folded.match_indices(token.as_str()) {
            let last = origin[start + matched.len() - 1];
            let end = last + display[last..].chars().next().map_or(0, char::len_utf8);
            ranges.push((origin[start], end));
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}
