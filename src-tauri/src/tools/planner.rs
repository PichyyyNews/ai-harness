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
        r#"You are the universal capability and ambiguity router for an AI assistant. Evaluate the conversation context and decide the next action. Return JSON only.

Available capabilities:
- search_chat_history: {{"query":"..."}}. Use when the user asks about prior chats or past sessions.
- get_session_details: {{"session_id":"latest"}}. Use when the user requests a transcript or details of a session.
- list_installed_models: {{}}. Use when the user asks which local models are installed.
- search_huggingface_models: {{"query":"..."}}. Use when the user asks to find/download a model.
- get_system_status: {{}}. Use when the user asks about this machine, engine, GPU, VRAM, or runtime.
- list_workspace_files: {{"subpath":"optional/path"}}. Use when the user asks to inspect workspace files.
- read_workspace_file: {{"relative_path":"..."}}. Use when the user asks to read a specific workspace file.
- evaluate_expression: {{"expression":"..."}}. Use for an explicit mathematical calculation.
- ask_user_choice: {{"question":"...", "options":["...", "..."], "reason":"..."}}. Use to ask a natural clarifying Q&A question with 2 to 4 options when the user's intent needs scoping before executing an answer or search.

INTERACTIVE Q&A SCOPING RULES:
1. CONCISE & BALANCED SCOPING (1 to 3 rounds max):
   - Ask clarifying Q&A choices to scope broad requests down to actionable parameters.
   - Do NOT ask too many questions or over-scope. As soon as the main topic & sub-focus (e.g., Tech -> AI / EV Car) are sufficient to answer or search, STOP asking and select "answer"!

2. SELECT "answer" IF:
   - The parameters are now sufficient to provide a specific answer or perform a search.
   - The user request is a simple greeting, casual conversation, or complete direct question.

3. SELECT "ask_user_choice" IF:
   - The request is broad/underspecified (e.g. "หาข่าวให้หน่อย", "ช่วยเขียนโค้ด", "วางแผนโปรเจกต์").
   - Formulate a clear, natural Q&A question in the user's language with 2 to 4 distinct options.

Examples:

User: "สวัสดีครับ"
Output: {{"action":"answer"}}

User: "หาข่าวให้หน่อย"
Output: {{"action":"ask_user_choice","question":"ต้องการทราบข่าวเกี่ยวกับเรื่องใดครับ?","options":["ข่าวการเมือง / สงครามโลก","ข่าวเทคโนโลยี & นวัตกรรม","ข่าวเศรษฐกิจ & การลงทุน"],"reason":"Round 1 scoping: Main news topic"}}

User: "หาข่าวให้หน่อย\nAssistant: Select an option above...\nUser: ข่าวเทคโนโลยี & นวัตกรรม"
Output: {{"action":"ask_user_choice","question":"สนใจเจาะลึกข่าวเทคโนโลยีเรื่องไหนเป็นพิเศษครับ?","options":["ข่าวปัญญาประดิษฐ์ (AI & LLMs)","ข่าวรถยนต์ไฟฟ้า (EV Cars & Battery)","ข่าวไอที สมาร์ทโฟน & บิ๊กเทค"],"reason":"Round 2 scoping: Sub-topic focus"}}

User: "หาข่าวให้หน่อย\nAssistant: Select an option above...\nUser: ข่าวเทคโนโลยี & นวัตกรรม\nAssistant: Select an option above...\nUser: ข่าวปัญญาประดิษฐ์ (AI & LLMs)"
Output: {{"action":"answer"}}

User: "ช่วยเขียนโค้ดให้หน่อย"
Output: {{"action":"ask_user_choice","question":"ต้องการเขียนโค้ดภาษาหรือสำหรับงานประเภทใดครับ?","options":["JavaScript / TypeScript (Web / React / Node)","Python (Data Analysis / Automation / Backend)","HTML / CSS (Frontend Layout)"],"reason":"Round 1 scoping: Programming language stack"}}

Return JSON only matching one of the schemas above.

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

    let json_str = if let (Some(start), Some(end)) = (clean.find('{'), clean.rfind('}')) {
        &clean[start..=end]
    } else {
        clean
    };

    let raw: RawToolRoute = serde_json::from_str(json_str).ok()?;
    match raw.action.as_str() {
        "answer" => Some(ToolRoute::Answer),
        "call_tool" if raw.tool == "ask_user_choice" => extract_choice(&raw),
        "call_tool" if is_registered_tool(&raw.tool) => {
            Some(ToolRoute::CallTool { name: raw.tool, arguments: raw.arguments })
        }
        "ask_user_choice" => extract_choice(&raw),
        _ => None,
    }
}

fn extract_choice(raw: &RawToolRoute) -> Option<ToolRoute> {
    let question = raw
        .arguments
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or(&raw.question)
        .trim()
        .to_string();

    let options: Vec<String> = if let Some(arr) = raw.arguments.get("options").and_then(Value::as_array) {
        arr.iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim().to_string())
            .collect()
    } else {
        raw.options.iter().map(|s| s.trim().to_string()).collect()
    };

    let reason = raw
        .arguments
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or(&raw.reason)
        .trim()
        .to_string();

    if valid_choice(&question, &options) {
        Some(ToolRoute::AskUserChoice { question, options, reason })
    } else {
        None
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
            | "ask_user_choice"
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
    fn parses_a_choice_called_via_call_tool_action() {
        let route = parse_route(
            r#"{"action":"call_tool","tool":"ask_user_choice","arguments":{"question":"Select framework","options":["React","Vue"],"reason":"Need UI framework"}}"#,
        )
        .expect("call_tool choice route");
        assert!(matches!(route, ToolRoute::AskUserChoice { .. }));
    }

    #[test]
    fn parses_choice_surrounded_by_thinking_text() {
        let raw = r#"Here is my decision: ```json
{"action":"ask_user_choice","question":"เลือกภาษาอะไรดี","options":["Python","TypeScript"],"reason":"Choice required"}
``` Hope this helps!"#;
        let route = parse_route(raw).expect("choice with surrounding text");
        assert!(matches!(route, ToolRoute::AskUserChoice { .. }));
    }

    #[test]
    fn rejects_unknown_tool_names() {
        assert!(parse_route(r#"{"action":"call_tool","tool":"invent_tool","arguments":{}}"#)
            .is_none());
    }
}
