pub mod evaluator;
pub mod history;
pub mod intent_router;
pub mod models;
pub mod planner;
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
                #[serde(default)]
                reason: String,
            }

            let payload: ChoicePayload = serde_json::from_str(clean_args)
                .unwrap_or_else(|_| ChoicePayload {
                    question: parse_single_arg(clean_args).unwrap_or_else(|| "Please select an option to proceed:".to_string()),
                    options: vec![
                        "Option 1: Proceed with automatic plan".to_string(),
                        "Option 2: Perform deep web research".to_string(),
                        "Option 3: Custom adjustments".to_string(),
                    ],
                    reason: String::new(),
                });

            use tauri::Emitter;
            let _ = app.emit("ai-choice-request", &payload);
            eprintln!("[tool-router] requesting user input: {}", payload.question);
            Ok(format!(
                "[choice-pending] Interactive Choice UI displayed to user with question: \"{}\" and {} options. Required input: {}. Awaiting user choice.",
                payload.question,
                payload.options.len(),
                payload.reason,
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
    r#"[Harness Native Tools]
The host routes and executes native tools before this answer is generated. Use any supplied tool result as authoritative context. Do not print tool markers, tool syntax, or a fake interactive-choice list. If the host needs user input, it will display the native Choice UI and resume with the user's selection.
"#
    .to_string()
}
