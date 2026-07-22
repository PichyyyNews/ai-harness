pub mod evaluator;
pub mod history;
pub mod intent_router;
pub mod models;
pub mod system;
pub mod workspace;

use tauri::AppHandle;

pub fn execute_tool(app: &AppHandle, name: &str, raw_args: &str) -> Result<String, String> {
    let clean_name = name.trim();
    let clean_args = raw_args.trim();

    match clean_name {
        "ask_user_choice" => {
            #[derive(serde::Deserialize, serde::Serialize)]
            struct ChoicePayload {
                question: String,
                options: Vec<String>,
            }

            let payload: ChoicePayload = serde_json::from_str(clean_args)
                .unwrap_or_else(|_| ChoicePayload {
                    question: parse_single_arg(clean_args).unwrap_or_else(|| "Please select an option to proceed:".to_string()),
                    options: vec![
                        "Option 1: Proceed with automatic plan".to_string(),
                        "Option 2: Perform deep web research".to_string(),
                        "Option 3: Custom adjustments".to_string(),
                    ],
                });

            use tauri::Emitter;
            let _ = app.emit("ai-choice-request", &payload);
            Ok(format!(
                "Interactive Choice UI displayed to user with question: \"{}\" and {} options. Awaiting user choice.",
                payload.question,
                payload.options.len()
            ))
        }
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

pub fn tools_system_prompt() -> String {
    r#"[CRITICAL SYSTEM RULE: Harness Native Tools]
You have access to powerful local system tools. You MUST invoke them using <<TOOL: tool_name(...)>> when appropriate:

1. Interactive Choice Tool (MANDATORY WHEN ASKING QUESTIONS OR OFFERING OPTIONS):
   - ask_user_choice({"question": "Title", "options": ["Opt 1", "Opt 2", "Opt 3"]})
   CRITICAL REQUIREMENT: Whenever you need to ask the user to clarify, narrow scope, pick a category, or select from multiple options (1, 2, 3, 4), YOU MUST CALL THIS TOOL IMMEDIATELY in your response. DO NOT write plain text lists asking questions without invoking this tool.

2. History & Memory Tools:
   - search_chat_history("query"): Search past conversations and chat sessions across all user history
   - get_session_details("session_id" or "latest"): Get full message transcript of a past session

3. Hardware & Workspace Tools:
   - get_system_status(): Check system VRAM, free memory, GPU device, and active backend
   - list_workspace_files("subpath"): List files in workspace
   - read_workspace_file("relative_path"): Read text/code file from workspace
   - evaluate_expression("expression"): Evaluate math calculations with 100% precision

EXACT SYNTAX EXAMPLES:
- If asking user to pick a topic or narrow scope:
<<TOOL: ask_user_choice({"question": "โปรดเลือกขอบเขตข้อมูลที่สนใจ:", "options": ["1. เน้นตามประเภทโมเดล (LLMs / Image / Video)", "2. เน้นตามค่ายผู้พัฒนา (OpenAI / Google / Anthropic)", "3. เน้นตามฟีเจอร์เด่น (Context Window / Multimodal)", "4. เน้นข่าวอัปเดตล่าสุด 1-2 เดือนนี้"]})>>
"#
    .to_string()
}
