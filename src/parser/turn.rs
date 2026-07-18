use serde_json::Value;

/// Extracts readable text from a user turn.
pub fn extract_user_text(v: &Value) -> Option<String> {
    for message in candidate_messages(v) {
        if message.get("type").and_then(Value::as_str) == Some("user_message") {
            if let Some(text) = message.get("message").and_then(Value::as_str) {
                return Some(text.to_string());
            }
        }
        if message.get("role").and_then(Value::as_str) == Some("user") {
            if let Some(text) = extract_text(message.get("content")) {
                return Some(text);
            }
        }
    }
    None
}

/// Normalizes AskUserQuestion/ask_question responses into a list of `question → answer`.
pub fn extract_question_answers(v: &Value) -> Option<String> {
    for root in candidate_roots(v) {
        let result = match root.get("toolUseResult") {
            Some(v) => v,
            None => continue,
        };
        let questions = match result.get("questions").and_then(Value::as_array) {
            Some(v) => v,
            None => continue,
        };
        let answers = match result.get("answers").and_then(Value::as_object) {
            Some(v) => v,
            None => continue,
        };

        let mut lines = Vec::new();
        for q in questions {
            let question = q
                .get("question")
                .or_else(|| q.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let answer = match answers.get(question).and_then(Value::as_str) {
                Some(v) => v,
                None => continue,
            };
            lines.push(format!("· {} → {}", question, answer));
        }

        if !lines.is_empty() {
            return Some(lines.join("\n"));
        }
    }
    None
}

fn candidate_roots(v: &Value) -> Vec<&Value> {
    let mut out = vec![v];
    if let Some(p) = v.get("payload") {
        out.push(p);
    }
    if let Some(m) = v.get("message") {
        out.push(m);
    }
    out
}

fn candidate_messages(v: &Value) -> Vec<&Value> {
    let mut out = vec![v];
    if let Some(p) = v.get("payload") {
        out.push(p);
    }
    if let Some(m) = v.get("message") {
        out.push(m);
    }
    out
}

fn extract_text(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(arr)) => {
            let mut parts = Vec::new();
            for item in arr {
                let ty = item.get("type").and_then(Value::as_str);
                if ty == Some("text") {
                    if let Some(t) = item.get("text").and_then(Value::as_str) {
                        parts.push(t.to_string());
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_codex_user_message_turn() {
        let v: Value = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": "안녕하세요"
            }
        });

        assert_eq!(extract_user_text(&v).as_deref(), Some("안녕하세요"));
    }

    #[test]
    fn extracts_response_item_user_message_turn() {
        let v: Value = serde_json::json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": "안녕하세요"
            }
        });

        assert_eq!(extract_user_text(&v).as_deref(), Some("안녕하세요"));
    }

    #[test]
    fn extracts_question_answers_from_payload() {
        let v: Value = serde_json::json!({
            "type": "response_item",
            "payload": {
                "toolUseResult": {
                    "questions": [{
                        "question": "헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요?"
                    }],
                    "answers": {
                        "헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요?": "적용"
                    }
                }
            }
        });

        assert_eq!(
            extract_question_answers(&v).as_deref(),
            Some("· 헤더를 5줄 → 4줄로 줄이는 변경(render.rs:55, Length(5)→Length(4))을 적용할까요? → 적용")
        );
    }
}
