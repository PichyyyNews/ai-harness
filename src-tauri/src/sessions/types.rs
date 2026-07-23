use serde::Serialize;
use crate::web_search::{RetrievalTraceEntry, WebSource};

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInteraction {
    pub id: String,
    pub session_id: String,
    pub request_content: String,
    pub question: String,
    pub options: Vec<InteractionOption>,
    pub reason: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub thinking_summary: Option<String>,
    pub finish_reason: Option<String>,
    pub web_sources: Option<Vec<WebSource>>,
    pub retrieval_trace: Option<Vec<RetrievalTraceEntry>>,
    pub created_at: String,
    pub sequence: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionSummary,
    pub messages: Vec<SessionMessage>,
    pub conversation_memory: String,
}
