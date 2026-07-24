pub mod evaluator;
pub mod history;
pub mod intent_router;
pub mod models;
pub mod planner;
pub mod system;
pub mod workspace;
pub mod agent_loop;
pub mod prompt_enhancer;
pub mod catalog;

use tauri::AppHandle;
use serde_json::Value;
use uuid::Uuid;

pub struct ToolExecutionResult {
    pub content: String,
    pub sources: Vec<crate::web_search::WebSource>,
    pub retrieval_trace: Vec<crate::web_search::RetrievalTraceEntry>,
}

pub fn execute_tool(
    app: &AppHandle,
    endpoint: Option<&str>,
    embedding_endpoint: Option<&str>,
    name: &str,
    raw_args: &str,
) -> Result<ToolExecutionResult, String> {
    let clean_name = name.trim();
    
    if clean_name == "search_web" {
        let clean_args = raw_args.trim();
        let parsed_json: Option<Value> = serde_json::from_str(clean_args).ok();
        let query = extract_arg(&parsed_json, clean_args, "query");

        let plan = crate::web_search::QueryPlan {
            original_query: query.clone(),
            sub_questions: vec![crate::web_search::SubQuestion {
                id: Uuid::new_v4(),
                text: query.clone(),
                source_hint: crate::web_search::SourceHint::GeneralWeb,
                depends_on: None,
            }],
            is_compound: false,
        };

        return match crate::web_search::orchestrator::run_adaptive_pipeline(
            app,
            endpoint.unwrap_or_default(),
            embedding_endpoint,
            plan,
            &[],
            8_000,
            |_msg| {},
        ) {
            Some(grounding) => Ok(ToolExecutionResult {
                content: grounding.prompt,
                sources: grounding.sources,
                retrieval_trace: grounding.retrieval_trace,
            }),
            None => Ok(ToolExecutionResult {
                content: format!("Web search for '{query}' completed. No current evidence was retrieved."),
                sources: vec![],
                retrieval_trace: vec![],
            }),
        };
    }

    let content = execute_tool_string(app, endpoint, embedding_endpoint, name, raw_args)?;
    Ok(ToolExecutionResult {
        content,
        sources: vec![],
        retrieval_trace: vec![],
    })
}

fn execute_tool_string(
    app: &AppHandle,
    _endpoint: Option<&str>,
    _embedding_endpoint: Option<&str>,
    name: &str,
    raw_args: &str,
) -> Result<String, String> {
    let clean_name = name.trim();
    let clean_args = raw_args.trim();

    match clean_name {
        "ask_user_choice" => Err("ask_user_choice is owned by the typed interaction controller.".to_string()),
        "search_chat_history" => {
            let query = parse_single_arg(clean_args).unwrap_or(clean_args.to_string());
            history::search_chat_history(app, &query, 5)
        }
        "get_session_details" => {
            let session_id = parse_single_arg(clean_args).unwrap_or(clean_args.to_string());
            history::get_session_details(app, &session_id)
        }
        "list_installed_models" => models::list_installed_models(app),
        "search_huggingface_models" => {
            let query = parse_single_arg(clean_args).unwrap_or(clean_args.to_string());
            models::search_huggingface_models(&query)
        }
        "get_system_status" => system::get_system_status(app),
        "list_workspace_files" => {
            let subpath = parse_single_arg(clean_args);
            workspace::list_workspace_files(app, subpath.as_deref())
        }
        "read_workspace_file" => {
            let relative_path = parse_single_arg(clean_args).unwrap_or(clean_args.to_string());
            workspace::read_workspace_file(app, &relative_path)
        }
        "evaluate_expression" => {
            let expr = parse_single_arg(clean_args).unwrap_or(clean_args.to_string());
            evaluator::evaluate_expression(&expr)
        }
        _ => Err(format!("Unknown tool: {clean_name}")),
    }
}

fn parse_single_arg(args: &str) -> Option<String> {
    let s = args.trim();
    if s.is_empty() {
        return None;
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
        if let Some(str_val) = value.as_str() {
            return Some(str_val.to_string());
        }
        if let Some(obj) = value.as_object() {
            if let Some(first_val) = obj.values().next() {
                if let Some(str_val) = first_val.as_str() {
                    return Some(str_val.to_string());
                }
            }
        }
    }

    let unquoted = s.trim_matches('"').trim_matches('\'').trim();
    if !unquoted.is_empty() {
        Some(unquoted.to_string())
    } else {
        None
    }
}

fn extract_arg(parsed: &Option<Value>, raw_args: &str, key: &str) -> String {
    if let Some(p) = parsed {
        if let Some(val) = p.get(key) {
            if let Some(s) = val.as_str() {
                return s.to_string();
            }
            return val.to_string();
        }
    }
    parse_single_arg(raw_args).unwrap_or(raw_args.to_string())
}

pub fn tools_system_prompt() -> String {
    r#"[System Core Directives - Tool & Interaction Execution]
You are an intelligent autonomous AI assistant capable of multi-step tool execution and native UI interaction.

1. MANDATORY WEB SEARCH & GROUNDING:
For factual, historical, current-events, or analytical deep-dive questions (e.g. historical events, AI concepts, news, weather, prices), prefer calling `search_web` to ground your answer in real sources before writing a long response — especially once the user's specific interest has been narrowed down via clarification. Don't default to writing from memory alone when grounding tools are available and relevant.

2. NATIVE USER CHOICE UI & CLARIFICATION:
- Call `ask_user_clarification` ONLY when the user's request is genuinely too broad or ambiguous to act on usefully.
- NEVER call `ask_user_clarification` for plain greetings ("สวัสดี", "hello", "hi"), acknowledgements, or simple conversational openers — respond to those directly with text and wait for their request.
- When a request IS broad, you may ask MULTIPLE rounds of clarifying questions in sequence — narrow from general topic, to subtopic, to specific angle — before producing a full grounded answer.

3. RESPONSE COMPLETENESS & STITCHING:
Always complete your thoughts and synthesize tool results clearly. Never stop mid-sentence.
"#
    .to_string()
}
