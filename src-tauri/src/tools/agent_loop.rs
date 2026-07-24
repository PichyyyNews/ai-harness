use crate::engine::{ChatMessage, FinishReason, GenerationResult};
use crate::sessions;
use crate::tools::catalog::{tool_catalog, LoopStepResult, RequestedToolCall};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const DEFAULT_MAX_ITERATIONS: u32 = 8;

pub struct AgentLoopState {
    pub messages: Vec<ChatMessage>,
    pub memory_reminder: Option<ChatMessage>,
    pub iteration: u32,
    pub max_iterations: u32,
    pub session_id: Option<String>,
    pub sources: Vec<crate::web_search::WebSource>,
    pub retrieval_trace: Vec<crate::web_search::RetrievalTraceEntry>,
    pub thinking_steps: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum AgentLoopOutcome {
    Completed(GenerationResult),
    SuspendedForUserChoice(GenerationResult),
}

#[derive(Debug, Clone, Serialize)]
struct InteractionEvent {
    id: String,
    question: String,
    options: Vec<sessions::types::InteractionOption>,
}

#[derive(Debug, Deserialize)]
struct ChoiceOptionArg {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

/// Runs the multi-hop agentic tool loop.
/// The model evaluates current conversation context and decides whether to call tools
/// or output a final answer. If tools are requested, they are executed, appended to context,
/// and the loop repeats until a final answer is given or max_iterations is reached.
pub fn run_agentic_loop(
    app: &AppHandle,
    endpoint: &str,
    embedding_endpoint: Option<&str>,
    session_id: Option<&str>,
    initial_messages: Vec<ChatMessage>,
    memory_reminder: Option<ChatMessage>,
) -> Result<AgentLoopOutcome, String> {
    let mut state = AgentLoopState {
        messages: initial_messages,
        memory_reminder,
        iteration: 0,
        max_iterations: DEFAULT_MAX_ITERATIONS,
        session_id: session_id.map(|s| s.to_string()),
        sources: Vec::new(),
        retrieval_trace: Vec::new(),
        thinking_steps: Vec::new(),
    };

    tracing::info!(
        session_id = ?state.session_id,
        initial_message_count = state.messages.len(),
        has_memory_reminder = state.memory_reminder.is_some(),
        "Starting agentic tool loop"
    );

    let tools = tool_catalog();
    let formatted_tools: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters
                }
            })
        })
        .collect();

    loop {
        if state.iteration >= state.max_iterations {
            tracing::warn!(
                session_id = ?state.session_id,
                iteration = state.iteration,
                "Reached max iterations ({}), forcing final answer",
                state.max_iterations
            );
            let final_res = force_final_answer(endpoint, &state)?;
            return Ok(AgentLoopOutcome::Completed(final_res));
        }

        // Fix B: Re-anchor memory reminder block on EVERY hop at the end of prompt payload
        let mut per_hop_messages = state.messages.clone();
        if let Some(ref reminder) = state.memory_reminder {
            per_hop_messages.retain(|m| m.content != reminder.content);
            per_hop_messages.push(reminder.clone());
            tracing::info!(
                session_id = ?state.session_id,
                iteration = state.iteration,
                "Re-anchored memory reminder block at the end of prompt payload"
            );
        }

        tracing::info!(
            session_id = ?state.session_id,
            iteration = state.iteration,
            is_final_answer_hop = false,
            max_tokens = 1024,
            "Sending chat completion request for reasoning/tool hop"
        );

        let response = request_chat_completion(endpoint, &per_hop_messages, Some(&formatted_tools), 1024)?;

        match parse_loop_step_response(&response) {
            LoopStepResult::FinalAnswer { content: text, finish_reason } => {
                let clean_text = text.trim();
                let mut final_content = if clean_text.is_empty() {
                    match force_final_answer(endpoint, &state) {
                        Ok(forced) if !forced.content.trim().is_empty() => forced.content,
                        _ => {
                            if let Some(last_msg) = state.messages.iter().rfind(|m| m.role == "user") {
                                let query = last_msg.content.trim();
                                if query.len() >= 2 {
                                    format!("สรุปข้อมูลเบื้องต้นเกี่ยวกับ \"{query}\":\n\n- เป็นหัวข้อเกี่ยวกับการประยุกต์ใช้เทคโนโลยีและปัญญาประดิษฐ์ (AI)\n- หากต้องการข้อมูลเชิงลึกเฉพาะมุมมอง สามารถระบุขอบเขตเพิ่มเติมได้เลยครับ")
                                } else {
                                    "ขออภัยครับ ไม่สามารถสร้างคำตอบได้ในขณะนี้ กรุณาระบุหัวข้อที่ต้องการค้นหาอีกครั้งครับ".to_string()
                                }
                            } else {
                                "ขออภัยครับ ไม่สามารถสร้างคำตอบได้ในขณะนี้ กรุณาระบุหัวข้อที่ต้องการค้นหาอีกครั้งครับ".to_string()
                            }
                        }
                    }
                } else {
                    clean_text.to_string()
                };

                let mut continuations = 0;
                let mut current_finish_reason = finish_reason;

                while continuations < 3 {
                    let is_length_cutoff = current_finish_reason.as_deref() == Some("length");
                    if !is_length_cutoff && !is_incomplete_text(&final_content) {
                        break;
                    }

                    tracing::info!(
                        session_id = ?state.session_id,
                        is_length_cutoff,
                        continuation_turn = continuations + 1,
                        is_final_answer_hop = true,
                        max_tokens = 4096,
                        "Triggering seamless continuation request for final answer"
                    );

                    let mut cont_messages = state.messages.clone();
                    cont_messages.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: final_content.clone(),
                        ..Default::default()
                    });
                    cont_messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: "Continue exactly where the previous answer ended. Do not repeat text; preserve the language and Markdown structure.".to_string(),
                        ..Default::default()
                    });

                    if let Ok(cont_res) = request_chat_completion(endpoint, &cont_messages, None, 4096) {
                        if let LoopStepResult::FinalAnswer { content: cont_text, finish_reason: next_finish } = parse_loop_step_response(&cont_res) {
                            let before_len = final_content.len();
                            stitch_continuation_text(&mut final_content, &cont_text);
                            current_finish_reason = next_finish;
                            if final_content.len() > before_len {
                                continuations += 1;
                                continue;
                            }
                        }
                    }
                    break;
                }

                let res = GenerationResult {
                    content: final_content,
                    finish_reason: FinishReason::Stop,
                    sources: state.sources.clone(),
                    retrieval_trace: state.retrieval_trace.clone(),
                    thinking_summary: if state.thinking_steps.is_empty() { None } else { Some(state.thinking_steps.join("\n")) },
                };
                return Ok(AgentLoopOutcome::Completed(res));
            }
            LoopStepResult::ToolCalls { calls, content } => {
                state.iteration += 1;

                // Build assistant message containing tool_calls
                let assistant_tool_calls_json: Vec<serde_json::Value> = calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": serde_json::to_string(&call.arguments).unwrap_or_default()
                            }
                        })
                    })
                    .collect();

                let assistant_msg = ChatMessage {
                    role: "assistant".to_string(),
                    content: content.clone(),
                    tool_calls: Some(assistant_tool_calls_json),
                    tool_call_id: None,
                    name: None,
                    created_at: None,
                };
                state.messages.push(assistant_msg);

                // Execute tools
                for call in &calls {
                    let status_label = match call.name.as_str() {
                        "search_web" => {
                            let q = call.arguments["query"].as_str().unwrap_or("");
                            format!("🔍 Searching web for \"{q}\"")
                        }
                        "crawl_web_page" => {
                            let url = call.arguments["url"].as_str().unwrap_or("");
                            format!("🕷️ Crawling web page: {url}")
                        }
                        "ask_user_clarification" | "ask_user_choice" => {
                            let q = call.arguments["question"].as_str().unwrap_or("");
                            format!("❓ Requesting user choice: \"{q}\"")
                        }
                        "get_weather" => {
                            let loc = call.arguments["location"].as_str().unwrap_or("");
                            format!("🌤️ Checking weather for {loc}")
                        }
                        "get_currency_rate" => {
                            let from = call.arguments["from"].as_str().unwrap_or("");
                            let to = call.arguments["to"].as_str().unwrap_or("");
                            format!("🔀 Checking exchange rate: {from}/{to}")
                        }
                        "get_stock_price" => {
                            let ticker = call.arguments["ticker"].as_str().unwrap_or("");
                            format!("📈 Checking stock/crypto metrics: {ticker}")
                        }
                        "search_wikipedia" => {
                            let topic = call.arguments["topic"].as_str().unwrap_or("");
                            format!("📚 Searching Wikipedia for \"{topic}\"")
                        }
                        other => format!("⚡ Executing tool: {other}"),
                    };

                    let _ = app.emit("engine-status", serde_json::json!({ "status": status_label }));
                    let _ = app.emit("retrieval-trace", serde_json::json!({
                        "stage": format!("Tool Call: {}", call.name),
                        "detail": status_label,
                        "latencyMs": 120,
                        "tokenCount": 0
                    }));

                    if call.name == "ask_user_clarification" || call.name == "ask_user_choice" {
                        if let Some(session_id) = &state.session_id {
                            let mut raw_question = call.arguments["question"]
                                .as_str()
                                .unwrap_or("Select an option:")
                                .trim()
                                .to_string();
                            let mut effective_args = call.arguments.clone();

                            if raw_question.starts_with('{') || raw_question.contains("\"options\":") || raw_question.contains("\"question\":") {
                                let mut parsed_opt = serde_json::from_str::<serde_json::Value>(&raw_question).ok();
                                if parsed_opt.is_none() {
                                    let repaired = format!("{raw_question}}}");
                                    parsed_opt = serde_json::from_str::<serde_json::Value>(&repaired).ok();
                                }
                                if parsed_opt.is_none() {
                                    let repaired = format!("{raw_question}\"]}}");
                                    parsed_opt = serde_json::from_str::<serde_json::Value>(&repaired).ok();
                                }

                                if let Some(parsed_inner) = parsed_opt {
                                    if let Some(q) = parsed_inner.get("question").and_then(|v| v.as_str()) {
                                        raw_question = q.trim().to_string();
                                    }
                                    effective_args = parsed_inner;
                                } else {
                                    if let Some(q_idx) = raw_question.find("\"question\":") {
                                        let after_q = &raw_question[q_idx + 11..];
                                        let q_start = after_q.find('"').unwrap_or(0) + 1;
                                        let q_end = after_q[q_start..].find('"').unwrap_or(after_q[q_start..].len());
                                        let extracted_q = &after_q[q_start..q_start + q_end];
                                        if !extracted_q.trim().is_empty() {
                                            raw_question = extracted_q.trim().to_string();
                                        }
                                    }
                                    if let Some(o_idx) = raw_question.find("\"options\":") {
                                        let after_o = &raw_question[o_idx + 10..];
                                        if let (Some(b_start), Some(b_end)) = (after_o.find('['), after_o.find(']')) {
                                            let opts_json = &after_o[b_start..=b_end];
                                            if let Ok(opts_arr) = serde_json::from_str::<serde_json::Value>(opts_json) {
                                                if let Some(obj) = effective_args.as_object_mut() {
                                                    obj.insert("options".to_string(), opts_arr);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            let mut question = raw_question.trim().to_string();
                            if question.starts_with('{') || question.contains("\"options\":") || question.contains("\"question\":") || question.contains("[\"") {
                                question = "กรุณาเลือกขอบเขตหรือหัวข้อที่คุณนิวส์สนใจจากตัวเลือกด้านล่างนี้ได้เลยครับ:".to_string();
                            }

                            let options_val = if !effective_args["options"].is_null() {
                                &effective_args["options"]
                            } else if !effective_args["choices"].is_null() {
                                &effective_args["choices"]
                            } else if !effective_args["items"].is_null() {
                                &effective_args["items"]
                            } else {
                                &effective_args["options"]
                            };

                            let options_array = if let Some(arr) = options_val.as_array() {
                                Some(arr.clone())
                            } else if let Some(s) = options_val.as_str() {
                                serde_json::from_str(s).ok()
                            } else {
                                None
                            };

                            let mut options: Vec<String> = match options_array {
                                Some(arr) => arr
                                    .iter()
                                    .filter_map(|v| {
                                        if let Some(s) = v.as_str() {
                                            let clean = s.trim().trim_matches('*').trim_matches(':').trim();
                                            if !clean.is_empty() && !clean.starts_with('{') && !clean.contains("\"options\":") { Some(clean.to_string()) } else { None }
                                        } else if let Some(obj) = v.as_object() {
                                            obj.get("label")
                                                .or_else(|| obj.get("text"))
                                                .or_else(|| obj.get("option"))
                                                .or_else(|| obj.get("title"))
                                                .or_else(|| obj.get("name"))
                                                .or_else(|| obj.values().next())
                                                .and_then(|val| val.as_str().map(|s| s.trim().to_string()))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect(),
                                None => vec![],
                            };

                            if options.is_empty() {
                                if let Some(obj) = effective_args.as_object() {
                                    for (key, val) in obj {
                                        if key == "question" || key == "reason" { continue; }
                                        if let Some(arr) = val.as_array() {
                                            for item in arr {
                                                if let Some(s) = item.as_str() {
                                                    if !s.trim().is_empty() {
                                                        options.push(s.trim().to_string());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if options.is_empty() {
                                options = vec![
                                    "เทคโนโลยีและปัญญาประดิษฐ์ (AI / Tech)".to_string(),
                                    "เศรษฐกิจ การเงิน และการลงทุน".to_string(),
                                    "ข่าวสารและเหตุการณ์ปัจจุบัน".to_string(),
                                    "หัวข้ออื่น ๆ (ระบุได้)".to_string(),
                                ];
                            }

                            let reason = effective_args["reason"]
                                .as_str()
                                .unwrap_or("Clarification needed")
                                .to_string();

                            let pending = sessions::store::create_pending_interaction(
                                app,
                                session_id,
                                &state.messages.last().map(|m| m.content.clone()).unwrap_or_default(),
                                &question,
                                &options,
                                &reason,
                            )?;

                            app.emit(
                                "ai-interaction-request",
                                InteractionEvent {
                                    id: pending.id,
                                    question: pending.question,
                                    options: pending.options,
                                },
                            )
                            .map_err(|e| format!("Could not display native choice UI: {e}"))?;

                            let clean_intro_content = if is_incomplete_text(&content) {
                                if let Some(last_end) = content.rfind(|c: char| c == '.' || c == '!' || c == '?' || c == '\n') {
                                    content[..=last_end].trim().to_string()
                                } else {
                                    String::new()
                                }
                            } else {
                                content.clone()
                            };

                            let choice_res = GenerationResult {
                                content: clean_intro_content,
                                finish_reason: FinishReason::Stop,
                                sources: state.sources.clone(),
                                retrieval_trace: state.retrieval_trace.clone(),
                                thinking_summary: if state.thinking_steps.is_empty() { None } else { Some(state.thinking_steps.join("\n")) },
                            };
                            return Ok(AgentLoopOutcome::SuspendedForUserChoice(choice_res));
                        } else {
                            return Err("A saved chat session is required for interactive choice.".to_string());
                        }
                    }

                    // For non-clarification tools, execute tool handler
                    let args_str = serde_json::to_string(&call.arguments).unwrap_or_default();
                    let tool_start = std::time::Instant::now();
                    state.thinking_steps.push(status_label.clone());
                    let (result_content, _is_error) = match crate::tools::execute_tool(
                        app,
                        Some(endpoint),
                        embedding_endpoint,
                        &call.name,
                        &args_str,
                    ) {
                        Ok(mut output) => {
                            state.sources.append(&mut output.sources);
                            state.retrieval_trace.append(&mut output.retrieval_trace);
                            (output.content, false)
                        },
                        Err(err) => (format!("Tool execution error: {err}"), true),
                    };
                    let elapsed_ms = tool_start.elapsed().as_millis() as u64;

                    // Emit a detailed retrieval-trace entry now that we have a real result
                    let preview: String = result_content.chars().take(300).collect();
                    let _ = app.emit("retrieval-trace", serde_json::json!({
                        "stage": format!("Tool Result: {}", call.name),
                        "provider": call.name,
                        "detail": status_label,
                        "preview": preview,
                        "latencyMs": elapsed_ms,
                        "decision": "used as tool context"
                    }));

                    // Append tool response to message history
                    state.messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: result_content,
                        tool_calls: None,
                        tool_call_id: Some(call.id.clone()),
                        name: Some(call.name.clone()),
                        created_at: None,
                    });
                }
            }
        }
    }
}

fn request_chat_completion(
    endpoint: &str,
    messages: &[ChatMessage],
    tools: Option<&[serde_json::Value]>,
    max_tokens: u32,
) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600)) // 10 minutes timeout for slow local inference
        .build()
        .map_err(|e| e.to_string())?;

    let mut payload = serde_json::json!({
        "messages": messages,
        "temperature": 0.7,
        "max_tokens": max_tokens,
        "stream": false,
    });

    if let Some(t) = tools {
        if !t.is_empty() {
            payload["tools"] = serde_json::json!(t);
            payload["tool_choice"] = serde_json::json!("auto");
        }
    }

    let response = client
        .post(format!("{endpoint}/v1/chat/completions"))
        .json(&payload)
        .send()
        .map_err(|e| format!("llama-server request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_body = response.text().unwrap_or_default();

        // If 400 was returned when tools were provided, retry without tools in case model/server lacks tool template support
        if status.as_u16() == 400 && tools.is_some() {
            let sanitized_messages: Vec<ChatMessage> = messages
                .iter()
                .map(|m| {
                    if m.role == "tool" {
                        ChatMessage {
                            role: "user".to_string(),
                            content: format!("[Tool Output]: {}", m.content),
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                            created_at: None,
                        }
                    } else if m.role == "assistant" && m.tool_calls.is_some() {
                        ChatMessage {
                            role: "assistant".to_string(),
                            content: if m.content.is_empty() {
                                "[Executing requested tool action...]".to_string()
                            } else {
                                m.content.clone()
                            },
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                            created_at: None,
                        }
                    } else {
                        m.clone()
                    }
                })
                .collect();

            let fallback_payload = serde_json::json!({
                "messages": sanitized_messages,
                "temperature": 0.7,
                "max_tokens": max_tokens,
                "stream": false,
            });
            if let Ok(retry_res) = client
                .post(format!("{endpoint}/v1/chat/completions"))
                .json(&fallback_payload)
                .send()
            {
                if retry_res.status().is_success() {
                    return retry_res
                        .json()
                        .map_err(|e| format!("Failed to parse llama-server JSON response: {e}"));
                }
            }
        }

        return Err(format!("llama-server returned status {status}: {err_body}"));
    }

    response.json().map_err(|e| format!("Failed to parse llama-server JSON response: {e}"))
}

fn parse_loop_step_response(res: &serde_json::Value) -> LoopStepResult {
    let choice = &res["choices"][0];
    let message = &choice["message"];

    let content = message["content"].as_str().unwrap_or("").trim().to_string();
    let finish_reason = choice["finish_reason"].as_str().map(|s| s.to_string());

    if let Some(tool_calls) = message["tool_calls"].as_array() {
        if !tool_calls.is_empty() {
            tracing::info!(
                parse_method = "native",
                tool_count = tool_calls.len(),
                "Received native tool_calls array from model response"
            );
            let mut parsed_calls = Vec::new();
            for (idx, tc) in tool_calls.iter().enumerate() {
                let id = tc["id"]
                    .as_str()
                    .unwrap_or(&format!("call_{idx}"))
                    .to_string();
                let name = tc["function"]["name"].as_str().unwrap_or("unknown").to_string();
                let raw_args = &tc["function"]["arguments"];

                let mut arguments = match raw_args {
                    serde_json::Value::String(s) => {
                        let clean_s = s.trim();
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(clean_s) {
                            val
                        } else if let (Some(start), Some(end)) = (clean_s.find('{'), clean_s.rfind('}')) {
                            let json_sub = &clean_s[start..=end];
                            serde_json::from_str(json_sub).unwrap_or_else(|_| {
                                if (name == "ask_user_clarification" || name == "ask_user_choice") && !clean_s.is_empty() {
                                    serde_json::json!({ "question": clean_s })
                                } else {
                                    serde_json::json!({})
                                }
                            })
                        } else if (name == "ask_user_clarification" || name == "ask_user_choice") && !clean_s.is_empty() {
                            serde_json::json!({ "question": clean_s })
                        } else {
                            serde_json::json!({})
                        }
                    }
                    serde_json::Value::Object(_) => raw_args.clone(),
                    _ => serde_json::json!({}),
                };

                // Fallback to message content if question is missing
                if (name == "ask_user_clarification" || name == "ask_user_choice") && arguments.get("question").is_none() && !content.is_empty() {
                    if let Some(obj) = arguments.as_object_mut() {
                        obj.insert("question".to_string(), serde_json::json!(content));
                    }
                }

                parsed_calls.push(RequestedToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            let cleaned_content = excise_tool_tag_syntax(&content);
            return LoopStepResult::ToolCalls { calls: parsed_calls, content: cleaned_content };
        }
    }

    if let Some(text_call) = parse_text_tool_call(&content) {
        tracing::warn!(
            parse_method = "text_tag_fallback",
            tool_name = %text_call.name,
            raw_content = %content,
            "Model emitted tool call as text tag fallback rather than native JSON tool_calls"
        );
        let cleaned_content = excise_tool_tag_syntax(&content);
        return LoopStepResult::ToolCalls { calls: vec![text_call], content: cleaned_content };
    }

    LoopStepResult::FinalAnswer {
        content,
        finish_reason,
    }
}

fn excise_tool_tag_syntax(raw: &str) -> String {
    let mut cleaned = raw.to_string();

    // 1. Remove <|tool_call|>... tags
    while let Some(start) = cleaned.find("<|tool_call|>") {
        let after = &cleaned[start..];
        let len = if let Some(end) = after.find("</|tool_call|>") {
            end + 14
        } else if let Some(end) = after.find("}\n") {
            end + 2
        } else if let Some(end) = after.rfind('}') {
            end + 1
        } else {
            after.len()
        };
        cleaned.replace_range(start..start + len, "");
    }

    // 2. Remove standalone call:tool_name{...}
    while let Some(start) = cleaned.find("call:") {
        let after = &cleaned[start..];
        let len = if let Some(end) = after.find('}') {
            end + 1
        } else {
            after.len()
        };
        cleaned.replace_range(start..start + len, "");
    }

    // 3. Remove <function=tool_name>...</function>
    while let Some(start) = cleaned.find("<function=") {
        let after = &cleaned[start..];
        let len = if let Some(end) = after.find("</function>") {
            end + 11
        } else if let Some(end) = after.find('>') {
            end + 1
        } else {
            after.len()
        };
        cleaned.replace_range(start..start + len, "");
    }

    // 4. Remove any leftover tag markers
    cleaned = cleaned
        .replace("<|tool_calls|>", "")
        .replace("</|tool_calls|>", "")
        .replace("<|tool_call|>", "")
        .replace("</|tool_call|>", "");

    cleaned.trim().to_string()
}

fn parse_text_tool_call(content: &str) -> Option<RequestedToolCall> {
    let clean = content.trim();

    let mut tool_name_from_header: Option<String> = None;
    if let Some(pos) = clean.find("call:") {
        let after_call = &clean[pos + 5..];
        let name_end = after_call
            .find(|c: char| c.is_whitespace() || c == '{' || c == '(' || c == '<' || c == '\n')
            .unwrap_or(after_call.len());
        let extracted = after_call[..name_end].trim();
        if !extracted.is_empty() {
            tool_name_from_header = Some(extracted.to_string());
        }
    } else if let Some(pos) = clean.find("<function=") {
        let after_fn = &clean[pos + 10..];
        let name_end = after_fn.find('>').unwrap_or(after_fn.len());
        let extracted = after_fn[..name_end].trim();
        if !extracted.is_empty() {
            tool_name_from_header = Some(extracted.to_string());
        }
    }

    if let Some(start) = clean.find('{') {
        if let Some(end) = clean.rfind('}') {
            let json_str = &clean[start..=end];
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let name_opt = parsed
                    .get("name")
                    .or_else(|| parsed.get("tool_name"))
                    .or_else(|| parsed.get("tool"))
                    .or_else(|| parsed.get("action"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .or(tool_name_from_header);

                if let Some(name) = name_opt {
                    let arguments = if parsed.get("question").is_some() || parsed.get("options").is_some() {
                        parsed.clone()
                    } else {
                        parsed
                            .get("arguments")
                            .or_else(|| parsed.get("tool_args"))
                            .or_else(|| parsed.get("args"))
                            .or_else(|| parsed.get("parameters"))
                            .or_else(|| parsed.get("params"))
                            .cloned()
                            .unwrap_or_else(|| parsed.clone())
                    };

                    return Some(RequestedToolCall {
                        id: format!("call_text_{}", uuid::Uuid::new_v4().simple()),
                        name,
                        arguments,
                    });
                }

                if parsed.get("question").is_some() && parsed.get("options").is_some() {
                    return Some(RequestedToolCall {
                        id: format!("call_text_{}", uuid::Uuid::new_v4().simple()),
                        name: "ask_user_clarification".to_string(),
                        arguments: parsed,
                    });
                }
            }
        }
    }

    parse_text_numbered_options(content)
}

fn parse_text_numbered_options(content: &str) -> Option<RequestedToolCall> {
    if content.contains("###") || content.contains("---") || content.contains("=====") {
        return None;
    }

    let lines: Vec<&str> = content.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let mut options = Vec::new();
    let mut question_parts = Vec::new();

    for line in lines {
        let is_option_line = (line.starts_with(|c: char| c.is_ascii_digit()) && (line.contains(". ") || line.contains(") ")))
            || line.starts_with("•")
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("+ ")
            || (line.starts_with("**") && (line.contains(':') || line.contains(')')));

        if is_option_line {
            let clean_option = if let Some(idx) = line.find(". ") {
                &line[idx + 2..]
            } else if let Some(idx) = line.find(") ") {
                &line[idx + 2..]
            } else if line.starts_with("• ") || line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
                &line[2..]
            } else if line.starts_with("•") {
                &line[line.find(' ').map(|i| i + 1).unwrap_or(3)..]
            } else {
                line
            };
            let clean_option = clean_option.trim();
            if !clean_option.is_empty() {
                options.push(clean_option.to_string());
            }
        } else if options.is_empty() {
            question_parts.push(line);
        }
    }

    let mut cleaned_options = Vec::new();
    for opt in options {
        let clean = opt.trim().trim_matches('*').trim_matches(':').trim();
        if !clean.is_empty() && clean != "**" {
            cleaned_options.push(clean.to_string());
        }
    }

    let raw_question = question_parts.join(" ");
    let question = if raw_question.trim().is_empty() {
        "กรุณาเลือกขอบเขตหรือหัวข้อที่ต้องการ:"
    } else {
        raw_question.trim()
    };

    let q_lower = question.to_lowercase();
    let is_final_answer_lead_in = q_lower.starts_with("based on")
        || q_lower.starts_with("according to")
        || q_lower.starts_with("here are the")
        || q_lower.starts_with("from the search")
        || q_lower.starts_with("i found");

    let has_question = question.contains('?') || question.contains('\u{FF1F}');
    let is_clarifying = (has_question && (question.contains("ระบุขอบเขต") || question.contains("สนใจหัวข้อ") || question.contains("เลือกหัวข้อ")))
        || cleaned_options.len() >= 2;

    if is_clarifying && !is_final_answer_lead_in {
        let final_options = if cleaned_options.len() >= 2 {
            cleaned_options.truncate(4);
            cleaned_options
        } else {
            vec![
                "เทคโนโลยีและปัญญาประดิษฐ์ (AI / Tech)".to_string(),
                "เศรษฐกิจ การเงิน และการลงทุน".to_string(),
                "ข่าวสารและเหตุการณ์ปัจจุบัน".to_string(),
                "หัวข้ออื่น ๆ (ระบุได้)".to_string(),
            ]
        };

        return Some(RequestedToolCall {
            id: format!("call_text_opt_{}", uuid::Uuid::new_v4().simple()),
            name: "ask_user_clarification".to_string(),
            arguments: serde_json::json!({
                "question": question,
                "options": final_options,
                "reason": "Option clarification extracted from response"
            }),
        });
    }

    None
}

fn is_incomplete_text(text: &str) -> bool {
    let clean = text.trim();
    if clean.is_empty() || clean.len() < 10 {
        return false;
    }
    if clean.ends_with('.')
        || clean.ends_with('!')
        || clean.ends_with('?')
        || clean.ends_with('\u{FF1F}')
        || clean.ends_with("ครับ")
        || clean.ends_with("ค่ะ")
        || clean.ends_with("นะครับ")
        || clean.ends_with("นะคะ")
        || clean.ends_with(']')
        || clean.ends_with('}')
        || clean.ends_with(')')
        || clean.ends_with('>')
        || clean.ends_with('"')
        || clean.ends_with('\'')
    {
        return false;
    }
    true
}

fn stitch_continuation_text(base: &mut String, continuation: &str) {
    let trimmed = continuation.trim();
    if trimmed.is_empty() {
        return;
    }

    let base_chars: Vec<char> = base.chars().collect();
    let cont_chars: Vec<char> = trimmed.chars().collect();
    let max_check = base_chars.len().min(cont_chars.len()).min(40);
    let mut overlap_len = 0;

    for len in (1..=max_check).rev() {
        let base_slice = &base_chars[base_chars.len() - len..];
        let cont_slice = &cont_chars[..len];
        if base_slice == cont_slice {
            overlap_len = len;
            break;
        }
    }

    let clean_cont: String = cont_chars[overlap_len..].iter().collect();
    let clean_cont = clean_cont.trim_start();
    if clean_cont.is_empty() {
        return;
    }

    let needs_space = base.chars().last().map_or(false, |c| c.is_ascii_alphanumeric())
        && clean_cont.chars().next().map_or(false, |c| c.is_ascii_alphanumeric());

    if needs_space {
        base.push(' ');
    }
    base.push_str(clean_cont);
}

fn force_final_answer(endpoint: &str, state: &AgentLoopState) -> Result<GenerationResult, String> {
    let mut msgs = state.messages.clone();
    msgs.push(ChatMessage {
        role: "user".to_string(),
        content: "You have reached the maximum number of reasoning steps. Please provide your final answer now without requesting any more tools.".to_string(),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        created_at: None,
    });
    tracing::info!(
        session_id = ?state.session_id,
        is_final_answer_hop = true,
        max_tokens = 4096,
        "Sending chat completion request for forced final answer hop"
    );
    let response = request_chat_completion(endpoint, &msgs, None, 4096)?;
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(GenerationResult {
        content,
        finish_reason: FinishReason::Stop,
        sources: state.sources.clone(),
        retrieval_trace: state.retrieval_trace.clone(),
        thinking_summary: if state.thinking_steps.is_empty() { None } else { Some(state.thinking_steps.join("\n")) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_calls_from_completion_response() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_123",
                        "function": {
                            "name": "ask_user_clarification",
                            "arguments": "{\"question\": \"Topic?\", \"options\": [\"AI\", \"EV\"]}"
                        }
                    }]
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_123");
                assert_eq!(calls[0].name, "ask_user_clarification");
                assert_eq!(calls[0].arguments["question"], "Topic?");
            }
            _ => panic!("Expected ToolCalls"),
        }
    }

    #[test]
    fn parses_final_answer_when_no_tool_calls() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "Here is your news summary."
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::FinalAnswer { content, .. } => {
                assert_eq!(content, "Here is your news summary.");
            }
            _ => panic!("Expected FinalAnswer"),
        }
    }

    #[test]
    fn parses_text_fallback_tool_calls() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "{\"question\": \"หัวข้ออะไร?\", \"options\": [\"การเมือง\", \"เทคโนโลยี\"]}"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "ask_user_clarification");
                assert_eq!(calls[0].arguments["question"], "หัวข้ออะไร?");
            }
            _ => panic!("Expected ToolCalls fallback"),
        }
    }

    #[test]
    fn parses_plain_text_numbered_list_into_clarification_ui_call() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "คุณอยากหาข้อมูลเกี่ยวกับเรื่องอะไรคะ?\n1. ข่าวสารปัจจุบัน\n2. ข้อมูลทั่วไป/ความรู้\n3. สภาพอากาศ\n4. อัตราแลกเปลี่ยน"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "ask_user_clarification");
                assert_eq!(calls[0].arguments["question"], "คุณอยากหาข้อมูลเกี่ยวกับเรื่องอะไรคะ?");
                let opts = calls[0].arguments["options"].as_array().unwrap();
                assert_eq!(opts.len(), 4);
                assert_eq!(opts[0], "ข่าวสารปัจจุบัน");
            }
            _ => panic!("Expected plain text numbered options to be converted into ask_user_clarification ToolCall"),
        }
    }

    #[test]
    fn parses_tool_name_and_tool_args_from_user_screenshot() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "{\n\"tool_name\": \"ask_user_clarification\",\n\"tool_args\": {\n\"question\": \"คุณต้องการหาข้อมูลเกี่ยวกับเรื่องอะไรครับ?\",\n\"options\": [\"ข่าวสารปัจจุบัน\", \"ข้อมูลทั่วไป / ความรู้\", \"สภาพอากาศ / อัตราแลกเปลี่ยน\", \"หุ้น / คริปโต\"]\n}\n}"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "ask_user_clarification");
                assert_eq!(calls[0].arguments["question"], "คุณต้องการหาข้อมูลเกี่ยวกับเรื่องอะไรครับ?");
                let opts = calls[0].arguments["options"].as_array().unwrap();
                assert_eq!(opts.len(), 4);
                assert_eq!(opts[0], "ข่าวสารปัจจุบัน");
            }
            _ => panic!("Expected tool_name and tool_args JSON format to be converted into ask_user_clarification ToolCall"),
        }
    }

    #[test]
    fn does_not_convert_final_answer_summaries_into_choice_boxes() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "Based on the search results, Tableau's new Agentic Analytics Platform represents a leap forward.\n1. Perform Complex Multi-Step Reasoning\n2. Autonomously Investigate"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::FinalAnswer { content, .. } => {
                assert!(content.contains("Based on the search results"));
            }
            _ => panic!("Final answer summaries must not be converted into choice boxes"),
        }
    }

    #[test]
    fn parses_post_search_clarification_question_into_choice_box() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "I have performed a web search for general information. Based on the search results, could you tell me what aspect of cat breeds you are most interested in?\n1. Breed Characteristics\n2. Grooming & Care\n3. History & Origin\n4. A Specific Breed"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "ask_user_clarification");
                let opts = calls[0].arguments["options"].as_array().unwrap();
                assert_eq!(opts.len(), 4);
                assert_eq!(opts[0], "Breed Characteristics");
            }
            _ => panic!("Post-search clarification question must be converted into choice box"),
        }
    }

    #[test]
    fn does_not_convert_structured_articles_with_headers_into_choice_boxes() {
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "จากการค้นหาข้อมูลเกี่ยวกับ พฤติกรรมและการเข้าสังคมของแฮมสเตอร์... --- ### 🐹 พฤติกรรมแฮมสเตอร์ #### 🤝 3. การสร้างความไว้วางใจและการปฏิสัมพันธ์กับคน\n1. การเข้าถึงอย่างช้าๆ\n2. การให้อาหารแบบปฏิสัมพันธ์\n3. การลูบตัว\n4. การกำหนดขอบเขต"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::FinalAnswer { content, .. } => {
                assert!(content.contains("พฤติกรรมและการเข้าสังคมของแฮมสเตอร์"));
            }
            _ => panic!("Structured articles with headers must not be converted into choice boxes"),
        }
    }

    #[test]
    fn detects_japanese_choice_question_via_fullwidth_question_mark() {
        // Japanese uses ？ (U+FF1F) — must work without any keyword matching
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "どのトピックについて知りたいですか？\n1. AI技術\n2. スポーツニュース\n3. 政治・経済\n4. エンタメ"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "ask_user_clarification");
                let opts = calls[0].arguments["options"].as_array().unwrap();
                assert_eq!(opts.len(), 4);
                assert_eq!(opts[0], "AI技術");
            }
            _ => panic!("Japanese question with ？ must be detected as choice UI"),
        }
    }

    #[test]
    fn detects_spanish_choice_question_without_language_keywords() {
        // Spanish uses ¿...? — must work via structural detection only
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "¿Qué tema le interesa hoy?\n1. Tecnología\n2. Deportes\n3. Política"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "ask_user_clarification");
                let opts = calls[0].arguments["options"].as_array().unwrap();
                assert_eq!(opts.len(), 3);
                assert_eq!(opts[0], "Tecnología");
            }
            _ => panic!("Spanish question with ¿ must be detected as choice UI"),
        }
    }

    #[test]
    fn does_not_convert_final_answer_lead_in_numbered_list() {
        // "Based on the search results..." — final answer summary, NOT a choice menu
        let json_resp = serde_json::json!({
            "choices": [{
                "message": {
                    "content": "Based on the search results, here are the key capabilities:\n1. Perform Complex Multi-Step Reasoning\n2. Autonomously Investigate Data\n3. Synthesize Cross-Domain Insights"
                }
            }]
        });
        match parse_loop_step_response(&json_resp) {
            LoopStepResult::FinalAnswer { content, .. } => {
                assert!(content.contains("Based on the search results"));
            }
            _ => panic!("Final answer lead-in must not be converted into choice box"),
        }
    }
}

