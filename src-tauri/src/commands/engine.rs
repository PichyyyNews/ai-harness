use crate::{engine::{self, ChatRequest, EngineInfo, EngineSettings, FinishReason, GenerationEvent, GenerationResult, HardwareProfile}, models, sessions, state::EngineState};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenEvent { token: String }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrimEvent { suffix: String }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent { status: String }

#[tauri::command]
pub fn detect_hardware() -> HardwareProfile {
    engine::hardware::detect()
}

#[tauri::command]
pub fn get_engine_settings(app: AppHandle) -> Result<EngineSettings, String> {
    engine::settings::load(&app)
}

#[tauri::command]
pub fn save_engine_settings(app: AppHandle, settings: EngineSettings) -> Result<EngineSettings, String> {
    engine::settings::save(&app, &settings)
}

#[tauri::command]
pub async fn start_engine(app: AppHandle, model_file: String) -> Result<EngineInfo, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let model_path = models::paths::model_path(&worker_app, &model_file)?;
        if !model_path.is_file() { return Err(format!("Model file was not found: {}", model_path.display())); }
        let settings = engine::settings::load(&worker_app)?;
        let cache_dir = engine::settings::backend_cache_directory(&worker_app)?;
        let state = worker_app.state::<EngineState>();
        state.cancel_generation.store(true, Ordering::SeqCst);
        let mut current = state.engine.lock().map_err(|_| "Engine state lock was poisoned.".to_string())?;
        *current = None;
        *state.conversation_memory.lock().map_err(|_| "Conversation memory lock was poisoned.".to_string())? = engine::context_manager::ConversationMemory::default();
        let engine = engine::Engine::start(&model_path, settings, &cache_dir)?;
        let info = engine.info();
        *current = Some(engine);
        state.cancel_generation.store(false, Ordering::SeqCst);
        Ok(info)
    }).await.map_err(|error| format!("The local engine worker stopped unexpectedly: {error}"))?
}

#[tauri::command]
pub async fn generate_chat(app: AppHandle, mut request: ChatRequest) -> Result<GenerationResult, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let session_id = request.session_id.clone();
        let pending_user = request.messages.iter().rev().find(|message| message.role == "user").cloned();
        let state = worker_app.state::<EngineState>();
        let time_context = {
            let mut authority = state.time_authority.lock().map_err(|_| "Time authority lock was poisoned.".to_string())?;
            engine::time_manager::resolve(&mut authority)
        };
        state.cancel_generation.store(false, Ordering::SeqCst);
        let mut current = state.engine.lock().map_err(|_| "Engine state lock was poisoned.".to_string())?;
        let engine = current.as_mut().ok_or_else(|| "Start a local engine before sending a message.".to_string())?;
        let mut memory = state.conversation_memory.lock().map_err(|_| "Conversation memory lock was poisoned.".to_string())?;
        if let Some(session_id) = &session_id {
            let detail = sessions::store::get(&worker_app, session_id)?;
            *memory = engine::context_manager::ConversationMemory::from_summary(detail.conversation_memory);
            if let Some(message) = &pending_user {
                sessions::store::append_message(&worker_app, session_id, "user", &message.content, None, None)?;
            }
            // The database is the temporal source of truth. Rehydrate the
            // prompt from its UTC-stamped rows so gap detection cannot depend
            // on a browser clock or on a renderer that has just restarted.
            let persisted = sessions::store::get(&worker_app, session_id)?;
            request.messages = persisted.messages.into_iter().map(|message| engine::ChatMessage {
                role: message.role,
                content: message.content,
                created_at: Some(message.created_at),
            }).collect();
        }
        let result = engine::context_manager::generate_with_recovery(engine, request, &mut memory, &time_context, |event| match event {
            GenerationEvent::Token(token) => worker_app.emit("engine-token", TokenEvent { token }).map_err(|error| format!("Could not stream a generated token: {error}")),
            GenerationEvent::TrimSuffix(suffix) => worker_app.emit("engine-trim", TrimEvent { suffix }).map_err(|error| format!("Could not trim repetitive output: {error}")),
            GenerationEvent::Status(status) => worker_app.emit("engine-status", StatusEvent { status }).map_err(|error| format!("Could not send generation status: {error}")),
        }, || state.cancel_generation.load(Ordering::SeqCst))?;
        if let Some(session_id) = &session_id {
            sessions::store::append_message(&worker_app, session_id, "assistant", &result.content, None, Some(finish_reason_name(&result.finish_reason)))?;
            sessions::store::set_memory(&worker_app, session_id, memory.summary())?;
        }
        Ok(result)
    }).await.map_err(|error| format!("The chat worker stopped unexpectedly: {error}"))?
}

fn finish_reason_name(reason: &FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::RepetitionDetected => "repetition_detected",
        FinishReason::Cancelled => "cancelled",
    }
}

#[tauri::command]
pub fn stop_generation(state: State<'_, EngineState>) {
    state.cancel_generation.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn stop_engine(state: State<'_, EngineState>) -> Result<(), String> {
    state.cancel_generation.store(true, Ordering::SeqCst);
    let mut current = state.engine.lock().map_err(|_| "Engine state lock was poisoned.".to_string())?;
    *current = None;
    Ok(())
}
