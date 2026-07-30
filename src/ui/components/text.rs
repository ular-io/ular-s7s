//! Width-aware string helpers. Terminal cells are one or two columns wide, so
//! layout must measure display width (via `unicode-width`) rather than `char`
//! or byte counts; `format!` width specifiers count chars and misalign CJK and
//! other double-width text.
//!
//! Splitting and measuring both work on extended grapheme clusters. Per-char
//! measurement not only risks cutting a combining sequence or a ZWJ emoji in
//! half, it also over-counts width: `unicode-width` reports the terminal width
//! of a full ZWJ sequence (2) only when the whole cluster is measured at once,
//! while summing its parts yields 8 for a family emoji.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Truncates a string to `max_w` display columns, appending `…` when clipped.
pub(crate) fn truncate_w(s: &str, max_w: usize) -> String {
    truncate_w_with_ellipsis(s, max_w, "…")
}

/// Right-pads a string with spaces to `w` display columns (no-op if already wider).
pub(crate) fn pad_w(s: &str, w: usize) -> String {
    let cur = s.width();
    if cur >= w {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(w - cur))
    }
}

/// Truncates to `max_w` display columns using a custom ellipsis marker.
pub(crate) fn truncate_w_with_ellipsis(s: &str, max_w: usize, ellipsis: &str) -> String {
    if s.width() <= max_w {
        return s.to_string();
    }
    if max_w == 0 {
        return String::new();
    }

    let ellipsis_w = ellipsis.width();
    if max_w <= ellipsis_w {
        // Not even the marker fits: emit as much of it as the width allows.
        return ellipsis
            .graphemes(true)
            .scan(0usize, |w, g| {
                let gw = g.width();
                if *w + gw > max_w {
                    return None;
                }
                *w += gw;
                Some(g)
            })
            .collect();
    }

    let mut out = String::new();
    let mut w = 0usize;
    for g in s.graphemes(true) {
        let gw = g.width();
        if w + gw + ellipsis_w > max_w {
            out.push_str(ellipsis);
            break;
        }
        out.push_str(g);
        w += gw;
    }
    out
}

/// Wraps text to a `max_w` display-column limit (no ellipsis).
pub(crate) fn wrap_w(s: &str, max_w: usize) -> Vec<String> {
    // Tab stops (space expansion width). unicode-width counts `\t` as 0 width, but terminals expand it.
    // This discrepancy causes text to overflow past frames. Expands tabs to spaces before calculation to match widths.
    const TAB_STOP: usize = 4;
    if max_w == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;

    for g in s.graphemes(true) {
        // Tab: fills with space up to next tab stop (wraps if exceeding max width).
        if g == "\t" {
            for _ in 0..(TAB_STOP - (current_w % TAB_STOP)) {
                if current_w + 1 > max_w {
                    lines.push(std::mem::take(&mut current));
                    current_w = 0;
                }
                current.push(' ');
                current_w += 1;
            }
            continue;
        }
        let gw = g.width();
        if current_w > 0 && current_w + gw > max_w {
            lines.push(current);
            current = String::new();
            current_w = 0;
        }
        if gw > max_w {
            // Extremely rare case where a single cluster is wider than the line;
            // it gets its own line rather than being split or dropped (wrapping
            // must never lose content — unlike truncation, which can elide).
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
                current_w = 0;
            }
            lines.push(g.to_string());
            continue;
        }
        current.push_str(g);
        current_w += gw;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMBINING: &str = "e\u{301}";
    const FAMILY: &str = "👨‍👩‍👧‍👦";
    /// Heart + VARIATION SELECTOR-16: one cluster, width 2.
    const HEART: &str = "❤\u{fe0f}";

    #[test]
    fn truncation_keeps_combining_sequences_whole() {
        // "ae\u{301}bc" is 4 columns wide; clipping to 3 keeps `a` + ellipsis.
        let out = truncate_w(&format!("a{COMBINING}bc"), 3);
        assert_eq!(out, "ae\u{301}…");
        // The accent never appears without its base character.
        assert!(!out.contains("\u{301}") || out.contains(COMBINING));
    }

    #[test]
    fn truncation_never_emits_a_partial_zwj_emoji() {
        let s = format!("{FAMILY}{FAMILY}");
        for max_w in 1..=6 {
            let out = truncate_w(&s, max_w);
            assert!(out.width() <= max_w, "width {max_w}: {out:?}");
            assert!(
                !out.contains('\u{200d}') || out.contains(FAMILY),
                "width {max_w} left a dangling joiner: {out:?}"
            );
        }
    }

    #[test]
    fn zwj_and_variation_clusters_measure_as_two_columns() {
        // Per-char measurement would report 8 for the family and 1 for the heart.
        assert_eq!(FAMILY.width(), 2);
        assert_eq!(HEART.width(), 2);
        assert_eq!(truncate_w(FAMILY, 2), FAMILY);
        assert_eq!(truncate_w(&format!("{HEART}{HEART}"), 4).width(), 4);
    }

    #[test]
    fn truncation_still_reserves_ellipsis_width() {
        assert_eq!(truncate_w("abcdef", 4), "abc…");
        assert_eq!(truncate_w("abcdef", 6), "abcdef");
        assert_eq!(truncate_w("abcdef", 1), "…");
        assert_eq!(truncate_w("abcdef", 0), "");
        assert_eq!(truncate_w_with_ellipsis("abcdef", 2, "..").width(), 2);
    }

    #[test]
    fn wrapping_preserves_grapheme_boundaries() {
        let s = format!("{FAMILY}{FAMILY}{FAMILY}");
        let lines = wrap_w(&s, 4);
        assert_eq!(lines, vec![format!("{FAMILY}{FAMILY}"), FAMILY.to_string()]);
        for line in &lines {
            assert!(line.width() <= 4);
        }

        let lines = wrap_w(&format!("a{COMBINING}b"), 2);
        assert_eq!(lines, vec![format!("a{COMBINING}"), "b".to_string()]);
    }

    #[test]
    fn tab_expansion_is_unchanged() {
        assert_eq!(wrap_w("a\tb", 8), vec!["a   b".to_string()]);
        assert_eq!(wrap_w("\tx", 8), vec!["    x".to_string()]);
    }

    #[test]
    fn cluster_wider_than_the_line_gets_its_own_line_without_overflowing_others() {
        // max_w 1 cannot hold a 2-column cluster: it lands alone, and no other
        // line exceeds the limit.
        let lines = wrap_w(&format!("a{FAMILY}b"), 1);
        assert_eq!(
            lines,
            vec!["a".to_string(), FAMILY.to_string(), "b".to_string()]
        );
        assert_eq!(truncate_w(FAMILY, 1), "…");
    }
}
