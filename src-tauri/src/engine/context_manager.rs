use super::{time_manager, ChatMessage, ChatRequest, Engine, FinishReason, GenerationResult};
use std::collections::HashSet;

const SAFETY_MARGIN_PERCENT: u32 = 6;
const DEFAULT_RESPONSE_TOKENS: u32 = 1_024;
const MIN_RESPONSE_TOKENS: u32 = 256;
const MAX_AUTO_CONTINUATIONS: usize = 3;
const MIN_MEMORY_TOKENS: u32 = 256;
const MAX_MEMORY_TOKENS: u32 = 768;

pub const REASONING_MAX_RESPONSE_TOKENS: u32 = 1_024;
pub const FINAL_ANSWER_MAX_RESPONSE_TOKENS: u32 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HopKind {
    Reasoning,
    FinalAnswer,
}

#[derive(Default)]
pub struct ConversationMemory {
    summary: String,
    summarized_messages: HashSet<u64>,
}

impl ConversationMemory {
    pub fn from_summary(summary: String) -> Self {
        Self {
            summary,
            summarized_messages: HashSet::new(),
        }
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

struct PreparedRequest {
    request: ChatRequest,
    dropped: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Copy)]
struct ContextBudget {
    safety_margin: u32,
    prompt_tokens: u32,
    response_tokens: u32,
    web_tokens: u32,
    memory_tokens: u32,
}

/// The retriever asks for an amount of text that scales with the active model
/// window. It is deliberately below the final web allowance because source
/// labels and the chat template add their own tokens.
pub fn web_context_char_budget(context_size: u32) -> usize {
    let usable = context_size.saturating_mul(100 - SAFETY_MARGIN_PERCENT) / 100;
    let web_tokens = (usable.saturating_mul(32) / 100).clamp(600, 3_000);
    (web_tokens as usize).saturating_mul(4)
}

fn dynamic_budget(
    context_size: u32,
    requested_tokens: u32,
    has_web: bool,
    has_memory: bool,
    hop_kind: HopKind,
) -> ContextBudget {
    let response_ceiling = match hop_kind {
        HopKind::Reasoning => REASONING_MAX_RESPONSE_TOKENS,
        HopKind::FinalAnswer => FINAL_ANSWER_MAX_RESPONSE_TOKENS,
    };
    let safety_margin = (context_size.saturating_mul(SAFETY_MARGIN_PERCENT) / 100).max(128);
    let usable = context_size.saturating_sub(safety_margin);
    let minimum_response = (context_size / 10).clamp(MIN_RESPONSE_TOKENS, 640);
    let response_cap =
        (context_size.saturating_mul(32) / 100).clamp(minimum_response, response_ceiling);
    let response_tokens = requested_tokens
        .clamp(MIN_RESPONSE_TOKENS, response_ceiling)
        .min(response_cap)
        .max(minimum_response);
    let prompt_tokens = usable.saturating_sub(response_tokens);
    let web_tokens = if has_web {
        (prompt_tokens.saturating_mul(48) / 100).clamp(600, 3_000)
    } else {
        0
    };
    let memory_tokens = if has_memory {
        (prompt_tokens.saturating_mul(18) / 100).clamp(MIN_MEMORY_TOKENS, MAX_MEMORY_TOKENS)
    } else {
        0
    };
    ContextBudget {
        safety_margin,
        prompt_tokens,
        response_tokens,
        web_tokens,
        memory_tokens,
    }
}

/// Runs every chat turn behind the Tauri command. The frontend supplies the
/// visible conversation, while this module applies model-tokenizer accounting,
/// a safety margin, invisible sliding-window compaction, and bounded recovery.
pub fn generate_with_recovery<F, P>(
    engine: &mut Engine,
    request: ChatRequest,
    memory: &mut ConversationMemory,
    time_context: &time_manager::TimeContext,
    hop_kind: HopKind,
    mut emit: F,
    mut observe_prepared_request: P,
    should_cancel: impl Fn() -> bool,
) -> Result<GenerationResult, String>
where
    F: FnMut(super::runtime::GenerationEvent) -> Result<(), String>,
    P: FnMut(&ChatRequest),
{
    let mut history = time_manager::inject_gap_markers(&request.messages);
    let requested_tokens = request.max_tokens.unwrap_or(DEFAULT_RESPONSE_TOKENS);
    let temperature = request.temperature;
    let mut full_output = String::new();

    for continuation in 0..=MAX_AUTO_CONTINUATIONS {
        if continuation == 0 {
            emit(super::runtime::GenerationEvent::Status(
                "Checking conversation context".to_string(),
            ))?;
        } else {
            emit(super::runtime::GenerationEvent::Status(
                "Continuing response".to_string(),
            ))?;
        }
        let prepared = prepare_request(
            engine,
            &history,
            requested_tokens,
            temperature,
            memory,
            time_context,
            hop_kind,
        )?;
        if !prepared.dropped.is_empty() {
            emit(super::runtime::GenerationEvent::Status(
                "Compacting earlier conversation".to_string(),
            ))?;
            compact_dropped_messages(
                engine,
                memory,
                &prepared.dropped,
                time_context,
                &should_cancel,
            )?;
        }
        // Rebuild after compaction so the newly-created memory is part of this
        // very request, not delayed until the next user message. A second pass
        // covers the rare case where the memory itself displaces one more turn.
        let prepared = prepare_request(
            engine,
            &history,
            requested_tokens,
            temperature,
            memory,
            time_context,
            hop_kind,
        )?;
        if !prepared.dropped.is_empty() {
            emit(super::runtime::GenerationEvent::Status(
                "Updating conversation memory".to_string(),
            ))?;
            compact_dropped_messages(
                engine,
                memory,
                &prepared.dropped,
                time_context,
                &should_cancel,
            )?;
        }
        let prepared = prepare_request(
            engine,
            &history,
            requested_tokens,
            temperature,
            memory,
            time_context,
            hop_kind,
        )?;

        emit(super::runtime::GenerationEvent::Status(
            "Writing response".to_string(),
        ))?;
        observe_prepared_request(&prepared.request);

        let result = engine.generate(prepared.request, |event| emit(event), &should_cancel)?;
        full_output.push_str(&result.content);

        let finish_reason = result.finish_reason.clone();

        if should_cancel() {
            return Ok(GenerationResult {
                content: full_output,
                finish_reason,
                sources: result.sources,
                retrieval_trace: result.retrieval_trace,
                thinking_summary: result.thinking_summary,
            });
        }

        match finish_reason {
            FinishReason::Stop | FinishReason::Cancelled | FinishReason::RepetitionDetected => {
                return Ok(GenerationResult {
                    content: full_output,
                    finish_reason,
                    sources: result.sources,
                    retrieval_trace: result.retrieval_trace,
                    thinking_summary: result.thinking_summary,
                });
            }
            FinishReason::Length => {
                if continuation == MAX_AUTO_CONTINUATIONS {
                    return Ok(GenerationResult {
                        content: full_output,
                        finish_reason: FinishReason::Length,
                        sources: result.sources,
                        retrieval_trace: result.retrieval_trace,
                        thinking_summary: result.thinking_summary,
                    });
                }

                // Append the partial assistant answer and a explicit continuation user turn.
                // Using two distinct messages lets the model follow its native
                // chat template; we stitch `full_output` locally before returning
                // so the user sees a single uninterrupted response block.
                history.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: result.content,
                    created_at: None,
                    ..Default::default()
                });
                history.push(ChatMessage {
                    role: "user".to_string(),
                    content: "Continue exactly where the previous answer ended. Do not repeat text; preserve the language and Markdown structure.".to_string(),
                    created_at: None,
                    ..Default::default()
                });
            }
        }
    }

    unreachable!("bounded continuation loop always returns")
}

fn prepare_request(
    engine: &Engine,
    history: &[ChatMessage],
    requested_tokens: u32,
    temperature: Option<f32>,
    memory: &ConversationMemory,
    time_context: &time_manager::TimeContext,
    hop_kind: HopKind,
) -> Result<PreparedRequest, String> {
    let context = engine.context_size();
    let has_web = history.iter().any(is_web_context);
    let has_memory = history.iter().any(is_memory_context);
    let budget = dynamic_budget(context, requested_tokens, has_web, has_memory, hop_kind);
    let response_ceiling = match hop_kind {
        HopKind::Reasoning => REASONING_MAX_RESPONSE_TOKENS,
        HopKind::FinalAnswer => FINAL_ANSWER_MAX_RESPONSE_TOKENS,
    };

    let mut kept = vec![time_manager::system_message(time_context)];
    for message in history.iter().filter(|message| message.role == "system") {
        let mut message = message.clone();
        if is_web_context(&message) {
            message.content = truncate_utf8(
                &message.content,
                (budget.web_tokens as usize).saturating_mul(4),
            );
        } else if message.content.starts_with("[Memory Directives]") {
            message.content = truncate_utf8(
                &message.content,
                (budget.memory_tokens as usize).saturating_mul(4),
            );
        }
        kept.push(message);
    }
    if !memory.summary.is_empty() {
        kept.push(ChatMessage {
            role: "system".to_string(),
            content: format!(
                "Conversation memory (private, compact):\n{}",
                memory.summary
            ),
            created_at: None,
            ..Default::default()
        });
    }
    fit_system_context(
        engine,
        &mut kept,
        budget.prompt_tokens,
        budget.memory_tokens,
    );
    let mut used = engine.count_messages_tokens(&kept);
    let non_system = history
        .iter()
        .filter(|message| message.role != "system")
        .cloned()
        .collect::<Vec<_>>();
    let mut newest_first = Vec::new();
    let mut dropped = Vec::new();

    for (index, message) in non_system.iter().enumerate().rev() {
        let cost = engine.count_message_tokens(message);
        if !newest_first.is_empty() && used.saturating_add(cost) > budget.prompt_tokens {
            // Once a recent turn will not fit, every still-older turn belongs
            // to compaction. Never skip a newer turn just to keep an older one.
            dropped.extend(non_system[..=index].iter().cloned());
            break;
        }
        // Always preserve the newest message. If it is exceptionally large,
        // the dynamic n_predict calculation below still leaves a safe floor.
        newest_first.push(message.clone());
        used = used.saturating_add(cost);
    }
    newest_first.reverse();
    dropped.reverse();
    kept.extend(newest_first);

    // The GGUF tokenizer is used above, but a model's chat template adds a few
    // hidden tokens. Enforce the safety limit even for an unusually long final
    // user message so the server never receives an over-context request.
    let hard_prompt_limit = context.saturating_sub(budget.safety_margin + MIN_RESPONSE_TOKENS);
    while engine.count_messages_tokens(&kept) > hard_prompt_limit {
        if shrink_message(&mut kept, "Conversation memory (private, compact):", 800) {
            continue;
        }
        let Some(index) = kept.iter().position(|message| message.role != "system") else {
            // Keep a compact web excerpt whenever possible; it is the
            // evidence layer that prevents a local model from hallucinating.
            if shrink_message(&mut kept, "[Retrieved Web Sources]", 1_200) {
                continue;
            }
            break;
        };
        if kept
            .iter()
            .filter(|message| message.role != "system")
            .count()
            > 1
        {
            dropped.push(kept.remove(index));
            continue;
        }
        let message = &mut kept[index];
        let shortened = truncate_tail(
            &message.content,
            message.content.len().saturating_mul(3) / 4,
        );
        if shortened == message.content {
            break;
        }
        message.content = shortened;
    }

    let actual_prompt_tokens = engine.count_messages_tokens(&kept);
    let remaining = context.saturating_sub(budget.safety_margin + actual_prompt_tokens);
    let max_tokens = remaining
        .clamp(MIN_RESPONSE_TOKENS, response_ceiling)
        .min(budget.response_tokens);

    tracing::info!(
        hop_kind = ?hop_kind,
        requested_tokens,
        response_ceiling,
        actual_prompt_tokens,
        final_max_tokens = max_tokens,
        "Prepared chat request context budget"
    );
    Ok(PreparedRequest {
        request: ChatRequest {
            messages: kept,
            max_tokens: Some(max_tokens),
            temperature,
            session_id: None,
            interaction_id: None,
            interaction_option_id: None,
        },
        dropped,
    })
}

fn is_web_context(message: &ChatMessage) -> bool {
    message.content.starts_with("[Retrieved Web Sources]")
}

fn is_memory_context(message: &ChatMessage) -> bool {
    message.content.starts_with("[Memory Directives]")
        || message.content.starts_with("[Active Memory Reminder]")
}

fn fit_system_context(
    engine: &Engine,
    kept: &mut Vec<ChatMessage>,
    prompt_budget: u32,
    memory_tokens: u32,
) {
    while engine.count_messages_tokens(kept) > prompt_budget {
        if shrink_message(kept, "Conversation memory (private, compact):", 800) {
            continue;
        }
        if shrink_message(kept, "[Retrieved Web Sources]", 1_200) {
            continue;
        }
        // Only the expanded memory context is compressible, and never below
        // its reserved floor. The active reminder itself is non-trimmable.
        if shrink_message_to_floor(
            kept,
            "[Memory Directives]",
            (memory_tokens as usize).saturating_mul(3),
        ) {
            continue;
        }
        break;
    }
}

fn shrink_message_to_floor(
    messages: &mut Vec<ChatMessage>,
    prefix: &str,
    minimum_bytes: usize,
) -> bool {
    let Some(message) = messages
        .iter_mut()
        .find(|message| message.content.starts_with(prefix))
    else {
        return false;
    };
    if message.content.len() <= minimum_bytes {
        return false;
    }
    let target = (message.content.len().saturating_mul(3) / 4).max(minimum_bytes);
    let shortened = truncate_utf8(&message.content, target);
    if shortened == message.content {
        return false;
    }
    message.content = shortened;
    true
}

fn shrink_message(messages: &mut Vec<ChatMessage>, prefix: &str, minimum_bytes: usize) -> bool {
    let Some(index) = messages
        .iter()
        .position(|message| message.content.starts_with(prefix))
    else {
        return false;
    };
    let message = &mut messages[index];
    if message.content.len() <= minimum_bytes {
        messages.remove(index);
        return true;
    }
    let shortened = truncate_utf8(
        &message.content,
        message.content.len().saturating_mul(3) / 4,
    );
    if shortened == message.content {
        return false;
    }
    message.content = shortened;
    true
}

fn compact_dropped_messages(
    engine: &mut Engine,
    memory: &mut ConversationMemory,
    dropped: &[ChatMessage],
    time_context: &time_manager::TimeContext,
    should_cancel: &impl Fn() -> bool,
) -> Result<(), String> {
    let new_messages = dropped
        .iter()
        .filter(|message| !memory.summarized_messages.contains(&message_hash(message)))
        .cloned()
        .collect::<Vec<_>>();
    if new_messages.is_empty() || should_cancel() {
        return Ok(());
    }

    // Keep hidden maintenance work bounded. This executes only when history
    // would otherwise be dropped, and its tokens are never emitted as chat text.
    let source = new_messages
        .iter()
        .map(|message| {
            let timestamp = message
                .created_at
                .as_deref()
                .unwrap_or("timestamp unavailable");
            format!("[{timestamp}] {}: {}", message.role, message.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let source = truncate_utf8(&source, 8_000);
    let summary_prompt = ChatRequest {
        messages: vec![
            time_manager::system_message(time_context),
            ChatMessage { role: "system".to_string(), content: "You are an automated context summarization module. Maintain a concise private memory of the conversation. Preserve facts, decisions, preferences, and unresolved tasks. STRICT TEMPORAL RULES: never use relative time words such as today, yesterday, tomorrow, this morning, or just now. Convert temporal references into explicit ISO dates or absolute timestamps using the timestamps supplied in the conversation log. Do not add commentary.".to_string(), created_at: None, ..Default::default() },
            ChatMessage { role: "user".to_string(), content: format!("Existing memory:\n{}\n\nNew conversation turns to compact (timestamps are UTC):\n{}\n\nReturn an updated memory in under 180 words.", memory.summary, source), created_at: None, ..Default::default() },
        ],
        max_tokens: Some(256),
        temperature: Some(0.3),
        session_id: None,
        interaction_id: None,
        interaction_option_id: None,
    };
    let result = engine.generate(summary_prompt, |_| Ok(()), should_cancel)?;
    if result.finish_reason == FinishReason::Stop && !result.content.trim().is_empty() {
        memory.summary = truncate_utf8(result.content.trim(), 3_000);
        memory
            .summarized_messages
            .extend(new_messages.iter().map(message_hash));
    }
    Ok(())
}

fn message_hash(message: &ChatMessage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    message.role.hash(&mut hasher);
    message.content.hash(&mut hasher);
    message.created_at.hash(&mut hasher);
    hasher.finish()
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

fn truncate_tail(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let prefix = "[Earlier part of this very long message was omitted to fit the model context.]\n";
    if max_bytes <= prefix.len() + 8 {
        return truncate_utf8(value, max_bytes);
    }
    let available = max_bytes.saturating_sub(prefix.len());
    let mut start = value.len().saturating_sub(available);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    format!("{prefix}{}", &value[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_nonzero_memory_budget_under_context_pressure() {
        let budget = dynamic_budget(4_096, 1_536, true, true, HopKind::FinalAnswer);
        assert!(budget.memory_tokens >= MIN_MEMORY_TOKENS);
    }

    #[test]
    fn final_answer_hop_gets_full_ceiling_regardless_of_reasoning_hops() {
        let budget_reasoning = dynamic_budget(32_768, 4_096, true, true, HopKind::Reasoning);
        assert_eq!(budget_reasoning.response_tokens, REASONING_MAX_RESPONSE_TOKENS);

        let budget_final = dynamic_budget(32_768, 4_096, true, true, HopKind::FinalAnswer);
        assert_eq!(budget_final.response_tokens, FINAL_ANSWER_MAX_RESPONSE_TOKENS);
    }

    #[test]
    fn memory_floor_is_never_removed() {
        let original = "[Memory Directives]\nYou MUST answer in Thai.".to_string();
        let mut messages = vec![ChatMessage {
            role: "system".to_string(),
            content: original.clone(),
            created_at: None,
            ..Default::default()
        }];
        assert!(!shrink_message_to_floor(
            &mut messages,
            "[Memory Directives]",
            256
        ));
        assert_eq!(messages[0].content, original);
    }
}
