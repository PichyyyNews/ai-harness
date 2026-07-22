pub mod bm25;
pub mod duckduckgo;
pub mod manager;
pub mod query;
pub mod scraper;
pub mod searxng;

use serde::{Deserialize, Serialize};

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
}
