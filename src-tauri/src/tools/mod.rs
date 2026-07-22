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
    r#"[Harness Native Tools Available]
You can call local tools using the exact syntax <<TOOL: tool_name("argument")>> in your reply:
- ask_user_choice({"question": "Title", "options": ["Opt 1", "Opt 2", "Opt 3"]}): Display an interactive choice UI card above composer input for user selection
- search_chat_history("query"): Search past conversations and chat sessions across all user history
- get_session_details("session_id" or "latest"): Get full message transcript of a past session
- list_installed_models(): List local GGUF models downloaded on machine
- search_huggingface_models("query"): Search open GGUF models on Hugging Face catalog
- get_system_status(): Check system VRAM, free memory, GPU device, and active backend
- list_workspace_files("subpath"): List files in workspace
- read_workspace_file("relative_path"): Read text/code file from workspace
- evaluate_expression("expression"): Evaluate math calculations with 100% precision

Example usage:
If user asks for choices or open-ended directions: <<TOOL: ask_user_choice({"question": "คุณต้องการให้ดำเนินการไปในแนวทางใด?", "options": ["แนวทาง A: ค้นหาข้อมูลเพิ่ม", "แนวทาง B: สร้างไฟล์ตามโครงร่าง", "แนวทาง C: ปรับแต่งรายละเอียด"]})>>
"#
    .to_string()
}
