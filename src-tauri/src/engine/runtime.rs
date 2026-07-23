use super::{
    hardware,
    repetition_guard::RepetitionGuard,
    runtime_manager,
    settings::{BackendPreference, EngineSettings},
};
use crate::web_search::{RetrievalTraceEntry, WebSource};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub interaction_id: Option<String>,
    #[serde(default)]
    pub interaction_option_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing, default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    RepetitionDetected,
    Cancelled,
}

impl FinishReason {
    fn from_server(reason: &str) -> Self {
        match reason {
            "length" => Self::Length,
            "cancelled" => Self::Cancelled,
            _ => Self::Stop,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub backend: BackendPreference,
    pub gpu_layers: i32,
    pub context_size: u32,
    pub runtime_release: String,
    pub fallback_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationResult {
    pub content: String,
    pub finish_reason: FinishReason,
    #[serde(default)]
    pub sources: Vec<WebSource>,
    #[serde(default)]
    pub retrieval_trace: Vec<RetrievalTraceEntry>,
}

#[derive(Debug, Clone)]
pub enum GenerationEvent {
    Token(String),
    TrimSuffix(String),
    Status(String),
}

pub struct Engine {
    process: Child,
    endpoint: String,
    info: EngineInfo,
    model_label: String,
    event_log: PathBuf,
}

const MIN_CONTEXT_SIZE: u32 = 4096;
const MAX_CONTEXT_SIZE: u32 = 16_384;
const DEFAULT_MAX_TOKENS: u32 = 1536;
const VRAM_RESERVE_MIB: u64 = 768;

#[derive(Debug, Clone, Copy)]
pub struct Sampling {
    temperature: f32,
    repeat_penalty: f32,
    repeat_last_n: u32,
    dry_multiplier: f32,
    min_p: f32,
}

impl Sampling {
    fn from_request(request: &ChatRequest) -> Self {
        Self {
            temperature: request.temperature.unwrap_or(0.75).clamp(0.0, 2.0),
            repeat_penalty: 1.08,
            repeat_last_n: 128,
            dry_multiplier: 0.8,
            min_p: 0.05,
        }
    }

    fn for_retry(self) -> Self {
        Self {
            temperature: self.temperature.min(0.55),
            repeat_penalty: self.repeat_penalty.max(1.18),
            repeat_last_n: self.repeat_last_n.max(256),
            dry_multiplier: self.dry_multiplier.max(1.2),
            min_p: self.min_p.max(0.08),
        }
    }
}

fn reserve_local_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("Could not reserve a local inference port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("Could not read the local inference port: {error}"))
}

fn wait_for_server(process: &mut Child, endpoint: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if let Some(status) = process
            .try_wait()
            .map_err(|error| format!("Could not inspect llama-server: {error}"))?
        {
            return Err(format!(
                "llama-server exited before it was ready (exit code {:?}).",
                status.code()
            ));
        }
        if client
            .get(format!("{endpoint}/health"))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    Err(
        "llama-server did not become ready within two minutes. Try CPU mode or reduce GPU offload."
            .to_string(),
    )
}

fn max_gpu_layers_argument(
    backend: BackendPreference,
    requested_layers: i32,
    model_path: &Path,
    profile: &hardware::HardwareProfile,
) -> String {
    if backend == BackendPreference::Cpu {
        return "0".to_string();
    }
    if requested_layers >= 0 {
        return requested_layers.to_string();
    }
    let Some(total) = profile.vram_total_mib else {
        return "auto".to_string();
    };
    let available = total.saturating_sub(profile.vram_used_mib.unwrap_or(0));
    let model_mib = std::fs::metadata(model_path)
        .map(|metadata| metadata.len() / 1_048_576)
        .unwrap_or(0);
    let full_model_budget = model_mib.saturating_mul(112) / 100 + 1_024 + VRAM_RESERVE_MIB;
    if available >= full_model_budget {
        "all".to_string()
    } else {
        "auto".to_string()
    }
}

/// Context is a KV-cache allocation, not a fixed quality setting. Pick the
/// largest conservative tier that leaves room for both the loaded GGUF and the
/// rest of the desktop, then let llama-server's `--fit` safety mechanism make
/// the final GPU-layer trade-off on machines with less free VRAM.
fn adaptive_context_size(
    model_path: &Path,
    backend: BackendPreference,
    profile: &hardware::HardwareProfile,
) -> u32 {
    let model_mib = std::fs::metadata(model_path)
        .map(|metadata| metadata.len() / 1_048_576)
        .unwrap_or(0);
    let system_headroom = hardware::available_system_memory_mib()
        .unwrap_or(8_192)
        .saturating_sub(model_mib / 2);
    let from_system = match system_headroom {
        value if value >= 12_288 => 16_384,
        value if value >= 6_144 => 8_192,
        _ => MIN_CONTEXT_SIZE,
    };
    if backend == BackendPreference::Cpu {
        return from_system;
    }

    let available_vram = profile
        .vram_total_mib
        .unwrap_or(0)
        .saturating_sub(profile.vram_used_mib.unwrap_or(0));
    // Account for the quantized model plus backend overhead before spending
    // remaining VRAM on the KV cache.
    let kv_headroom = available_vram.saturating_sub(model_mib.saturating_mul(112) / 100);
    let from_gpu = match kv_headroom {
        value if value >= 8_192 => 16_384,
        value if value >= 2_560 => 8_192,
        _ => MIN_CONTEXT_SIZE,
    };
    from_system
        .min(from_gpu)
        .clamp(MIN_CONTEXT_SIZE, MAX_CONTEXT_SIZE)
}

fn build_engine(
    model_path: &Path,
    backend_choice: BackendPreference,
    requested_layers: i32,
    cache_dir: &Path,
    profile: &hardware::HardwareProfile,
) -> Result<Engine, String> {
    let runtime = runtime_manager::ensure(backend_choice, cache_dir)?;
    let port = reserve_local_port()?;
    let endpoint = format!("http://127.0.0.1:{port}");
    let layer_argument =
        max_gpu_layers_argument(backend_choice, requested_layers, model_path, profile);
    let context_size = adaptive_context_size(model_path, backend_choice, profile);
    let before_vram = if backend_choice == BackendPreference::Cuda {
        hardware::nvidia_vram_used_mib()
    } else {
        None
    };
    let mut process = Command::new(&runtime.server);
    process
        .args([
            "-m",
            &model_path.to_string_lossy(),
            "--port",
            &port.to_string(),
            "--ctx-size",
            &context_size.to_string(),
            "--n-gpu-layers",
            &layer_argument,
            "--fit",
            "on",
            "--fit-target",
            &VRAM_RESERVE_MIB.to_string(),
            "--fit-ctx",
            &context_size.to_string(),
            "--flash-attn",
            "auto",
            "--parallel",
            "1",
        ])
        // Use the model's embedded Jinja template and EOS handling. We never
        // construct prompt markers in the application itself.
        .arg("--jinja")
        // Repetition-resistant defaults: mild token penalty plus DRY sequence
        // sampling. They are engine-level so every request gets the same guard.
        .args([
            "--samplers",
            "top_k;top_p;min_p;temperature;dry;typ_p;xtc",
            "--repeat-penalty",
            "1.08",
            "--repeat-last-n",
            "128",
            "--dry-multiplier",
            "0.8",
            "--dry-allowed-length",
            "2",
            "--dry-sequence-breaker",
            "\n",
            "--dry-sequence-breaker",
            ":",
            "--dry-sequence-breaker",
            "\"",
            "--dry-sequence-breaker",
            "*",
            "--min-p",
            "0.05",
            "--temp",
            "0.75",
        ])
        .current_dir(&runtime.directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut process = process
        .spawn()
        .map_err(|error| format!("Could not start llama-server: {error}"))?;
    if let Err(error) = wait_for_server(&mut process, &endpoint) {
        let _ = process.kill();
        return Err(error);
    }
    if backend_choice == BackendPreference::Cuda {
        let after_vram = hardware::nvidia_vram_used_mib();
        if !matches!((before_vram, after_vram), (Some(before), Some(after)) if after.saturating_sub(before) >= 256)
        {
            let _ = process.kill();
            return Err("CUDA runtime started but did not reserve meaningful VRAM. Close GPU-heavy applications or choose CPU; AI Harness will not label this as GPU acceleration.".to_string());
        }
    }
    let effective_layers = if backend_choice == BackendPreference::Cpu {
        0
    } else if layer_argument == "auto" || layer_argument == "all" {
        -1
    } else {
        requested_layers
    };
    let mut engine = Engine {
        process,
        endpoint,
        info: EngineInfo {
            backend: backend_choice,
            gpu_layers: effective_layers,
            context_size,
            runtime_release: runtime.release,
            fallback_reason: None,
        },
        model_label: model_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("local-model")
            .to_string(),
        event_log: cache_dir.join("generation-quality-events.jsonl"),
    };
    engine.verify_chat_template();
    Ok(engine)
}

impl Engine {
    pub fn start(
        model_path: &Path,
        settings: EngineSettings,
        cache_dir: &Path,
    ) -> Result<Self, String> {
        let detected = hardware::detect();
        let desired = if settings.backend == BackendPreference::Auto {
            detected.recommended_backend
        } else {
            settings.backend
        };
        let layers = if desired == BackendPreference::Cpu {
            0
        } else {
            settings.gpu_layers
        };
        match build_engine(model_path, desired, layers, cache_dir, &detected) {
            Ok(engine) => Ok(engine),
            Err(error)
                if settings.backend == BackendPreference::Auto
                    && desired != BackendPreference::Cpu =>
            {
                let mut fallback =
                    build_engine(model_path, BackendPreference::Cpu, 0, cache_dir, &detected)?;
                fallback.info.fallback_reason =
                    Some(format!("{error} CPU fallback was loaded instead."));
                Ok(fallback)
            }
            Err(error) => Err(error),
        }
    }

    pub fn info(&self) -> EngineInfo {
        self.info.clone()
    }
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
    pub fn context_size(&self) -> u32 {
        self.info.context_size
    }

    pub fn count_message_tokens(&self, message: &ChatMessage) -> u32 {
        self.tokenize(&message.content)
            .unwrap_or_else(|| estimate_tokens(&message.content))
            .saturating_add(10)
    }

    pub fn count_messages_tokens(&self, messages: &[ChatMessage]) -> u32 {
        messages
            .iter()
            .map(|message| self.count_message_tokens(message))
            .sum::<u32>()
            .saturating_add(8)
    }

    pub fn generate<F>(
        &mut self,
        request: ChatRequest,
        mut emit: F,
        should_cancel: impl Fn() -> bool,
    ) -> Result<GenerationResult, String>
    where
        F: FnMut(GenerationEvent) -> Result<(), String>,
    {
        let initial_sampling = Sampling::from_request(&request);
        let first = self.generate_attempt(
            request.clone(),
            &mut emit,
            &should_cancel,
            0,
            initial_sampling,
        )?;
        if first.finish_reason != FinishReason::RepetitionDetected
            || should_cancel()
        {
            return Ok(first);
        }

        // A model can still enter a genuine loop late in an otherwise useful
        // answer. Remove that first draft and retry once with stronger
        // repetition controls instead of exposing a loop warning to the user.
        if !first.content.is_empty() {
            emit(GenerationEvent::TrimSuffix(first.content))?;
        }
        let retry_sampling = initial_sampling.for_retry();
        let mut retry_request = request;
        retry_request.temperature = Some(retry_sampling.temperature);
        self.generate_attempt(
            retry_request,
            &mut emit,
            &should_cancel,
            1,
            retry_sampling,
        )
    }

    fn generate_attempt<F, C>(
        &mut self,
        request: ChatRequest,
        emit: &mut F,
        should_cancel: &C,
        retry_attempt: usize,
        sampling: Sampling,
    ) -> Result<GenerationResult, String>
    where
        F: FnMut(GenerationEvent) -> Result<(), String>,
        C: Fn() -> bool,
    {
        let payload = serde_json::json!({
            "messages": request.messages,
            "max_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS).clamp(1, DEFAULT_MAX_TOKENS),
            "temperature": sampling.temperature,
            "repeat_penalty": sampling.repeat_penalty,
            "repeat_last_n": sampling.repeat_last_n,
            "dry_multiplier": sampling.dry_multiplier,
            "min_p": sampling.min_p,
            "stream": true,
            // The Gemma model otherwise spends its output budget in a hidden
            // reasoning stream before answering. Chat responses need direct
            // user-visible text; background classifiers already use this flag.
            "chat_template_kwargs": {"enable_thinking": false},
        });
        let response = reqwest::blocking::Client::new()
            .post(format!("{}/v1/chat/completions", self.endpoint))
            .json(&payload)
            .send()
            .map_err(|error| format!("Could not send the message to llama-server: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "llama-server rejected the message (HTTP {}).",
                response.status()
            ));
        }
        let reader = BufReader::new(response);
        let mut output = String::new();
        let mut finish_reason = FinishReason::Stop;
        let mut guard = RepetitionGuard::default();
        for line in reader.lines() {
            if should_cancel() {
                finish_reason = FinishReason::Cancelled;
                break;
            }
            let line = line.map_err(|error| format!("Could not read generated text: {error}"))?;
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }
            let value: serde_json::Value = serde_json::from_str(data)
                .map_err(|error| format!("Could not parse generated text: {error}"))?;
            if let Some(reason) = value
                .pointer("/choices/0/finish_reason")
                .and_then(serde_json::Value::as_str)
            {
                finish_reason = FinishReason::from_server(reason);
            }
            if let Some(piece) = value
                .pointer("/choices/0/delta/content")
                .and_then(serde_json::Value::as_str)
            {
                if let Some(detection) = guard.observe(piece) {
                    if !detection.emitted_suffix.is_empty() {
                        output
                            .truncate(output.len().saturating_sub(detection.emitted_suffix.len()));
                        emit(GenerationEvent::TrimSuffix(detection.emitted_suffix))?;
                    }
                    finish_reason = FinishReason::RepetitionDetected;
                    self.log_repetition_abort(retry_attempt, detection.position, sampling);
                    break;
                }
                output.push_str(piece);
                emit(GenerationEvent::Token(piece.to_string()))?;
            }
        }
        Ok(GenerationResult {
            content: output,
            finish_reason,
            sources: Vec::new(),
            retrieval_trace: Vec::new(),
        })
    }

    pub fn log_repetition_abort(
        &self,
        retry_attempt: usize,
        position: usize,
        sampling: Sampling,
    ) {
        let entry = serde_json::json!({
            "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).map(|value| value.as_secs()).unwrap_or_default(),
            "model": self.model_label,
            "event": "repetition_detected",
            "position": position,
            "retry_attempt": retry_attempt,
            "sampling": { "repeat_penalty": sampling.repeat_penalty, "repeat_last_n": sampling.repeat_last_n, "dry_multiplier": sampling.dry_multiplier, "min_p": sampling.min_p, "temperature": sampling.temperature },
        });
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.event_log)
        {
            let _ = writeln!(file, "{entry}");
        }
    }

    fn tokenize(&self, content: &str) -> Option<u32> {
        #[derive(Deserialize)]
        struct TokenizeResponse {
            tokens: Vec<i64>,
        }
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .ok()?
            .post(format!("{}/tokenize", self.endpoint))
            .json(&serde_json::json!({ "content": content, "add_special": false }))
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .json::<TokenizeResponse>()
            .ok()
            .map(|response| response.tokens.len() as u32)
    }

    fn verify_chat_template(&mut self) {
        let probe = ChatRequest {
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Reply with exactly: OK".to_string(),
                created_at: None,
            }],
            max_tokens: Some(24),
            temperature: Some(0.2),
            session_id: None,
            interaction_id: None,
            interaction_option_id: None,
        };
        match self.generate(probe, |_| Ok(()), || false) {
            Ok(result) if result.finish_reason == FinishReason::Stop => {}
            Ok(_) => self.info.fallback_reason = Some("The model's chat-template probe did not stop naturally. Chat can still run, but this model may not be chat-tuned.".to_string()),
            Err(_) => self.info.fallback_reason = Some("The model did not pass the chat-template probe. Chat can still run, but watch for malformed or repetitive replies.".to_string()),
        }
    }
}

fn estimate_tokens(content: &str) -> u32 {
    ((content.chars().count() as u32).saturating_add(3) / 4).max(1)
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
