use super::runtime_manager;
use crate::{models::paths, web_search::ProviderKind};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

const MODEL_REPOSITORY: &str = "ggml-org/embeddinggemma-300M-GGUF";
const MODEL_FILE: &str = "embeddinggemma-300M-Q8_0.gguf";
const MODEL_SHA256: &str = "b5ce9d77a3fc4b3b39ccb5643c36777911cc4eb46a66962eadfa3f5f60490d63";
// Calibrated against the bundled Q8 model with Thai, Japanese, Arabic and
// Spanish probes. Constraint alignment is materially lower than greetings,
// so a single guessed threshold made Tier 0 miss most non-English rules.
const GREETING_HIGH_CONFIDENCE: f32 = 0.80;
const CONSTRAINT_HIGH_CONFIDENCE: f32 = 0.67;
const CLASS_MARGIN: f32 = 0.08;
const AMBIGUOUS_FLOOR: f32 = 0.52;

const GREETING_PROTOTYPES: &[&str] = &["hello", "hi", "thanks", "ok", "goodbye", "got it"];
const CONSTRAINT_PROTOTYPES: &[&str] = &[
    "don't do that",
    "always respond briefly",
    "never use emojis",
    "from now on do X",
    "please stop doing Y",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier0Intent {
    NoSearch,
    SessionConstraint,
}

#[derive(Debug, Clone, Copy)]
pub struct Tier0Classification {
    pub greeting_score: f32,
    pub constraint_score: f32,
    pub intent: Option<Tier0Intent>,
}

#[derive(Debug, Clone)]
pub struct Tier0Analysis {
    pub classification: Tier0Classification,
    pub provider_candidates: Vec<(ProviderKind, f32)>,
}

pub struct EmbeddingRuntime {
    process: Child,
    endpoint: String,
    greeting_vectors: Vec<Vec<f32>>,
    constraint_vectors: Vec<Vec<f32>>,
    provider_vectors: Vec<(ProviderKind, Vec<f32>)>,
}

impl EmbeddingRuntime {
    pub fn start(app: &AppHandle, cache_dir: &Path) -> Result<Self, String> {
        let model_path = ensure_model(app)?;
        let runtime = runtime_manager::ensure(super::settings::BackendPreference::Cpu, cache_dir)?;
        let port = reserve_local_port()?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let log_directory = app
            .path()
            .app_data_dir()
            .map_err(|error| format!("Could not resolve the embedding log directory: {error}"))?
            .join("logs");
        std::fs::create_dir_all(&log_directory)
            .map_err(|error| format!("Could not create the embedding log directory: {error}"))?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_directory.join("embedding-runtime.log"))
            .map_err(|error| format!("Could not open the embedding runtime log: {error}"))?;
        let stdout = log
            .try_clone()
            .map_err(|error| format!("Could not clone the embedding runtime log: {error}"))?;
        let mut process = Command::new(&runtime.server)
            .args([
                "-m",
                &model_path.to_string_lossy(),
                "--port",
                &port.to_string(),
                "--ctx-size",
                "2048",
                "--n-gpu-layers",
                "0",
                "--embeddings",
                "--batch-size",
                "512",
                "--parallel",
                "1",
            ])
            .current_dir(&runtime.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|error| format!("Could not start the embedding sidecar: {error}"))?;
        if let Err(error) = wait_for_server(&mut process, &endpoint) {
            let _ = process.kill();
            return Err(error);
        }
        let cached_vectors = (|| -> Result<_, String> {
            let greeting_vectors = embed_many(&endpoint, GREETING_PROTOTYPES)?;
            let constraint_vectors = embed_many(&endpoint, CONSTRAINT_PROTOTYPES)?;
            let providers = provider_prototypes();
            let descriptions = providers
                .iter()
                .map(|(_, description)| *description)
                .collect::<Vec<_>>();
            let vectors = embed_many(&endpoint, &descriptions)?;
            let provider_vectors = providers
                .into_iter()
                .zip(vectors)
                .map(|((provider, _), vector)| (provider, vector))
                .collect();
            Ok((greeting_vectors, constraint_vectors, provider_vectors))
        })();
        let (greeting_vectors, constraint_vectors, provider_vectors) = match cached_vectors {
            Ok(vectors) => vectors,
            Err(error) => {
                let _ = process.kill();
                let _ = process.wait();
                return Err(format!(
                    "Could not initialize embedding prototypes: {error}"
                ));
            }
        };
        Ok(Self {
            process,
            endpoint,
            greeting_vectors,
            constraint_vectors,
            provider_vectors,
        })
    }

    pub fn analyze(&self, message: &str) -> Result<Tier0Analysis, String> {
        let embedding = embed(&self.endpoint, message)?;
        let greeting_score = max_similarity(&embedding, &self.greeting_vectors);
        let constraint_score = max_similarity(&embedding, &self.constraint_vectors);
        let intent = select_intent(greeting_score, constraint_score);
        let mut scored_providers = self
            .provider_vectors
            .iter()
            .map(|(provider, vector)| (*provider, cosine_similarity(&embedding, vector)))
            .collect::<Vec<_>>();
        scored_providers.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let best = scored_providers
            .first()
            .map(|(_, score)| *score)
            .unwrap_or(-1.0);
        let provider_candidates = scored_providers
            .into_iter()
            .filter(|(_, score)| best >= 0.30 && *score >= 0.28 && best - *score <= 0.12)
            .take(2)
            .collect();
        Ok(Tier0Analysis {
            classification: Tier0Classification {
                greeting_score,
                constraint_score,
                intent,
            },
            provider_candidates,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn is_ambiguous(result: &Tier0Classification) -> bool {
        result.intent.is_none()
            && result.greeting_score.max(result.constraint_score) >= AMBIGUOUS_FLOOR
    }
}

fn select_intent(greeting_score: f32, constraint_score: f32) -> Option<Tier0Intent> {
    if greeting_score >= GREETING_HIGH_CONFIDENCE
        && greeting_score - constraint_score >= CLASS_MARGIN
    {
        Some(Tier0Intent::NoSearch)
    } else if constraint_score >= CONSTRAINT_HIGH_CONFIDENCE
        && constraint_score - greeting_score >= CLASS_MARGIN
    {
        Some(Tier0Intent::SessionConstraint)
    } else {
        None
    }
}

impl Drop for EmbeddingRuntime {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn ensure_model(app: &AppHandle) -> Result<PathBuf, String> {
    let target = paths::models_directory(app)?.join(MODEL_FILE);
    if target.is_file() {
        return Ok(target);
    }
    let partial = target.with_extension("part");
    let url = format!(
        "https://huggingface.co/{MODEL_REPOSITORY}/resolve/main/{MODEL_FILE}?download=true"
    );
    let mut response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .map_err(|error| format!("Could not create embedding-model client: {error}"))?
        .get(url)
        .send()
        .map_err(|error| format!("Could not download the embedding model: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Embedding-model download failed: {error}"))?;
    let mut output = File::create(&partial)
        .map_err(|error| format!("Could not create embedding-model download: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("Could not read embedding-model download: {error}"))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("Could not write embedding-model download: {error}"))?;
        hash.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hash.finalize());
    if actual != MODEL_SHA256 {
        let _ = std::fs::remove_file(&partial);
        return Err("Embedding-model checksum verification failed.".to_string());
    }
    std::fs::rename(&partial, &target)
        .map_err(|error| format!("Could not finalize embedding-model download: {error}"))?;
    Ok(target)
}

fn reserve_local_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("Could not reserve an embedding port: {error}"))?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("Could not read the embedding port: {error}"))
}

fn wait_for_server(process: &mut Child, endpoint: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if process
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("The embedding sidecar exited before becoming ready.".to_string());
        }
        if client
            .get(format!("{endpoint}/health"))
            .send()
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("The embedding sidecar did not become ready within two minutes.".to_string())
}

fn embed_many(endpoint: &str, values: &[&str]) -> Result<Vec<Vec<f32>>, String> {
    let values = values
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    embed_texts(endpoint, &values)
}

fn embed(endpoint: &str, input: &str) -> Result<Vec<f32>, String> {
    embed_texts(endpoint, &[input.to_string()])?
        .into_iter()
        .next()
        .ok_or_else(|| "The embedding server returned no vector.".to_string())
}

#[derive(serde::Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingDatum>,
}

#[derive(serde::Deserialize)]
struct EmbeddingDatum {
    index: usize,
    embedding: Vec<f32>,
}

pub fn embed_texts(endpoint: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;

    // llama.cpp accepts arrays for small prototype batches, but a mixed batch
    // of scraped passages can exceed its physical prompt batch and return
    // HTTP 500. Keep each semantic document independent; the shared client
    // reuses the localhost connection and preserves deterministic ordering.
    let mut vectors = Vec::with_capacity(inputs.len());
    for input in inputs {
        let mut response = client
            .post(format!("{endpoint}/v1/embeddings"))
            .json(&serde_json::json!({"input": input, "model": "embeddinggemma"}))
            .send()
            .map_err(|error| error.to_string())?
            .error_for_status()
            .map_err(|error| error.to_string())?
            .json::<EmbeddingResponse>()
            .map_err(|error| error.to_string())?;
        response.data.sort_by_key(|item| item.index);
        let vector = response
            .data
            .into_iter()
            .next()
            .map(|item| item.embedding)
            .filter(|embedding| !embedding.is_empty())
            .ok_or_else(|| "The embedding server returned no vector.".to_string())?;
        vectors.push(vector);
    }
    Ok(vectors)
}

/// EmbeddingGemma uses asymmetric prompts for information retrieval. Keeping
/// this formatting here prevents callers from accidentally comparing two
/// query vectors or two untyped raw strings.
pub fn embed_retrieval(
    endpoint: &str,
    query: &str,
    documents: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let mut inputs = Vec::with_capacity(documents.len() + 1);
    inputs.push(format!(
        "task: search result | query: {}",
        query.chars().take(384).collect::<String>()
    ));
    inputs.extend(documents.iter().map(|document| {
        format!(
            "title: none | text: {}",
            document.chars().take(384).collect::<String>()
        )
    }));
    embed_texts(endpoint, &inputs)
}

pub fn embed_sentence_similarity(
    endpoint: &str,
    inputs: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let formatted = inputs
        .iter()
        .map(|input| {
            format!(
                "task: sentence similarity | query: {}",
                input.chars().take(384).collect::<String>()
            )
        })
        .collect::<Vec<_>>();
    embed_texts(endpoint, &formatted)
}

fn max_similarity(embedding: &[f32], prototypes: &[Vec<f32>]) -> f32 {
    prototypes
        .iter()
        .map(|prototype| cosine_similarity(embedding, prototype))
        .fold(-1.0_f32, f32::max)
}

pub(crate) fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return -1.0;
    }
    let (dot, left_norm, right_norm) = left.iter().zip(right).fold(
        (0.0_f32, 0.0_f32, 0.0_f32),
        |(dot, left_norm, right_norm), (left, right)| {
            (
                dot + left * right,
                left_norm + left * left,
                right_norm + right * right,
            )
        },
    );
    dot / (left_norm.sqrt() * right_norm.sqrt()).max(f32::EPSILON)
}

fn provider_prototypes() -> [(ProviderKind, &'static str); 14] {
    [
        (
            ProviderKind::Wikipedia,
            "encyclopedia explanation and established general knowledge",
        ),
        (
            ProviderKind::Wikidata,
            "entity identity structured facts and identifiers",
        ),
        (
            ProviderKind::Arxiv,
            "academic preprint and scientific research paper",
        ),
        (
            ProviderKind::SemanticScholar,
            "scholarly paper literature and citations",
        ),
        (
            ProviderKind::CoinGecko,
            "cryptocurrency price and market data",
        ),
        (
            ProviderKind::OpenMeteo,
            "current weather and forecast for a place",
        ),
        (
            ProviderKind::OpenStreetMap,
            "place address map coordinates and location",
        ),
        (
            ProviderKind::GitHub,
            "software repository source code and project",
        ),
        (
            ProviderKind::StackExchange,
            "programming error question and developer answer",
        ),
        (
            ProviderKind::Nvd,
            "software vulnerability CVE and security advisory",
        ),
        (
            ProviderKind::RestCountries,
            "country capital population and geography",
        ),
        (
            ProviderKind::ExchangeRate,
            "currency conversion and exchange rate",
        ),
        (
            ProviderKind::GoogleNews,
            "latest news today current events and breaking headlines",
        ),
        (
            ProviderKind::GeneralWeb,
            "general web search for factual information",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_cosine_similarity_for_normalized_embeddings() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 0.0001);
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 0.0001);
    }

    #[test]
    fn reserves_the_middle_similarity_band_for_tier_one() {
        assert_eq!(select_intent(0.91, 0.31), Some(Tier0Intent::NoSearch));
        assert_eq!(
            select_intent(0.30, 0.69),
            Some(Tier0Intent::SessionConstraint)
        );
        assert_eq!(select_intent(0.79, 0.70), None);
        assert_eq!(select_intent(0.62, 0.68), None);
    }
}
