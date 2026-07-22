use super::{
    bm25, manager, source_router, sources, ProviderKind, RawEvidence, RetrievalTraceEntry,
    RetrievalTraceRecorder, SubQuestion,
};
use crate::engine::hardware;
use std::{
    collections::HashSet,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use tauri::AppHandle;

pub const MAX_ACTIVE_PROVIDERS: usize = 3;
pub const MAX_FALLBACK_PROVIDERS: usize = 2;
const RETRIEVAL_DEADLINE: Duration = Duration::from_secs(8);
const DEFAULT_RELEVANCE_FLOOR: f32 = 0.28;
const CURRENT_NEWS_RELEVANCE_FLOOR: f32 = 0.28;

/// Executes dedicated API and web workers concurrently. AI plans are always
/// validated against this bounded runtime; no provider can make arbitrary HTTP
/// requests or block the chat indefinitely.
pub fn retrieve(
    app: &AppHandle,
    sub_q: &SubQuestion,
    planned: &[ProviderKind],
    embedding_endpoint: Option<&str>,
    trace: &RetrievalTraceRecorder,
    mut status: impl FnMut(String),
) -> RawEvidence {
    let mut kinds = planned.to_vec();
    if kinds.is_empty() {
        kinds = source_router::candidates(&sub_q.text, &sub_q.source_hint);
    }
    if matches!(&sub_q.source_hint, super::SourceHint::News) {
        // Current-news prompts must not be satisfied by a same-label entity.
        kinds.retain(|kind| *kind != ProviderKind::Wikidata);
        if !kinds.contains(&ProviderKind::GoogleNews) {
            kinds.insert(0, ProviderKind::GoogleNews);
        }
    }
    let mut seen = HashSet::new();
    kinds.retain(|kind| seen.insert(*kind));
    // General web remains a baseline evidence source even when the AI planner
    // selects only structured APIs. Reserve one primary slot for it.
    if !kinds.contains(&ProviderKind::GeneralWeb) {
        if kinds.len() >= MAX_ACTIVE_PROVIDERS {
            kinds.truncate(MAX_ACTIVE_PROVIDERS - 1);
        }
        kinds.push(ProviderKind::GeneralWeb);
    }
    kinds.truncate(MAX_ACTIVE_PROVIDERS + MAX_FALLBACK_PROVIDERS);
    trace.record(RetrievalTraceEntry {
        stage: "retrieval query".to_string(),
        provider: "Retrieval planner".to_string(),
        endpoint: None,
        title: Some(sub_q.text.clone()),
        url: None,
        preview: None,
        score: None,
        decision: "sent unchanged to live providers".to_string(),
        detail: Some(format!("planned providers: {}", provider_list(&kinds))),
    });
    let active_limit = resource_aware_worker_limit();
    let primary = kinds.iter().take(active_limit).copied().collect::<Vec<_>>();
    status(format!(
        "Fetching current information from {}",
        provider_list(&primary)
    ));
    let fallback = kinds
        .iter()
        .skip(active_limit)
        .copied()
        .take(MAX_FALLBACK_PROVIDERS)
        .collect::<Vec<_>>();
    let started = Instant::now();
    let (sender, receiver) = mpsc::channel();
    for kind in kinds.iter().take(active_limit).copied() {
        let app = app.clone();
        let task = sub_q.clone();
        let endpoint = embedding_endpoint.map(ToOwned::to_owned);
        let trace = trace.clone();
        let sender = sender.clone();
        thread::spawn(move || {
            let evidence = match kind {
                ProviderKind::GeneralWeb => {
                    manager::search_and_rank_sub_q(
                        &app,
                        &task.text,
                        6,
                        endpoint.as_deref(),
                        &trace,
                    )
                }
                _ => match sources::fetch_kind(kind, &task) {
                    Ok(evidence) => {
                        record_api_result(&trace, kind, &evidence);
                        evidence
                    }
                    Err(error) => {
                        crate::web_search::observability::log_provider_error(
                            &app,
                            &task.text,
                            provider_label(kind),
                            &error.to_string(),
                        );
                        trace.record(RetrievalTraceEntry {
                            stage: "API provider".to_string(),
                            provider: provider_label(kind).to_string(),
                            endpoint: Some(provider_endpoint(kind).to_string()),
                            title: None,
                            url: None,
                            preview: None,
                            score: None,
                            decision: "failed".to_string(),
                            detail: Some(error.to_string()),
                        });
                        RawEvidence {
                            chunks: Vec::new(),
                            source_kind: super::SourceKind::Dedicated(format!("{kind:?}")),
                        }
                    }
                },
            };
            let _ = sender.send((kind, evidence));
        });
    }
    drop(sender);
    let mut merged = RawEvidence {
        chunks: Vec::new(),
        source_kind: super::SourceKind::Web,
    };
    let mut received = 0;
    while received < primary.len() {
        let remaining = RETRIEVAL_DEADLINE.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        if let Ok((kind, evidence)) = receiver.recv_timeout(remaining) {
            received += 1;
            if kind != ProviderKind::GeneralWeb {
                crate::web_search::observability::log_retrieval(
                    app,
                    &sub_q.text,
                    provider_label(kind),
                    evidence.chunks.len(),
                    evidence.chunks.iter().map(|chunk| {
                        (chunk.source_title.as_str(), chunk.source_url.as_str(), None)
                    }),
                );
            }
            merged.chunks.extend(evidence.chunks);
        } else {
            break;
        }
    }
    if merged.chunks.is_empty() {
        if !fallback.is_empty() {
            status(format!(
                "Primary sources returned no usable evidence; trying {}",
                provider_list(&fallback)
            ));
        }
        for kind in fallback {
            if started.elapsed() >= RETRIEVAL_DEADLINE {
                break;
            }
            let evidence = match kind {
                ProviderKind::GeneralWeb => {
                    manager::search_and_rank_sub_q(
                        app,
                        &sub_q.text,
                        6,
                        embedding_endpoint,
                        trace,
                    )
                }
                _ => match sources::fetch_kind(kind, sub_q) {
                    Ok(evidence) => {
                        record_api_result(trace, kind, &evidence);
                        evidence
                    }
                    Err(error) => {
                        crate::web_search::observability::log_provider_error(
                            app,
                            &sub_q.text,
                            provider_label(kind),
                            &error.to_string(),
                        );
                        trace.record(RetrievalTraceEntry {
                            stage: "API provider".to_string(),
                            provider: provider_label(kind).to_string(),
                            endpoint: Some(provider_endpoint(kind).to_string()),
                            title: None,
                            url: None,
                            preview: None,
                            score: None,
                            decision: "failed".to_string(),
                            detail: Some(error.to_string()),
                        });
                        RawEvidence {
                            chunks: Vec::new(),
                            source_kind: super::SourceKind::Dedicated(format!("{kind:?}")),
                        }
                    }
                },
            };
            crate::web_search::observability::log_retrieval(
                app,
                &sub_q.text,
                provider_label(kind),
                evidence.chunks.len(),
                evidence
                    .chunks
                    .iter()
                    .map(|chunk| (chunk.source_title.as_str(), chunk.source_url.as_str(), None)),
            );
            if !evidence.chunks.is_empty() {
                merged.chunks.extend(evidence.chunks);
                break;
            }
        }
    }
    if let Some(endpoint) = embedding_endpoint {
        let original = std::mem::take(&mut merged.chunks);
        let relevance_floor = if kinds.contains(&ProviderKind::GoogleNews) {
            CURRENT_NEWS_RELEVANCE_FLOOR
        } else {
            DEFAULT_RELEVANCE_FLOOR
        };
        match bm25::rerank_evidence(endpoint, &sub_q.text, original.clone()) {
            Ok(ranked) => {
                let mut selected = Vec::new();
                for item in ranked {
                    let selected_for_context = item.score >= relevance_floor;
                    trace.record(RetrievalTraceEntry {
                        stage: "final relevance filter".to_string(),
                        provider: "Tier 0 embeddings".to_string(),
                        endpoint: None,
                        title: Some(item.chunk.source_title.clone()),
                        url: Some(item.chunk.source_url.clone()),
                        preview: None,
                        score: Some(item.score as f64),
                        decision: if selected_for_context {
                            "kept for answer context".to_string()
                        } else {
                            "discarded as insufficiently relevant".to_string()
                        },
                        detail: Some(format!(
                            "minimum semantic relevance for this retrieval: {:.2}",
                            relevance_floor
                        )),
                    });
                    if selected_for_context {
                        selected.push(item.chunk);
                    }
                }
                merged.chunks = selected;
            }
            Err(error) => {
                crate::web_search::observability::log_provider_error(
                    app,
                    &sub_q.text,
                    "merged_embedding_reranker",
                    &error,
                );
                // A failed embedding runtime must not silently destroy live
                // evidence. The trace records the failure so this fallback is
                // visible instead of looking like a relevance decision.
                merged.chunks = original;
            }
        }
    }
    if merged.chunks.is_empty() {
        status(
            "No live source returned usable text; keeping the answer explicit about that limit"
                .to_string(),
        );
    } else {
        status(format!(
            "Retrieved {} evidence excerpt{} from live sources",
            merged.chunks.len(),
            if merged.chunks.len() == 1 { "" } else { "s" }
        ));
    }
    merged
}

fn record_api_result(
    trace: &RetrievalTraceRecorder,
    provider: ProviderKind,
    evidence: &RawEvidence,
) {
    trace.record(RetrievalTraceEntry {
        stage: "API provider".to_string(),
        provider: provider_label(provider).to_string(),
        endpoint: Some(provider_endpoint(provider).to_string()),
        title: None,
        url: None,
        preview: None,
        score: None,
        decision: if evidence.chunks.is_empty() {
            "returned no usable evidence".to_string()
        } else {
            "returned evidence".to_string()
        },
        detail: Some(format!("{} evidence excerpt(s)", evidence.chunks.len())),
    });
    for chunk in &evidence.chunks {
        trace.record(RetrievalTraceEntry {
            stage: "API result".to_string(),
            provider: provider_label(provider).to_string(),
            endpoint: Some(provider_endpoint(provider).to_string()),
            title: Some(chunk.source_title.clone()),
            url: Some(chunk.source_url.clone()),
            preview: (!chunk.text.trim().is_empty())
                .then(|| chunk.text.chars().take(500).collect()),
            score: None,
            decision: "returned before final reranking".to_string(),
            detail: None,
        });
    }
}

fn provider_endpoint(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Wikipedia => "https://en.wikipedia.org/w/api.php",
        ProviderKind::Wikidata => "https://www.wikidata.org/w/api.php",
        ProviderKind::Arxiv => "https://export.arxiv.org/api/query",
        ProviderKind::SemanticScholar => "https://api.semanticscholar.org/graph/v1/paper/search",
        ProviderKind::CoinGecko => "https://api.coingecko.com/api/v3/simple/price",
        ProviderKind::OpenMeteo => "https://geocoding-api.open-meteo.com/v1/search + https://api.open-meteo.com/v1/forecast",
        ProviderKind::OpenStreetMap => "https://nominatim.openstreetmap.org/search",
        ProviderKind::GitHub => "https://api.github.com/search/repositories",
        ProviderKind::StackExchange => "https://api.stackexchange.com/2.3/search/advanced",
        ProviderKind::Nvd => "https://services.nvd.nist.gov/rest/json/cves/2.0",
        ProviderKind::RestCountries => "https://restcountries.com/v3.1/name/{name}",
        ProviderKind::ExchangeRate => "https://open.er-api.com/v6/latest/{currency}",
        ProviderKind::GoogleNews => "https://news.google.com/rss/search",
        ProviderKind::GeneralWeb => "DuckDuckGo HTML / Brave Search / Bing RSS / SearXNG",
    }
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Wikipedia => "wikipedia",
        ProviderKind::Wikidata => "wikidata",
        ProviderKind::Arxiv => "arxiv",
        ProviderKind::SemanticScholar => "semantic_scholar",
        ProviderKind::CoinGecko => "coingecko",
        ProviderKind::OpenMeteo => "open_meteo",
        ProviderKind::OpenStreetMap => "open_street_map",
        ProviderKind::GitHub => "github",
        ProviderKind::StackExchange => "stack_exchange",
        ProviderKind::Nvd => "nvd",
        ProviderKind::RestCountries => "rest_countries",
        ProviderKind::ExchangeRate => "exchange_rate",
        ProviderKind::GoogleNews => "google_news",
        ProviderKind::GeneralWeb => "general_web",
    }
}

fn provider_list(providers: &[ProviderKind]) -> String {
    let names = providers
        .iter()
        .map(|provider| match provider {
            ProviderKind::Wikipedia => "Wikipedia",
            ProviderKind::Wikidata => "Wikidata",
            ProviderKind::Arxiv => "arXiv",
            ProviderKind::SemanticScholar => "Semantic Scholar",
            ProviderKind::CoinGecko => "CoinGecko",
            ProviderKind::OpenMeteo => "Open-Meteo",
            ProviderKind::OpenStreetMap => "OpenStreetMap",
            ProviderKind::GitHub => "GitHub",
            ProviderKind::StackExchange => "Stack Exchange",
            ProviderKind::Nvd => "NVD",
            ProviderKind::RestCountries => "REST Countries",
            ProviderKind::ExchangeRate => "ExchangeRate API",
            ProviderKind::GoogleNews => "Google News RSS",
            ProviderKind::GeneralWeb => "web search",
        })
        .collect::<Vec<_>>();
    match names.as_slice() {
        [] => "fallback sources".to_string(),
        [only] => (*only).to_string(),
        _ => names.join(" + "),
    }
}

/// API workers do not consume model VRAM, but parsing, scraping and connection
/// buffers do consume RAM. Keep the chat inference lane alone and scale only
/// network concurrency from real headroom. A future planner sidecar can use
/// this same governor before reserving a separate GPU lane.
fn resource_aware_worker_limit() -> usize {
    let ram = hardware::available_system_memory_mib().unwrap_or(8_192);
    let profile = hardware::detect();
    let free_vram = profile
        .vram_total_mib
        .unwrap_or(0)
        .saturating_sub(profile.vram_used_mib.unwrap_or(0));
    if ram < 2_048 || (profile.vram_total_mib.is_some() && free_vram < 768) {
        1
    } else if ram < 4_096 || (profile.vram_total_mib.is_some() && free_vram < 1_536) {
        2
    } else {
        MAX_ACTIVE_PROVIDERS
    }
}
