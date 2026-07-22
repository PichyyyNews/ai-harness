use crate::{language_classifier::MessageClassification, sessions::store};
use serde::Deserialize;
use tauri::AppHandle;

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractedConstraint {
    pub text: String,
    pub scope: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractedGoal {
    pub description: String,
    pub status: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractedDecision {
    pub what: String,
    pub why: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractedPlanStep {
    pub description: String,
    pub status: String,
}

#[derive(Debug, Deserialize, Default)]
struct MidTermScanResult {
    #[serde(default)]
    goals: Vec<ExtractedGoal>,
    #[serde(default)]
    decisions: Vec<ExtractedDecision>,
    #[serde(default)]
    plan_steps: Vec<ExtractedPlanStep>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractedFact {
    pub category: String,
    pub content: String,
    pub confidence: f32,
}

#[derive(Debug, Deserialize, Default)]
pub struct SessionEndMemoryResult {
    #[serde(default)]
    pub facts: Vec<ExtractedFact>,
    #[serde(default)]
    pub session_summary: String,
}

/// Helper function to perform silent background model inference.
/// `endpoint` must be passed in from the caller (e.g. read before spawning the thread)
/// so this function never needs to acquire the engine Mutex.
fn run_silent_generation(endpoint: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| format!("Could not create memory client: {error}"))?;
    let request = |messages: serde_json::Value| -> Result<String, String> {
        let response = client
            .post(format!("{}/v1/chat/completions", endpoint))
            .json(&serde_json::json!({
                "messages": messages,
                "max_tokens": max_tokens,
                "temperature": 0.1,
                "stream": false,
                "response_format": {"type":"json_object"},
                "chat_template_kwargs": {"enable_thinking": false}
            }))
            .send()
            .map_err(|error| {
                format!("Could not send background memory request to llama-server: {error}")
            })?;
        if !response.status().is_success() {
            return Err(format!(
                "llama-server memory extraction HTTP {}",
                response.status()
            ));
        }
        let value: serde_json::Value = response
            .json()
            .map_err(|error| format!("Could not parse memory extraction response: {error}"))?;
        Ok(value
            .pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string())
    };
    let first = request(serde_json::json!([
        {"role":"system","content":"You are a background memory extraction agent. Return valid JSON only."},
        {"role":"user","content":prompt}
    ]));
    if let Ok(content) = &first {
        if !content.is_empty() {
            return Ok(content.clone());
        }
    }
    let second = request(serde_json::json!([
        {"role":"user","content":format!("Return valid JSON only. {prompt}")}
    ]));
    match (first, second) {
        (_, Ok(content)) if !content.is_empty() => Ok(content),
        (Err(first_error), Err(second_error)) => Err(format!(
            "Memory extraction failed twice: {first_error}; retry: {second_error}"
        )),
        (_, Ok(_)) => Err("The memory model returned an empty response twice.".to_string()),
        (Ok(_), Err(error)) => Err(error),
    }
}

pub fn run_constraint_scan(
    app: &AppHandle,
    endpoint: &str,
    session_id: &str,
    user_message: &str,
    classification: Option<&MessageClassification>,
) -> Result<usize, String> {
    let classification = classification
        .cloned()
        .or_else(|| crate::language_classifier::classify(endpoint, user_message));
    // A turn-only instruction has already been consumed by the main response
    // when this asynchronous scan runs; persisting it would incorrectly apply
    // it to the next turn. Session-scoped constraints are the durable output.
    let constraints = classification
        .and_then(|result| result.session_constraint(user_message))
        .map(|(text, scope)| ExtractedConstraint { text, scope })
        .into_iter()
        .collect::<Vec<_>>();
    let count = constraints.len();
    if count > 0 {
        super::short_term::save_extracted_constraints(app, session_id, &constraints);
    }
    super::observability::log_extraction_counts(app, session_id, count, 0, 0, 0);
    Ok(count)
}

pub fn run_mid_term_scan(
    app: &AppHandle,
    endpoint: &str,
    session_id: &str,
    user_message: &str,
    assistant_response: &str,
) -> Result<usize, String> {
    let prompt = format!(
        "Extract structured ongoing memory from this completed turn. JSON schema: {{\"goals\":[{{\"description\":\"...\",\"status\":\"active|achieved|abandoned\"}}],\"decisions\":[{{\"what\":\"...\",\"why\":null}}],\"plan_steps\":[{{\"description\":\"...\",\"status\":\"pending|in_progress|done\"}}]}}. User: {}\nAssistant: {}",
        user_message,
        assistant_response.chars().take(4_000).collect::<String>()
    );
    let raw = run_silent_generation(endpoint, &prompt, 384)?;
    let parsed: MidTermScanResult = serde_json::from_str(&clean_json_str(&raw))
        .map_err(|error| format!("Could not parse mid-term scan JSON: {error}"))?;
    let count = parsed.goals.len() + parsed.decisions.len() + parsed.plan_steps.len();
    if count > 0 {
        super::mid_term::merge_extracted_memory(
            app,
            session_id,
            &parsed.goals,
            &parsed.decisions,
            &parsed.plan_steps,
        );
    }
    super::observability::log_extraction_counts(app, session_id, 0, count, 0, 0);
    Ok(count)
}

/// Runs when a session ends or app closes to extract durable long-term facts & compact session summary.
/// `endpoint` is the llama-server URL, passed in from the caller before spawning the thread
/// so this function never needs to acquire the engine Mutex.
pub fn run_session_end_extraction(
    app: &AppHandle,
    endpoint: &str,
    embedding_endpoint: Option<&str>,
    session_id: &str,
) -> Result<(), String> {
    let detail = match store::get(app, session_id) {
        Ok(d) => d,
        Err(e) => return Err(format!("Could not load session for memory extraction: {e}")),
    };

    if detail.messages.is_empty() {
        return Ok(());
    }

    let transcript: String = detail
        .messages
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Analyze this chat transcript and extract durable facts about the user (preferences, communication style, ongoing named projects, skill level, recurring topics) that remain relevant in 3+ months.\nExplicitly exclude sensitive info like health, politics, religion, or credentials.\n\nReturn ONLY valid JSON matching this schema:\n{{\n  \"facts\": [\n    {{\"category\": \"preference|communication_style|recurring_project|recurring_topic|skill_level\", \"content\": \"durable fact statement\", \"confidence\": 0.9}}\n  ],\n  \"session_summary\": \"Concise 2-3 sentence summary of what this chat session accomplished.\"\n}}\n\nTranscript:\n{}",
        transcript
    );

    let raw_json = run_silent_generation(endpoint, &prompt, 640).map_err(|error| {
        super::observability::log_error(app, session_id, "session_end_generation", &error);
        error
    })?;

    let cleaned = clean_json_str(&raw_json);
    if cleaned.trim().is_empty() {
        let error = "The session-end memory model returned empty JSON.".to_string();
        super::observability::log_error(app, session_id, "session_end_parse", &error);
        return Err(error);
    }

    let parsed: SessionEndMemoryResult = match serde_json::from_str(&cleaned) {
        Ok(res) => res,
        Err(err) => {
            let error = format!("Could not parse session-end memory JSON: {err}");
            super::observability::log_error(app, session_id, "session_end_parse", &error);
            return Err(error);
        }
    };

    eprintln!(
        "[memory-worker] session-end: extracted {} facts, summary length: {}",
        parsed.facts.len(),
        parsed.session_summary.len()
    );

    let stored = if parsed.facts.is_empty() {
        0
    } else {
        super::long_term::process_extracted_facts(
            app,
            embedding_endpoint,
            session_id,
            &parsed.facts,
        )
    };
    super::observability::log_extraction_counts(app, session_id, 0, 0, parsed.facts.len(), stored);

    if !parsed.session_summary.trim().is_empty() {
        let _ = store::save_session_summary(app, session_id, &parsed.session_summary);
        eprintln!("[memory-worker] saved session summary for {session_id}");
    }

    Ok(())
}

fn clean_json_str(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(stripped) = trimmed.strip_prefix("```json") {
        if let Some(end) = stripped.rfind("```") {
            return stripped[..end].trim().to_string();
        }
    } else if let Some(stripped) = trimmed.strip_prefix("```") {
        if let Some(end) = stripped.rfind("```") {
            return stripped[..end].trim().to_string();
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start <= end {
            return trimmed[start..=end].to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_markdown_json_wrappers() {
        let raw = "```json\n{\n  \"constraints\": []\n}\n```";
        let cleaned = clean_json_str(raw);
        assert_eq!(cleaned, "{\n  \"constraints\": []\n}");
    }

    #[test]
    fn parses_after_turn_json_schema() {
        let json = r#"{
            "constraints": [{"text": "Always reply in Thai", "scope": "session"}],
            "goals": [{"description": "Build Tiered Memory", "status": "active"}],
            "decisions": [{"what": "Use SQLite", "why": "Fast and local"}],
            "plan_steps": [{"description": "Implement worker", "status": "done"}]
        }"#;

        let value: serde_json::Value = serde_json::from_str(json).expect("valid JSON parse");
        let constraints: Vec<ExtractedConstraint> =
            serde_json::from_value(value["constraints"].clone()).expect("constraint schema");
        let mid_term: MidTermScanResult = serde_json::from_value(serde_json::json!({
            "goals": value["goals"],
            "decisions": value["decisions"],
            "plan_steps": value["plan_steps"]
        }))
        .expect("mid-term schema");
        assert_eq!(constraints[0].text, "Always reply in Thai");
        assert_eq!(mid_term.goals.len(), 1);
        assert_eq!(mid_term.decisions.len(), 1);
        assert_eq!(mid_term.plan_steps.len(), 1);
    }

    #[test]
    fn parses_session_end_json_schema() {
        let json = r#"{
            "facts": [{"category": "recurring_project", "content": "Building AI Harness desktop app", "confidence": 0.95}],
            "session_summary": "Implemented background memory worker for tiered memory."
        }"#;

        let parsed: SessionEndMemoryResult = serde_json::from_str(json).expect("valid JSON parse");
        assert_eq!(parsed.facts.len(), 1);
        assert_eq!(parsed.facts[0].content, "Building AI Harness desktop app");
        assert!(parsed.session_summary.contains("background memory worker"));
    }
}
