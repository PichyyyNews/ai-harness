use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MessageClassification {
    pub needs_search: bool,
    pub is_constraint: bool,
    pub constraint_text: Option<String>,
    pub scope: Option<String>,
}

impl MessageClassification {
    pub fn session_constraint(&self, original_message: &str) -> Option<(String, String)> {
        if !self.is_constraint || self.scope.as_deref() == Some("turn_only") {
            return None;
        }
        let text = self
            .constraint_text
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(original_message)
            .trim()
            .chars()
            .take(1_200)
            .collect::<String>();
        (!text.is_empty()).then_some((text, "session".to_string()))
    }
}

/// Tier 1 language-agnostic classifier. The model receives the raw user text
/// and returns intent, rather than the application matching language-specific
/// acknowledgement, offline, or imperative keyword lists.
pub fn classify(endpoint: &str, message: &str) -> Option<MessageClassification> {
    let prompt = format!(
        "Classify the user's message by meaning in whatever language it uses. Reply with JSON only, exactly this shape: {{\"needs_search\":true|false,\"is_constraint\":true|false,\"constraint_text\":string|null,\"scope\":\"session\"|\"turn_only\"|null}}. needs_search is true only when the answer requires current or externally verified information, such as news, live prices, current facts, a requested web lookup, or citations. needs_search is false for writing, code generation, configuration examples, drafting, explanation, translation, summarization of supplied text, greetings, acknowledgements, and requests whose answer can be created without retrieving current facts. is_constraint is true only for an explicit instruction that should govern assistant behavior. Preserve the constraint's meaning concisely. User message: {}",
        message.chars().take(2_000).collect::<String>()
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let request = |messages: serde_json::Value| -> Option<String> {
        let response: serde_json::Value = client
            .post(format!("{endpoint}/v1/chat/completions"))
            .json(&serde_json::json!({
                "messages": messages,
                "max_tokens": 128,
                "temperature": 0.0,
                "stream": false,
                "response_format": {"type":"json_object"},
                "chat_template_kwargs": {"enable_thinking": false}
            }))
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .ok()?;
        response
            .pointer("/choices/0/message/content")?
            .as_str()
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .map(ToOwned::to_owned)
    };
    let raw = request(serde_json::json!([
        {"role":"system","content":"Return valid JSON only. Do not add Markdown."},
        {"role":"user","content":prompt}
    ]))
    .or_else(|| {
        request(serde_json::json!([
            {"role":"user","content":format!("Return JSON only. {prompt}")}
        ]))
    })?;
    parse(&raw)
}

/// Checks the completed response against the user's active constraints using
/// the same language-capable model rather than hand-written language rules.
pub fn violated_constraint_indexes(
    endpoint: &str,
    response: &str,
    constraints: &[String],
) -> Option<Vec<usize>> {
    if constraints.is_empty() {
        return Some(Vec::new());
    }
    let prompt = format!(
        "Check whether the assistant response violates any active user constraints, regardless of the language used. Reply with JSON only: {{\"violated_indexes\":[integer]}}. Indexes are zero-based and only include constraints that are clearly violated; do not infer a violation when uncertain. Constraints: {}\nAssistant response: {}",
        serde_json::to_string(constraints).ok()?,
        response.chars().take(6_000).collect::<String>(),
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .ok()?;
    let response: serde_json::Value = client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&serde_json::json!({
            "messages": [{"role":"user","content":prompt}],
            "max_tokens": 96,
            "temperature": 0.0,
            "stream": false,
            "response_format": {"type":"json_object"},
            "chat_template_kwargs": {"enable_thinking": false}
        }))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()?;
    let raw = response.pointer("/choices/0/message/content")?.as_str()?;
    parse_violation_indexes(raw, constraints.len())
}

fn parse(raw: &str) -> Option<MessageClassification> {
    let cleaned = clean_json(raw);
    let classification: MessageClassification = serde_json::from_str(cleaned).ok()?;
    match classification.scope.as_deref() {
        None | Some("session") | Some("turn_only") => Some(classification),
        Some(_) => None,
    }
}

fn clean_json(raw: &str) -> &str {
    let cleaned = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
        if start <= end {
            return &cleaned[start..=end];
        }
    }
    cleaned
}

fn parse_violation_indexes(raw: &str, constraint_count: usize) -> Option<Vec<usize>> {
    #[derive(Deserialize)]
    struct Evaluation {
        violated_indexes: Vec<usize>,
    }
    let cleaned = clean_json(raw);
    let mut indexes = serde_json::from_str::<Evaluation>(cleaned)
        .ok()?
        .violated_indexes;
    indexes.retain(|index| *index < constraint_count);
    indexes.sort_unstable();
    indexes.dedup();
    Some(indexes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_classifier_results_independent_of_input_script() {
        for message in ["hello", "สวัสดี", "こんにちは", "hola", "مرحبا"] {
            let classification = parse(
                r#"{"needs_search":false,"is_constraint":false,"constraint_text":null,"scope":null}"#,
            )
            .expect(message);
            assert!(!classification.needs_search, "{message}");
        }
    }

    #[test]
    fn extracts_a_session_constraint_from_structured_output() {
        let classification = parse(
            r#"{"needs_search":false,"is_constraint":true,"constraint_text":"do not use emojis","scope":"session"}"#,
        )
        .expect("classification");
        assert_eq!(
            classification.session_constraint("ไม่ตอบ emoji ได้ไหม"),
            Some(("do not use emojis".to_string(), "session".to_string()))
        );
    }

    #[test]
    fn rejects_out_of_range_constraint_evaluation_indexes() {
        assert_eq!(
            parse_violation_indexes(r#"{"violated_indexes":[1,1,9]}"#, 2),
            Some(vec![1])
        );
    }
}
