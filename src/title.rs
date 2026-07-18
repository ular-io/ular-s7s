//! Session title selection logic.

use crate::model::one_line;

/// Resolves the title candidate into the final display string.
///
/// Priority:
/// - Explicit names where `fixed == true` are used directly
/// - Otherwise, if the candidate is a "bad auto title", it falls back to the first user question
/// - Otherwise, the candidate is used
pub fn resolve(user_turns: &[String], hint: Option<&str>, fixed: bool) -> String {
    let first = user_turns
        .iter()
        .find(|t| !t.trim().is_empty())
        .map(|s| s.as_str());
    let last = user_turns
        .iter()
        .rfind(|t| !t.trim().is_empty())
        .map(|s| s.as_str());

    if let Some(hint) = hint.map(clean) {
        if fixed || !is_bad_auto_title(&hint, first, last) {
            return hint;
        }
    }

    first.map(clean).unwrap_or_else(|| "(empty)".to_string())
}

fn clean(s: &str) -> String {
    one_line(s)
}

fn is_bad_auto_title(title: &str, first: Option<&str>, last: Option<&str>) -> bool {
    if title.is_empty() {
        return true;
    }

    let lowered = title.trim().to_ascii_lowercase();
    if is_command_title(&lowered) {
        return true;
    }

    if title.len() < 3 {
        return true;
    }

    if title.is_ascii() && title.chars().any(|c| c.is_ascii_alphabetic()) {
        return true;
    }

    if let Some(last) = last {
        let last = clean(last);
        if !last.is_empty() && last == title {
            return true;
        }
    }

    if let Some(first) = first {
        let first = clean(first);
        if !first.is_empty() && first == title {
            return false;
        }
    }

    false
}

fn is_command_title(title: &str) -> bool {
    matches!(
        title,
        "/exit" | "exit" | "quit" | "\\q" | "\\quit" | ":q" | ":wq" | "q" | "/clear" | "/status"
    ) || title.starts_with('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_fixed_title() {
        let turns = vec!["첫 질문".to_string()];
        assert_eq!(
            resolve(&turns, Some("Debug Claude CLI keybinding issue"), true),
            "Debug Claude CLI keybinding issue"
        );
    }

    #[test]
    fn falls_back_from_ascii_auto_title() {
        let turns = vec!["첫 질문".to_string(), "마지막 질문".to_string()];
        assert_eq!(
            resolve(&turns, Some("Debug Claude CLI keybinding issue"), false),
            "첫 질문"
        );
    }

    #[test]
    fn falls_back_from_last_user_command() {
        let turns = vec!["첫 질문".to_string(), "/exit".to_string()];
        assert_eq!(resolve(&turns, Some("/exit"), false), "첫 질문");
    }

    #[test]
    fn keeps_good_auto_title() {
        let turns = vec!["첫 질문".to_string()];
        assert_eq!(
            resolve(
                &turns,
                Some("GPS 센서 데이터 결합하여 정차 판정 개선"),
                false
            ),
            "GPS 센서 데이터 결합하여 정차 판정 개선"
        );
    }
}
