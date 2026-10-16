//! Match snippet: the best-matching line of the matched text plus one line of
//! context on each side, sanitized for the terminal and cut to a window around
//! the first highlight.

use super::query::{fold, highlight_ranges, Query};
use unicode_normalization::UnicodeNormalization;

/// Characters kept per snippet line; a longer line is cut around its match.
const LINE_CHARS: usize = 160;
/// Characters kept before the first highlight when a line is cut.
const LEAD_CHARS: usize = 40;
const ELLIPSIS: &str = "…";

/// One snippet line with display byte ranges to highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetLine {
    pub text: String,
    pub highlights: Vec<(usize, usize)>,
}

/// Builds the snippet for `text`, the matched field. The centre line is the
/// first line holding the most distinct query tokens; blank neighbours are
/// left out rather than shown as empty rows.
pub(crate) fn build(text: &str, query: &Query) -> Vec<SnippetLine> {
    let lines: Vec<&str> = text.lines().collect();
    let all = query.all().count_ones();
    let mut best = (0, 0);
    for (i, line) in lines.iter().enumerate() {
        let found = query.found_in(&fold(line)).count_ones();
        if found > best.1 {
            best = (i, found);
            if found == all {
                break;
            }
        }
    }
    let centre = best.0;
    let mut out = Vec::new();
    let before = centre.checked_sub(1).and_then(|i| lines.get(i));
    if let Some(line) = before.filter(|l| !l.trim().is_empty()) {
        out.push(window(line, query, false));
    }
    if let Some(line) = lines.get(centre) {
        out.push(window(line, query, true));
    }
    if let Some(line) = lines.get(centre + 1).filter(|l| !l.trim().is_empty()) {
        out.push(window(line, query, false));
    }
    out
}

/// Sanitizes `line` and keeps at most `LINE_CHARS` characters of it: around
/// the first highlight for the centre line, from the start otherwise.
fn window(line: &str, query: &Query, centre: bool) -> SnippetLine {
    let display = sanitize(&line.nfc().collect::<String>());
    let highlights = highlight_ranges(&display, query);
    let bounds: Vec<usize> = display
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(display.len()))
        .collect();
    let chars = bounds.len() - 1;
    if chars <= LINE_CHARS {
        return SnippetLine {
            text: display,
            highlights,
        };
    }
    let first = highlights
        .first()
        .filter(|_| centre)
        .map_or(0, |&(start, _)| bounds.partition_point(|&b| b < start));
    let start_char = first.saturating_sub(LEAD_CHARS).min(chars - LINE_CHARS);
    let (start, end) = (bounds[start_char], bounds[start_char + LINE_CHARS]);
    let lead = if start > 0 { ELLIPSIS } else { "" };
    let trail = if end < display.len() { ELLIPSIS } else { "" };
    let shift = |at: usize| at - start + lead.len();
    let highlights = highlights
        .into_iter()
        .filter(|&(s, e)| e > start && s < end)
        .map(|(s, e)| (shift(s.max(start)), shift(e.min(end))))
        .collect();
    SnippetLine {
        text: format!("{lead}{}{trail}", &display[start..end]),
        highlights,
    }
}

/// Removes ANSI CSI/OSC escape sequences (common in captured command output)
/// and turns any other control character into a space, so the snippet cannot
/// drive the terminal.
fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(if c.is_control() { ' ' } else { c });
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // Parameters and intermediates, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // Terminated by BEL or ST (ESC \).
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => out.push(' '),
        }
    }
    out
}
