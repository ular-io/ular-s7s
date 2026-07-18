//! Unicode-safe excerpt generation for compact context output.
//!
//! All limits count Unicode scalar values (`chars()`), never bytes, so Korean,
//! combining characters, and emoji can never be split mid-codepoint. Redaction
//! must be applied BEFORE these functions (truncation cannot be allowed to
//! defeat secret-pattern recognition).

/// Compact user-turn limit: texts at or below this length print in full.
pub const USER_COMPACT_MAX: usize = 1_000;
/// Characters kept from the head of an over-limit user turn.
pub const USER_KEEP_HEAD: usize = 500;
/// Characters kept from the tail of an over-limit user turn.
pub const USER_KEEP_TAIL: usize = 500;
/// Assistant excerpt limit for historical turns.
pub const ASSISTANT_HISTORICAL_MAX: usize = 500;
/// Assistant excerpt limit for the latest active turn.
pub const ASSISTANT_LATEST_MAX: usize = 2_000;

/// Compact user-turn excerpt: full text up to 1,000 chars, otherwise the first
/// 500 and last 500 chars around an explicit omission marker with exact counts.
pub fn compact_user(text: &str) -> String {
    let total = text.chars().count();
    if total <= USER_COMPACT_MAX {
        return text.to_string();
    }
    let head: String = text.chars().take(USER_KEEP_HEAD).collect();
    let tail: String = text.chars().skip(total - USER_KEEP_TAIL).collect();
    let omitted = total - USER_KEEP_HEAD - USER_KEEP_TAIL;
    format!("{head}\n[... {omitted} of {total} characters omitted ...]\n{tail}")
}

/// Assistant excerpt: first `max` chars with an explicit truncation marker.
pub fn assistant(text: &str, max: usize) -> String {
    truncate_marked(text, max)
}

/// First `max` chars with an explicit truncation marker when anything was cut.
pub fn truncate_marked(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}\n[... truncated: showing first {max} of {total} characters ...]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_at_or_below_limit_prints_in_full() {
        for len in [0usize, 1, 999, 1000] {
            let text: String = "가".repeat(len);
            assert_eq!(compact_user(&text), text, "len {len}");
        }
    }

    #[test]
    fn user_text_over_limit_keeps_exact_head_and_tail() {
        // 1,001 chars: head 500 + tail 500, 1 omitted.
        let text: String = ('a'..='z').cycle().take(1001).collect();
        let out = compact_user(&text);
        let head: String = text.chars().take(500).collect();
        let tail: String = text.chars().skip(501).collect();
        assert!(out.starts_with(&head));
        assert!(out.ends_with(&tail));
        assert!(out.contains("[... 1 of 1001 characters omitted ...]"));
    }

    #[test]
    fn unicode_heavy_text_never_splits_scalars() {
        // Korean + combining chars + emoji: counted per scalar, no panic, valid UTF-8.
        let unit = "한글e\u{301}😀";
        let text: String = unit.chars().cycle().take(2400).collect();
        let out = compact_user(&text);
        assert!(out.contains("characters omitted"));
        assert_eq!(String::from_utf8(out.into_bytes()).is_ok(), true);
    }

    #[test]
    fn assistant_limits_and_markers() {
        let text: String = "가나다라".chars().cycle().take(600).collect();
        let hist = assistant(&text, ASSISTANT_HISTORICAL_MAX);
        assert!(hist.contains("[... truncated: showing first 500 of 600 characters ...]"));
        assert_eq!(hist.lines().next().unwrap().chars().count(), 500);

        let long: String = "x".repeat(2500);
        let latest = assistant(&long, ASSISTANT_LATEST_MAX);
        assert!(latest.contains("showing first 2000 of 2500"));

        // At or below limits: unchanged, no marker.
        assert_eq!(assistant("short", ASSISTANT_HISTORICAL_MAX), "short");
    }
}
