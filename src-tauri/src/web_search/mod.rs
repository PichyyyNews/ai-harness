pub mod bing_rss;
pub mod bm25;
pub mod brave;
pub mod duckduckgo;
pub mod manager;
pub mod observability;
pub mod orchestrator;
pub mod planner;
pub mod query;
pub mod scraper;
pub mod searxng;
pub mod source_router;
pub mod sources;
pub mod worker_runtime;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSource {
    pub id: usize,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct Grounding {
    pub sources: Vec<WebSource>,
    pub prompt: String,
    pub retrieval_trace: Vec<RetrievalTraceEntry>,
}

/// A user-visible, secret-safe record of the live retrieval pipeline. It
/// contains only public endpoints and result URLs; request headers, API keys
/// and request parameters are deliberately never recorded here.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalTraceEntry {
    pub stage: String,
    pub provider: String,
    pub endpoint: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    pub preview: Option<String>,
    pub score: Option<f64>,
    pub decision: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RetrievalTraceRecorder(Arc<Mutex<Vec<RetrievalTraceEntry>>>);

impl RetrievalTraceRecorder {
    pub fn record(&self, entry: RetrievalTraceEntry) {
        if let Ok(mut entries) = self.0.lock() {
            entries.push(entry);
        }
    }

    pub fn snapshot(&self) -> Vec<RetrievalTraceEntry> {
        self.0
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }
}

/// The only source identities an AI retrieval plan may request. The planner
/// never emits URLs or arbitrary tool names; the runtime validates this enum
/// before any network request is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Wikipedia,
    Wikidata,
    Arxiv,
    SemanticScholar,
    CoinGecko,
    OpenMeteo,
    OpenStreetMap,
    GitHub,
    StackExchange,
    Nvd,
    RestCountries,
    ExchangeRate,
    GoogleNews,
    GeneralWeb,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubQuestion {
    pub id: Uuid,
    pub text: String,
    pub source_hint: SourceHint,
    pub depends_on: Option<Uuid>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SourceHint {
    Wikipedia,
    Weather {
        location_text: String,
    },
    Currency {
        from: String,
        to: String,
    },
    StockOrCrypto {
        ticker: String,
    },
    Sports {
        teams_or_league: String,
    },
    News,
    PackageRegistry {
        ecosystem: Ecosystem,
        package: String,
    },
    GeneralWeb,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Ecosystem {
    Rust,
    Npm,
    PyPI,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct QueryPlan {
    pub original_query: String,
    pub sub_questions: Vec<SubQuestion>,
    pub is_compound: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RawEvidence {
    pub chunks: Vec<EvidenceChunk>,
    pub source_kind: SourceKind,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EvidenceChunk {
    pub text: String,
    pub source_url: String,
    pub source_title: String,
    pub host: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SourceKind {
    Dedicated(String),
    Web,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Confidence {
    pub relevance: f32,
    pub agreement: f32,
    pub coverage: f32,
    pub combined: f32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceQuality {
    Strong,
    Adequate,
    Weak,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubQuestionResult {
    pub sub_q: SubQuestion,
    pub evidence: RawEvidence,
    pub confidence: Confidence,
    pub quality: EvidenceQuality,
}
