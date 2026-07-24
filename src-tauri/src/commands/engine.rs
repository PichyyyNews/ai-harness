use crate::{
    engine::{
        self, ChatRequest, EngineInfo, EngineSettings, FinishReason,
        GenerationResult, HardwareProfile,
    },
    models, sessions,
    state::EngineState,
};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenEvent {
    token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrimEvent {
    suffix: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InteractionEvent {
    id: String,
    question: String,
    options: Vec<crate::sessions::InteractionOption>,
}

struct GenerationActivityGuard(Arc<std::sync::atomic::AtomicBool>);

impl GenerationActivityGuard {
    fn start(flag: Arc<std::sync::atomic::AtomicBool>) -> Self {
        flag.store(true, Ordering::SeqCst);
        Self(flag)
    }
}

impl Drop for GenerationActivityGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn detect_hardware() -> HardwareProfile {
    engine::hardware::detect()
}

#[tauri::command]
pub fn get_engine_settings(app: AppHandle) -> Result<EngineSettings, String> {
    engine::settings::load(&app)
}

#[tauri::command]
pub fn save_engine_settings(
    app: AppHandle,
    settings: EngineSettings,
) -> Result<EngineSettings, String> {
    engine::settings::save(&app, &settings)
}

#[tauri::command]
pub async fn start_engine(app: AppHandle, model_file: String) -> Result<EngineInfo, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let model_path = models::paths::model_path(&worker_app, &model_file)?;
        if !model_path.is_file() {
            return Err(format!(
                "Model file was not found: {}",
                model_path.display()
            ));
        }
        let settings = engine::settings::load(&worker_app)?;
        let cache_dir = engine::settings::backend_cache_directory(&worker_app)?;
        let state = worker_app.state::<EngineState>();
        state.cancel_generation.store(true, Ordering::SeqCst);
        let mut current = state
            .engine
            .lock()
            .map_err(|_| "Engine state lock was poisoned.".to_string())?;
        *current = None;
        *state
            .embedding_runtime
            .lock()
            .map_err(|_| "Embedding runtime lock was poisoned.".to_string())? = None;
        *state
            .conversation_memory
            .lock()
            .map_err(|_| "Conversation memory lock was poisoned.".to_string())? =
            engine::context_manager::ConversationMemory::default();
        let engine = engine::Engine::start(&model_path, settings.clone(), &cache_dir)?;
        let mut info = engine.info();
        let embedding_runtime = if settings.embedding_enabled {
            let _ = worker_app.emit(
                "engine-status",
                StatusEvent {
                    status: "Preparing multilingual embedding model (the first start downloads it once)"
                        .to_string(),
                },
            );
            match engine::embedding_runtime::EmbeddingRuntime::start(&worker_app, &cache_dir) {
                Ok(runtime) => {
                    let _ = worker_app.emit(
                        "engine-status",
                        StatusEvent {
                            status: "Multilingual embedding classifier is ready".to_string(),
                        },
                    );
                    Some(runtime)
                }
                Err(error) => {
                    info.fallback_reason = Some(format!(
                        "Tier 0 embeddings are unavailable ({error}); language classification will use Tier 1."
                    ));
                    None
                }
            }
        } else {
            None
        };
        let main_endpoint = engine.endpoint().to_string();
        let memory_endpoint = settings
            .memory_agent_endpoint
            .clone()
            .unwrap_or_else(|| main_endpoint.clone());
        let embedding_endpoint = embedding_runtime
            .as_ref()
            .map(|runtime| runtime.endpoint().to_string());
        let memory_agent = settings.memory_agent_enabled.then(|| {
            engine::memory::agent::MemoryAgentHandle::start(
                worker_app.clone(),
                memory_endpoint.clone(),
                embedding_endpoint,
                state.generation_active.clone(),
                memory_endpoint == main_endpoint,
            )
        });
        state
            .memory_injection_enabled
            .store(settings.memory_injection_enabled, Ordering::SeqCst);
        *state
            .memory_agent
            .lock()
            .map_err(|_| "Memory agent state lock was poisoned.".to_string())? =
            memory_agent;
        *state
            .embedding_runtime
            .lock()
            .map_err(|_| "Embedding runtime lock was poisoned.".to_string())? =
            embedding_runtime;
        *current = Some(engine);
        state.cancel_generation.store(false, Ordering::SeqCst);
        Ok(info)
    })
    .await
    .map_err(|error| format!("The local engine worker stopped unexpectedly: {error}"))?
}

#[tauri::command]
pub async fn generate_chat(
    app: AppHandle,
    mut request: ChatRequest,
) -> Result<GenerationResult, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let session_id = request.session_id.clone();
        let interaction_selection = match (
            session_id.as_deref(),
            request.interaction_id.as_deref(),
            request.interaction_option_id.as_deref(),
        ) {
            (Some(session), Some(interaction), Some(option)) => Some(
                sessions::store::resolve_pending_interaction(
                    &worker_app,
                    interaction,
                    option,
                    session,
                )?,
            ),
            (None, Some(_), _) | (_, Some(_), None) => {
                return Err("A choice response must include its session and option identifiers.".to_string())
            }
            _ => None,
        };
        let _is_choice_selection = interaction_selection.is_some();
        let pending_user = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned();
        let state = worker_app.state::<EngineState>();
        let _generation_activity = GenerationActivityGuard::start(state.generation_active.clone());
        let _time_context = {
            let mut authority = state
                .time_authority
                .lock()
                .map_err(|_| "Time authority lock was poisoned.".to_string())?;
            engine::time_manager::resolve(&mut authority)
        };
        state.cancel_generation.store(false, Ordering::SeqCst);
        let mut current = state
            .engine
            .lock()
            .map_err(|_| "Engine state lock was poisoned.".to_string())?;
        let engine = current
            .as_mut()
            .ok_or_else(|| "Start a local engine before sending a message.".to_string())?;
        let mut memory = state
            .conversation_memory
            .lock()
            .map_err(|_| "Conversation memory lock was poisoned.".to_string())?;
        if let Some(session_id) = &session_id {
            let detail = sessions::store::get(&worker_app, session_id)?;
            *memory = engine::context_manager::ConversationMemory::from_summary(
                detail.conversation_memory,
            );
            if let Some((_interaction, option)) = &interaction_selection {
                let should_append = match detail.messages.last() {
                    Some(last) => last.role != "user" || last.content != option.label,
                    None => true,
                };
                if should_append {
                    sessions::store::append_message(
                        &worker_app,
                        session_id,
                        "user",
                        &option.label,
                        None,
                        None,
                        None,
                        None,
                    )?;
                }
            } else if let Some(message) = &pending_user {
                let should_append = match detail.messages.last() {
                    Some(last) => last.role != "user" || last.content != message.content,
                    None => true,
                };
                if should_append {
                    sessions::store::append_message(
                        &worker_app,
                        session_id,
                        "user",
                        &message.content,
                        None,
                        None,
                        None,
                        None,
                    )?;
                }
            }
            // The database is the temporal source of truth. Rehydrate the
            // prompt from its UTC-stamped rows so gap detection cannot depend
            // on a browser clock or on a renderer that has just restarted.
            let persisted = sessions::store::get(&worker_app, session_id)?;
            request.messages = persisted
                .messages
                .into_iter()
                .map(|message| engine::ChatMessage {
                    role: message.role,
                    content: message.content,
                    created_at: Some(message.created_at),
                    ..Default::default()
                })
                .collect();
        }
        // Background Step: Enhance raw prompt into a structured intent statement
        if let Some(message) = &pending_user {
            let mut routing_context = request
                .messages
                .iter()
                .rev()
                .filter(|entry| entry.role == "user" || entry.role == "assistant")
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|entry| format!("{}: {}", entry.role, entry.content))
                .collect::<Vec<_>>()
                .join("\n");
            let memory_summary = memory.summary();
            if !memory_summary.is_empty() {
                routing_context = format!("[Active Memory Context]\n{}\n\n{}", memory_summary, routing_context);
            }
            let enhanced_intent = crate::tools::prompt_enhancer::enhance_prompt(
                engine.endpoint(),
                &message.content,
                &routing_context,
            );
            if !enhanced_intent.trim().is_empty() && enhanced_intent.trim() != message.content.trim() {
                tracing::info!(
                    raw_input = %message.content,
                    enhanced_intent = %enhanced_intent,
                    "Prompt enhancer optimized user intent"
                );
                if let Some(user_idx) = request.messages.iter().rposition(|m| m.role == "user") {
                    request.messages[user_idx].content = enhanced_intent;
                }
            }
        }

        // If user just selected a choice option, inject the resolution directive
        if let Some((interaction, option)) = &interaction_selection {
            let current_user_index = request
                .messages
                .iter()
                .rposition(|item| item.role == "user")
                .unwrap_or(request.messages.len());
            request.messages.insert(
                current_user_index,
                engine::ChatMessage {
                    role: "system".to_string(),
                    content: format!(
                        "[Native interaction resolved]\nOriginal request: {}\nRequired decision: {}\nSelected option: {}\nCRITICAL DIRECTIVE: The user has already selected a specific option (\"{}\") to narrow down the request. DO NOT ask another choice question or call ask_user_clarification again. Call search_web to gather details or synthesize the complete, detailed final answer in Thai immediately.",
                        interaction.request_content, interaction.question, option.label, option.label
                    ),
                    created_at: None,
                    ..Default::default()
                },
            );
        }

        // Inject System Capabilities & Guidelines
        let current_user_index = request
            .messages
            .iter()
            .rposition(|item| item.role == "user")
            .unwrap_or(request.messages.len());
        request.messages.insert(
            current_user_index,
            engine::ChatMessage {
                role: "system".to_string(),
                content: crate::tools::tools_system_prompt(),
                created_at: None,
                ..Default::default()
            },
        );

        // Inject 3-Tier Memory (short-term constraints, mid-term session goals,
        // long-term personalization facts, cross-session RAG).
        // This must be injected ABOVE the tools system prompt so that
        // memory constraints can influence every tool decision the model makes.
        let mut memory_reminder_for_loop = None;
        if state.memory_injection_enabled.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(session_id) = &session_id {
                let user_msg_text = pending_user
                    .as_ref()
                    .map(|m| m.content.as_str())
                    .unwrap_or("");
                let embedding_ep = state
                    .embedding_runtime
                    .lock()
                    .ok()
                    .and_then(|guard| guard.as_ref().map(|rt| rt.endpoint().to_string()));
                let tiered = engine::memory::assemble_tiered_memory_prompts(
                    &worker_app,
                    session_id,
                    user_msg_text,
                    embedding_ep.as_deref(),
                );
                if let Some(primary) = tiered.primary {
                    // Insert memory block just before the tools system prompt
                    // (which was just inserted at current_user_index, so we use same index)
                    let insert_before = request
                        .messages
                        .iter()
                        .rposition(|item| item.role == "user")
                        .unwrap_or(request.messages.len());
                    request.messages.insert(
                        insert_before,
                        engine::ChatMessage {
                            role: "system".to_string(),
                            content: primary,
                            created_at: None,
                            ..Default::default()
                        },
                    );
                }
                let memory_reminder_msg = tiered.reminder.map(|reminder| engine::ChatMessage {
                    role: "system".to_string(),
                    content: reminder,
                    created_at: None,
                    ..Default::default()
                });

                let counts = tiered.layer_counts;
                if counts.active_constraints + counts.mid_term_items + counts.long_term_facts > 0 {
                    let _ = worker_app.emit(
                        "engine-status",
                        StatusEvent {
                            status: format!(
                                "Memory loaded: {} constraint(s), {} session goal(s), {} long-term fact(s)",
                                counts.active_constraints,
                                counts.mid_term_items,
                                counts.long_term_facts
                            ),
                        },
                    );
                }

                memory_reminder_for_loop = memory_reminder_msg;
            }
        }

        // AGENTIC TOOL LOOP: Multi-step reason -> tool call -> tool result -> answer cycle
        let embedding_ep_for_loop = state
            .embedding_runtime
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|rt| rt.endpoint().to_string()));
        let loop_outcome = crate::tools::agent_loop::run_agentic_loop(
            &worker_app,
            engine.endpoint(),
            embedding_ep_for_loop.as_deref(),
            session_id.as_deref(),
            request.messages.clone(),
            memory_reminder_for_loop,
        )?;

        let result = match loop_outcome {
            crate::tools::agent_loop::AgentLoopOutcome::SuspendedForUserChoice(choice_result) => {
                if let Some(session_id) = &session_id {
                    if !choice_result.content.trim().is_empty() {
                        sessions::store::append_message(
                            &worker_app,
                            session_id,
                            "assistant",
                            &choice_result.content,
                            None,
                            Some(finish_reason_name(&choice_result.finish_reason)),
                            Some(&choice_result.sources),
                            Some(&choice_result.retrieval_trace),
                        )?;
                        sessions::store::set_memory(&worker_app, session_id, memory.summary())?;
                    }
                }
                return Ok(choice_result);
            }
            crate::tools::agent_loop::AgentLoopOutcome::Completed(comp_result) => comp_result,
        };

        if let Some(session_id) = &session_id {
            if !result.content.trim().is_empty() {
                if let Some(message) = &pending_user {
                    engine::memory::short_term::extract_and_save_direct_memories(
                        &worker_app,
                        session_id,
                        &message.content,
                        &result.content,
                    );
                }
                sessions::store::append_message(
                    &worker_app,
                    session_id,
                    "assistant",
                    &result.content,
                    result.thinking_summary.as_deref(),
                    Some(finish_reason_name(&result.finish_reason)),
                    Some(&result.sources),
                    Some(&result.retrieval_trace),
                )?;
                sessions::store::set_memory(&worker_app, session_id, memory.summary())?;
            }
            engine::memory::short_term::expire_turn_constraints(&worker_app, session_id);

            let turn_index = sessions::store::get(&worker_app, session_id)
                .map(|detail| detail.messages.iter().filter(|message| message.role == "assistant").count())
                .unwrap_or(1);
            let memory_job = engine::memory::agent::MemoryUpdateJob {
                session_id: session_id.clone(),
                user_message: pending_user
                    .as_ref()
                    .map(|message| message.content.clone())
                    .unwrap_or_default(),
                assistant_response: result.content.clone(),
                turn_index,
                classification: None,
            };
            let memory_agent = state
                .memory_agent
                .lock()
                .map_err(|_| "Memory agent state lock was poisoned.".to_string())?
                .clone();
            drop(current);
            drop(memory);
            if let Some(agent) = memory_agent {
                let _ = agent.enqueue(memory_job);
            }
        }
        Ok(result)
    })
    .await
    .map_err(|error| format!("The chat worker stopped unexpectedly: {error}"))?
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
    let mut current = state
        .engine
        .lock()
        .map_err(|_| "Engine state lock was poisoned.".to_string())?;
    *current = None;
    *state
        .memory_agent
        .lock()
        .map_err(|_| "Memory agent state lock was poisoned.".to_string())? = None;
    *state
        .embedding_runtime
        .lock()
        .map_err(|_| "Embedding runtime lock was poisoned.".to_string())? = None;
    Ok(())
}

#[tauri::command]
pub async fn trigger_session_end_memory(app: AppHandle, session_id: String) -> Result<(), String> {
    let state = app.state::<EngineState>();
    let memory_agent = state
        .memory_agent
        .lock()
        .map_err(|_| "Memory agent state lock was poisoned.".to_string())?
        .clone();
    if let Some(agent) = memory_agent {
        agent.enqueue_session_end(session_id)?;
    }
    Ok(())
}
