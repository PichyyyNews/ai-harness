use super::query::RoutingDecision;
use chrono::Utc;
use serde::Serialize;
use std::{fs::OpenOptions, io::Write};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RouteEvent<'a> {
    timestamp_utc: String,
    event: &'static str,
    query: &'a str,
    decision: &'static str,
    reason: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RetrievalEvent<'a> {
    timestamp_utc: String,
    event: &'static str,
    query: &'a str,
    provider: &'a str,
    result_count: usize,
    top_results: Vec<DebugResult<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DebugResult<'a> {
    title: &'a str,
    url: &'a str,
    relevance_score: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptEvent<'a> {
    timestamp_utc: String,
    event: &'static str,
    session_id: Option<&'a str>,
    grounding_prompt: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tier0Event<'a> {
    timestamp_utc: String,
    event: &'static str,
    query: &'a str,
    greeting_score: f32,
    constraint_score: f32,
    decision: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticEvent<'a> {
    timestamp_utc: String,
    event: &'static str,
    query: &'a str,
    component: &'a str,
    detail: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderPlanEvent<'a> {
    timestamp_utc: String,
    event: &'static str,
    query: &'a str,
    candidates: &'a [(String, f32)],
}

pub fn log_route(app: &AppHandle, raw_query: &str, decision: &RoutingDecision) {
    let (decision, reason) = match decision {
        RoutingDecision::Search { reason, .. } => ("search", *reason),
        RoutingDecision::Skip { reason } => ("skip", *reason),
    };
    append(
        app,
        &RouteEvent {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "routing",
            query: raw_query,
            decision,
            reason,
        },
    );
}

pub fn log_retrieval<'a>(
    app: &AppHandle,
    query: &str,
    provider: &str,
    result_count: usize,
    top_results: impl Iterator<Item = (&'a str, &'a str, Option<f64>)>,
) {
    let top_results = top_results
        .take(3)
        .map(|(title, url, relevance_score)| DebugResult {
            title,
            url,
            relevance_score,
        })
        .collect();
    append(
        app,
        &RetrievalEvent {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "retrieval",
            query,
            provider,
            result_count,
            top_results,
        },
    );
}

/// Captures the final, context-budgeted grounding block immediately before the
/// request is sent to llama-server. It intentionally excludes conversation and
/// memory messages from the debug log.
pub fn log_assembled_prompt(app: &AppHandle, session_id: Option<&str>, grounding_prompt: &str) {
    append(
        app,
        &PromptEvent {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "assembled_grounding_prompt",
            session_id,
            grounding_prompt,
        },
    );
}

pub fn log_tier0(
    app: &AppHandle,
    query: &str,
    greeting_score: f32,
    constraint_score: f32,
    decision: &str,
) {
    append(
        app,
        &Tier0Event {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "tier0_embedding_classification",
            query,
            greeting_score,
            constraint_score,
            decision,
        },
    );
}

pub fn log_tier0_error(app: &AppHandle, query: &str, error: &str) {
    append(
        app,
        &DiagnosticEvent {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "tier0_embedding_error",
            query,
            component: "embedding_classifier",
            detail: error,
        },
    );
}

pub fn log_provider_plan(app: &AppHandle, query: &str, candidates: &[(String, f32)]) {
    append(
        app,
        &ProviderPlanEvent {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "tier0_provider_plan",
            query,
            candidates,
        },
    );
}

pub fn log_provider_error(app: &AppHandle, query: &str, provider: &str, error: &str) {
    append(
        app,
        &DiagnosticEvent {
            timestamp_utc: Utc::now().to_rfc3339(),
            event: "provider_error",
            query,
            component: provider,
            detail: error,
        },
    );
}

fn append(app: &AppHandle, event: &impl Serialize) {
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    let Ok(directory) = app.path().app_data_dir().map(|path| path.join("logs")) else {
        return;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("web-search-debug.jsonl"))
    {
        let _ = writeln!(file, "{line}");
    }
}
