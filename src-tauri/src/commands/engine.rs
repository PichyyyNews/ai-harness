use crate::{
    engine::{
        self, ChatRequest, EngineInfo, EngineSettings, FinishReason, GenerationEvent,
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
        let pending_user = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned();
        let state = worker_app.state::<EngineState>();
        let _generation_activity = GenerationActivityGuard::start(state.generation_active.clone());
        let time_context = {
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
            if let Some(message) = &pending_user {
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
                })
                .collect();
        }
        let tier0_analysis = pending_user.as_ref().and_then(|message| {
            let runtime = state.embedding_runtime.lock().ok()?;
            let runtime = runtime.as_ref()?;
            match runtime.analyze(&message.content) {
                Ok(analysis) => {
                    let provider_log = analysis
                        .provider_candidates
                        .iter()
                        .map(|(provider, score)| (format!("{provider:?}"), *score))
                        .collect::<Vec<_>>();
                    crate::web_search::observability::log_provider_plan(
                        &worker_app,
                        &message.content,
                        &provider_log,
                    );
                    Some(analysis)
                }
                Err(error) => {
                    crate::web_search::observability::log_tier0_error(
                        &worker_app,
                        &message.content,
                        &error,
                    );
                    None
                }
            }
        });
        let embedding_endpoint = state
            .embedding_runtime
            .lock()
            .ok()
            .and_then(|runtime| runtime.as_ref().map(|runtime| runtime.endpoint().to_string()));
        let language_classification = pending_user.as_ref().and_then(|message| {
            if let Some(analysis) = tier0_analysis.as_ref() {
                let result = &analysis.classification;
                let decision = match result.intent {
                    Some(engine::embedding_runtime::Tier0Intent::NoSearch) => "no_search",
                    Some(engine::embedding_runtime::Tier0Intent::SessionConstraint) => "session_constraint",
                    None if engine::embedding_runtime::EmbeddingRuntime::is_ambiguous(&result) => "ambiguous",
                    None => "tier1",
                };
                crate::web_search::observability::log_tier0(
                    &worker_app,
                    &message.content,
                    result.greeting_score,
                    result.constraint_score,
                    decision,
                );
                match result.intent {
                    Some(engine::embedding_runtime::Tier0Intent::NoSearch) => {
                        return Some(crate::language_classifier::MessageClassification {
                            needs_search: false,
                            is_constraint: false,
                            constraint_text: None,
                            scope: None,
                        });
                    }
                    Some(engine::embedding_runtime::Tier0Intent::SessionConstraint) => {
                        return Some(crate::language_classifier::MessageClassification {
                            needs_search: false,
                            is_constraint: true,
                            constraint_text: Some(message.content.clone()),
                            scope: Some("session".to_string()),
                        });
                    }
                    None => {}
                }
            }
            crate::language_classifier::classify(engine.endpoint(), &message.content)
        });
        // Tier A constraints must affect the response to the instruction that
        // introduces them. Waiting for the background queue made the model
        // acknowledge "do not use emojis" with an emoji before the rule was
        // persisted. Save the structured result now; the background worker
        // remains idempotent and handles later memory tiers.
        if let (Some(session_id), Some(message), Some(classification)) = (
            session_id.as_deref(),
            pending_user.as_ref(),
            language_classification.as_ref(),
        ) {
            if let Some((text, scope)) = classification.session_constraint(&message.content) {
                engine::memory::short_term::save_extracted_constraints(
                    &worker_app,
                    session_id,
                    &[engine::memory::worker::ExtractedConstraint { text, scope }],
                );
            }
        }
        let search_plan = pending_user.as_ref().and_then(|message| {
            let decision = crate::web_search::query::routing_decision(
                &message.content,
                language_classification.as_ref(),
            );
            crate::web_search::observability::log_route(&worker_app, &message.content, &decision);
            matches!(decision, crate::web_search::query::RoutingDecision::Search { .. })
                .then(|| {
                    crate::web_search::query::plan_query(
                        &message.content,
                        language_classification.as_ref(),
                    )
                })
                .flatten()
        });
        let grounding = search_plan.and_then(|plan| {
            let web_budget =
                engine::context_manager::web_context_char_budget(engine.context_size());
            let tier0_providers = tier0_analysis
                .as_ref()
                .map(|analysis| {
                    vec![analysis
                        .provider_candidates
                        .iter()
                        .map(|(provider, _)| *provider)
                        .collect::<Vec<_>>()]
                })
                .unwrap_or_default();
            Some(crate::web_search::orchestrator::run_adaptive_pipeline(
                &worker_app,
                engine.endpoint(),
                embedding_endpoint.as_deref(),
                plan,
                &tier0_providers,
                web_budget,
                |status| {
                    let _ = worker_app.emit("engine-status", StatusEvent { status });
                },
            ))
        }).flatten();
        if let Some(grounding) = &grounding {
            for trace_entry in &grounding.retrieval_trace {
                let _ = worker_app.emit("retrieval-trace", trace_entry);
            }
            request.messages.insert(
                0,
                engine::ChatMessage {
                    role: "system".to_string(),
                    content: grounding.prompt.clone(),
                    created_at: None,
                },
            );
        }
        let mut active_constraints = Vec::new();
        if state.memory_injection_enabled.load(Ordering::SeqCst) {
        if let Some(session_id) = &session_id {
            if let Some(message) = &pending_user {
                let memory_prompts = engine::memory::assemble_tiered_memory_prompts(
                    &worker_app,
                    session_id,
                    &message.content,
                    embedding_endpoint.as_deref(),
                );
                active_constraints = memory_prompts.enforced_constraints.clone();
                let primary = memory_prompts.primary.unwrap_or_default();
                let reminder = memory_prompts.reminder.unwrap_or_default();
                let primary_tokens = if primary.is_empty() {
                    0
                } else {
                    engine.count_message_tokens(&engine::ChatMessage {
                        role: "system".to_string(),
                        content: primary.clone(),
                        created_at: None,
                    })
                };
                let reminder_tokens = if reminder.is_empty() {
                    0
                } else {
                    engine.count_message_tokens(&engine::ChatMessage {
                        role: "system".to_string(),
                        content: reminder.clone(),
                        created_at: None,
                    })
                };
                engine::memory::observability::log_prompt_assembly(
                    &worker_app,
                    session_id,
                    memory_prompts.layer_counts,
                    primary_tokens,
                    reminder_tokens,
                    &primary,
                    &reminder,
                );
                if !primary.is_empty() {
                    request.messages.insert(
                        0,
                        engine::ChatMessage {
                            role: "system".to_string(),
                            content: primary,
                            created_at: None,
                        },
                    );
                }
                if !reminder.is_empty() {
                    let current_user_index = request
                        .messages
                        .iter()
                        .rposition(|item| item.role == "user")
                        .unwrap_or(request.messages.len());
                    request.messages.insert(
                        current_user_index,
                        engine::ChatMessage {
                            role: "system".to_string(),
                            content: reminder,
                            created_at: None,
                        },
                    );
                }
            }
        }
        }
        let original_generation_request = request.clone();
        let web_debug_app = worker_app.clone();
        let web_debug_session_id = session_id.clone();
        let mut result = engine::context_manager::generate_with_recovery(
            engine,
            request,
            &mut memory,
            &time_context,
            |event| match event {
                GenerationEvent::Token(token) => worker_app
                    .emit("engine-token", TokenEvent { token })
                    .map_err(|error| format!("Could not stream a generated token: {error}")),
                GenerationEvent::TrimSuffix(suffix) => worker_app
                    .emit("engine-trim", TrimEvent { suffix })
                    .map_err(|error| format!("Could not trim repetitive output: {error}")),
                GenerationEvent::Status(status) => worker_app
                    .emit("engine-status", StatusEvent { status })
                    .map_err(|error| format!("Could not send generation status: {error}")),
            },
            |prepared| {
                if let Some(grounding) = prepared
                    .messages
                    .iter()
                    .find(|message| message.content.starts_with("[Retrieved Web Sources]"))
                {
                    crate::web_search::observability::log_assembled_prompt(
                        &web_debug_app,
                        web_debug_session_id.as_deref(),
                        &grounding.content,
                    );
                }
            },
            || state.cancel_generation.load(Ordering::SeqCst),
        )?;
        let violations = engine::memory::constraint_guard::violations(
            engine.endpoint(),
            &result.content,
            &active_constraints,
        );
        if !violations.is_empty()
            && !matches!(result.finish_reason, FinishReason::Cancelled)
            && !state.cancel_generation.load(Ordering::SeqCst)
        {
            worker_app
                .emit(
                    "engine-status",
                    StatusEvent {
                        status: "Revising the draft to enforce active memory constraints".to_string(),
                    },
                )
                .map_err(|error| format!("Could not send constraint status: {error}"))?;
            worker_app
                .emit(
                    "engine-trim",
                    TrimEvent {
                        suffix: result.content.clone(),
                    },
                )
                .map_err(|error| format!("Could not replace a noncompliant draft: {error}"))?;
            let mut correction_request = original_generation_request;
            correction_request.messages.push(engine::ChatMessage {
                role: "assistant".to_string(),
                content: result.content.clone(),
                created_at: None,
            });
            correction_request.messages.push(engine::ChatMessage {
                role: "user".to_string(),
                content: format!(
                    "Revise the previous draft because it violated these active requirements: {}. Return only the corrected final answer and preserve supported factual content.",
                    violations.join(" | ")
                ),
                created_at: None,
            });
            result = engine::context_manager::generate_with_recovery(
                engine,
                correction_request,
                &mut memory,
                &time_context,
                |event| match event {
                    GenerationEvent::Token(token) => worker_app
                        .emit("engine-token", TokenEvent { token })
                        .map_err(|error| format!("Could not stream a corrected token: {error}")),
                    GenerationEvent::TrimSuffix(suffix) => worker_app
                        .emit("engine-trim", TrimEvent { suffix })
                        .map_err(|error| format!("Could not trim corrected output: {error}")),
                    GenerationEvent::Status(status) => worker_app
                        .emit("engine-status", StatusEvent { status })
                        .map_err(|error| format!("Could not send correction status: {error}")),
                },
                |prepared| {
                    if let Some(grounding) = prepared
                        .messages
                        .iter()
                        .find(|message| message.content.starts_with("[Retrieved Web Sources]"))
                    {
                        crate::web_search::observability::log_assembled_prompt(
                            &web_debug_app,
                            web_debug_session_id.as_deref(),
                            &grounding.content,
                        );
                    }
                },
                || state.cancel_generation.load(Ordering::SeqCst),
            )?;
        }
        if let Some(grounding) = &grounding {
            if grounding.sources.is_empty()
                && !matches!(result.finish_reason, FinishReason::Cancelled)
                && !state.cancel_generation.load(Ordering::SeqCst)
            {
                let user_request = pending_user
                    .as_ref()
                    .map(|message| message.content.as_str())
                    .unwrap_or_default();
                let safe_answer = crate::web_search::planner::no_evidence_answer(
                    engine.endpoint(),
                    user_request,
                )
                .unwrap_or_else(|| {
                    "The live search completed but returned no usable current evidence for this request. Please try a more specific query.".to_string()
                });
                worker_app
                    .emit(
                        "engine-trim",
                        TrimEvent {
                            suffix: result.content.clone(),
                        },
                    )
                    .map_err(|error| format!("Could not replace an ungrounded draft: {error}"))?;
                worker_app
                    .emit(
                        "engine-token",
                        TokenEvent {
                            token: safe_answer.clone(),
                        },
                    )
                    .map_err(|error| format!("Could not stream the no-evidence answer: {error}"))?;
                result.content = safe_answer;
            }
            let (_claims, _flagged) =
                engine::faithfulness::check_faithfulness(&result.content, grounding);
            result.sources = grounding.sources.clone();
            result.retrieval_trace = grounding.retrieval_trace.clone();
        }
        if let Some(session_id) = &session_id {
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
                None,
                Some(finish_reason_name(&result.finish_reason)),
                Some(&result.sources),
                Some(&result.retrieval_trace),
            )?;
            sessions::store::set_memory(&worker_app, session_id, memory.summary())?;
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
                classification: language_classification.clone(),
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
