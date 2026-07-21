use crate::{engine::{ChatMessage, ChatRequest, FinishReason}, sessions::{self, SessionDetail, SessionSummary}};
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn create_session(app: AppHandle, model_id: Option<String>) -> Result<SessionSummary, String> {
    sessions::store::create(&app, model_id)
}

#[tauri::command]
pub fn list_sessions(app: AppHandle, query: Option<String>) -> Result<Vec<SessionSummary>, String> {
    sessions::store::list(&app, query)
}

#[tauri::command]
pub fn get_session(app: AppHandle, session_id: String) -> Result<SessionDetail, String> {
    sessions::store::get(&app, &session_id)
}

#[tauri::command]
pub fn rename_session(app: AppHandle, session_id: String, title: String) -> Result<SessionSummary, String> {
    sessions::store::rename(&app, &session_id, &title)
}

#[tauri::command]
pub fn delete_session(app: AppHandle, session_id: String) -> Result<(), String> {
    sessions::store::delete(&app, &session_id)
}

#[tauri::command]
pub async fn generate_session_title(app: AppHandle, session_id: String) -> Result<SessionSummary, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let detail = sessions::store::get(&worker_app, &session_id)?;
        if !detail.messages.iter().any(|message| message.role == "assistant") {
            return Ok(detail.session);
        }
        let state = worker_app.state::<crate::state::EngineState>();
        let time_context = {
            let mut authority = state.time_authority.lock().map_err(|_| "Time authority lock was poisoned.".to_string())?;
            crate::engine::time_manager::resolve(&mut authority)
        };
        let mut current = state.engine.lock().map_err(|_| "Engine state lock was poisoned.".to_string())?;
        let engine = current.as_mut().ok_or_else(|| "Start the local engine before generating a session title.".to_string())?;
        let source = detail.messages.iter().take(4).map(|message| format!("{}: {}", message.role, message.content)).collect::<Vec<_>>().join("\n");
        let request = ChatRequest {
            messages: vec![
                crate::engine::time_manager::system_message(&time_context),
                ChatMessage { role: "system".to_string(), content: "Return only a concise 4 to 8 word title for this conversation. Do not add quotes or punctuation.".to_string(), created_at: None },
                ChatMessage { role: "user".to_string(), content: source, created_at: None },
            ],
            max_tokens: Some(24),
            temperature: Some(0.3),
            session_id: None,
        };
        let result = engine.generate(request, |_| Ok(()), || false)?;
        if result.finish_reason == FinishReason::Stop && !result.content.trim().is_empty() {
            sessions::store::set_title(&worker_app, &session_id, result.content.lines().next().unwrap_or_default().trim())
        } else {
            Ok(detail.session)
        }
    }).await.map_err(|error| format!("The session title worker stopped unexpectedly: {error}"))?
}
