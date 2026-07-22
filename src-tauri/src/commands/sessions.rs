use crate::sessions::{self, SessionDetail, SessionSummary};
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
pub fn rename_session(
    app: AppHandle,
    session_id: String,
    title: String,
) -> Result<SessionSummary, String> {
    sessions::store::rename(&app, &session_id, &title)
}

#[tauri::command]
pub fn delete_session(app: AppHandle, session_id: String) -> Result<(), String> {
    sessions::store::delete(&app, &session_id)
}

#[tauri::command]
pub async fn generate_session_title(
    app: AppHandle,
    session_id: String,
) -> Result<SessionSummary, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let detail = sessions::store::get(&worker_app, &session_id)?;
        if !detail.messages.iter().any(|message| message.role == "assistant") {
            return Ok(detail.session);
        }

        // Lock engine for < 1us to get endpoint and release lock immediately
        let endpoint = {
            let state = worker_app.state::<crate::state::EngineState>();
            let guard = match state.engine.try_lock() {
                Ok(g) => g,
                Err(_) => return Ok(detail.session),
            };
            let Some(engine) = guard.as_ref() else {
                return Ok(detail.session);
            };
            engine.endpoint().to_string()
        };

        let source = detail
            .messages
            .iter()
            .take(4)
            .map(|message| format!("{}: {}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n");

        let payload = serde_json::json!({
            "messages": [
                {
                    "role": "system",
                    "content": "Return only a concise 4 to 8 word title for this conversation. Do not add quotes or punctuation."
                },
                {
                    "role": "user",
                    "content": source
                }
            ],
            "max_tokens": 24,
            "temperature": 0.3,
            "stream": false
        });

        let response = match reqwest::blocking::Client::new()
            .post(format!("{}/v1/chat/completions", endpoint))
            .json(&payload)
            .send()
        {
            Ok(res) => res,
            Err(_) => return Ok(detail.session),
        };

        if !response.status().is_success() {
            return Ok(detail.session);
        }

        let value: serde_json::Value = match response.json() {
            Ok(v) => v,
            Err(_) => return Ok(detail.session),
        };

        let title = value
            .pointer("/choices/0/message/content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim();

        if !title.is_empty() {
            sessions::store::set_title(&worker_app, &session_id, title.lines().next().unwrap_or_default().trim())
        } else {
            Ok(detail.session)
        }
    })
    .await
    .map_err(|error| format!("The session title worker stopped unexpectedly: {error}"))?
}
