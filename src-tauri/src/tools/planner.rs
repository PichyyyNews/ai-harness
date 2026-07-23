use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum ToolRoute {
    Answer,
    CallTool { name: String, arguments: Value },
    AskUserChoice {
        question: String,
        options: Vec<String>,
        reason: String,
    },
}

#[derive(Debug, Deserialize)]
struct RawToolRoute {
    action: String,
    #[serde(default)]
    tool: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    question: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    reason: String,
}

/// Ask the local model to make one typed routing decision before answer
/// generation. The host validates the returned schema, then executes the
/// selected capability itself; generated prose is never parsed as a tool call.
pub fn decide(endpoint: &str, conversation: &str) -> ToolRoute {
    let prompt = format!(
        r#"You are the native-tool router for an AI assistant. Decide the next action from the available capabilities and return JSON only.

Available capabilities:
- search_chat_history: {{"query":"..."}}. Use only when the user asks about prior chats or past sessions.
- get_session_details: {{"session_id":"latest"}}. Use only when the user requests a transcript or details of a session.
- list_installed_models: {{}}. Use only when the user asks which local models are installed.
- search_huggingface_models: {{"query":"..."}}. Use only when the user asks to find/download a model.
- get_system_status: {{}}. Use only when the user asks about this machine, engine, GPU, VRAM, or runtime.
- list_workspace_files: {{"subpath":"optional/path"}}. Use only when the user asks to inspect workspace files.
- read_workspace_file: {{"relative_path":"..."}}. Use only when the user asks to read a specific workspace file.
- evaluate_expression: {{"expression":"..."}}. Use only for an explicit mathematical calculation.
- ask_user_choice: use only when a missing user decision is essential before any useful answer or action can be taken.

For current facts and news, live web retrieval is automatically orchestrated by the host: select "answer", not a tool. Do not request a choice merely because a request is broad. Give a broad answer when that is useful; ask only when different user selections are genuinely required to proceed. Never create a plain-text list of choices.

Return exactly one of:
{{"action":"answer"}}
{{"action":"call_tool","tool":"capability_name","arguments":{{...}}}}
{{"action":"ask_user_choice","question":"...","options":["...","..."],"reason":"..."}}

Recent conversation:
{conversation}"#
    );

    request_route(endpoint, &prompt)
}

fn request_route(endpoint: &str, prompt: &str) -> ToolRoute {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
    else {
        return ToolRoute::Answer;
    };
    let response: serde_json::Value = match client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&serde_json::json!({
            "messages": [{"role":"user","content":prompt}],
            "max_tokens": 192,
            "temperature": 0.0,
            "stream": false,
            "response_format": {"type":"json_object"},
            "chat_template_kwargs": {"enable_thinking": false}
        }))
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(|response| response.json())
    {
        Ok(response) => response,
        Err(error) => {
            eprintln!("[tool-router] planner unavailable: {error}");
            return ToolRoute::Answer;
        }
    };

    let Some(content) = response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    else {
        return ToolRoute::Answer;
    };
    parse_route(content).unwrap_or(ToolRoute::Answer)
}

fn parse_route(content: &str) -> Option<ToolRoute> {
    let clean = content
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let raw: RawToolRoute = serde_json::from_str(clean).ok()?;
    match raw.action.as_str() {
        "answer" => Some(ToolRoute::Answer),
        "call_tool" if is_registered_tool(&raw.tool) => {
            Some(ToolRoute::CallTool { name: raw.tool, arguments: raw.arguments })
        }
        "ask_user_choice" if valid_choice(&raw.question, &raw.options) => Some(
            ToolRoute::AskUserChoice {
                question: raw.question.trim().to_string(),
                options: raw
                    .options
                    .into_iter()
                    .map(|option| option.trim().to_string())
                    .collect(),
                reason: raw.reason.trim().to_string(),
            },
        ),
        _ => None,
    }
}

fn is_registered_tool(name: &str) -> bool {
    matches!(
        name,
        "search_chat_history"
            | "get_session_details"
            | "list_installed_models"
            | "search_huggingface_models"
            | "get_system_status"
            | "list_workspace_files"
            | "read_workspace_file"
            | "evaluate_expression"
    )
}

fn valid_choice(question: &str, options: &[String]) -> bool {
    (2..=6).contains(&options.len())
        && !question.trim().is_empty()
        && options.iter().all(|option| !option.trim().is_empty())
        && options
            .iter()
            .map(|option| option.trim().to_lowercase())
            .collect::<std::collections::HashSet<_>>()
            .len()
            == options.len()
}

#[cfg(test)]
mod tests {
    use super::{parse_route, ToolRoute};

    #[test]
    fn parses_a_schema_valid_choice_without_topic_keywords() {
        let route = parse_route(
            r#"{"action":"ask_user_choice","question":"Which target should I use?","options":["Target A","Target B"],"reason":"The target is required."}"#,
        )
        .expect("choice route");
        assert!(matches!(route, ToolRoute::AskUserChoice { .. }));
    }

    #[test]
    fn rejects_unknown_tool_names() {
        assert!(parse_route(r#"{"action":"call_tool","tool":"invent_tool","arguments":{}}"#)
            .is_none());
    }
}
