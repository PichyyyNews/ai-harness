use super::SourceError;
use crate::web_search::{EvidenceChunk, ProviderKind, RawEvidence, SourceKind, SubQuestion};
use reqwest::blocking::Client;
use serde_json::Value;
use std::time::Duration;

pub struct OpenDataProvider;

impl OpenDataProvider {
    pub fn fetch(
        &self,
        kind: ProviderKind,
        sub_q: &SubQuestion,
    ) -> Result<RawEvidence, SourceError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .user_agent("AI Harness retrieval/1.0")
            .build()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
        let query = sub_q.text.trim();
        let (url, title, host, text) = match kind {
            ProviderKind::Wikidata => {
                let v = get_json(
                    &client,
                    "https://www.wikidata.org/w/api.php",
                    &[
                        ("action", "wbsearchentities"),
                        ("search", query),
                        ("language", "en"),
                        ("format", "json"),
                        ("limit", "3"),
                    ],
                )?;
                let rows = v["search"].as_array().ok_or(SourceError::Empty)?;
                let body = rows
                    .iter()
                    .map(|r| {
                        format!(
                            "{} ({}){}",
                            r["label"].as_str().unwrap_or(""),
                            r["id"].as_str().unwrap_or(""),
                            r["description"]
                                .as_str()
                                .map(|d| format!(": {d}"))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "https://www.wikidata.org/w/api.php".to_string(),
                    "Wikidata entity search".to_string(),
                    "wikidata.org".to_string(),
                    body,
                )
            }
            ProviderKind::Arxiv => {
                let xml = client
                    .get("https://export.arxiv.org/api/query")
                    .query(&[
                        ("search_query", format!("all:{query}")),
                        ("start", "0".to_string()),
                        ("max_results", "3".to_string()),
                    ])
                    .send()
                    .map_err(map_error)?
                    .error_for_status()
                    .map_err(map_error)?
                    .text()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
                let compact = xml
                    .split("<entry>")
                    .skip(1)
                    .take(3)
                    .map(|e| strip_xml(e))
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "https://export.arxiv.org/api/query".to_string(),
                    "arXiv search".to_string(),
                    "arxiv.org".to_string(),
                    compact,
                )
            }
            ProviderKind::SemanticScholar => {
                let v = get_json(
                    &client,
                    "https://api.semanticscholar.org/graph/v1/paper/search",
                    &[
                        ("query", query),
                        ("limit", "3"),
                        ("fields", "title,abstract,year,citationCount,url"),
                    ],
                )?;
                let body = v["data"]
                    .as_array()
                    .ok_or(SourceError::Empty)?
                    .iter()
                    .map(|p| {
                        format!(
                            "{} ({}) citations: {}\n{}",
                            p["title"].as_str().unwrap_or(""),
                            p["year"],
                            p["citationCount"],
                            p["abstract"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                (
                    "https://api.semanticscholar.org/graph/v1/paper/search".to_string(),
                    "Semantic Scholar papers".to_string(),
                    "semanticscholar.org".to_string(),
                    body,
                )
            }
            ProviderKind::OpenStreetMap => {
                let v = get_json(
                    &client,
                    "https://nominatim.openstreetmap.org/search",
                    &[("q", query), ("format", "jsonv2"), ("limit", "3")],
                )?;
                let body = v
                    .as_array()
                    .ok_or(SourceError::Empty)?
                    .iter()
                    .map(|p| {
                        format!(
                            "{} (lat {}, lon {})",
                            p["display_name"].as_str().unwrap_or(""),
                            p["lat"],
                            p["lon"]
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "https://nominatim.openstreetmap.org/search".to_string(),
                    "OpenStreetMap geocoding".to_string(),
                    "openstreetmap.org".to_string(),
                    body,
                )
            }
            ProviderKind::GitHub => {
                let v = get_json(
                    &client,
                    "https://api.github.com/search/repositories",
                    &[("q", query), ("per_page", "3")],
                )?;
                let body = v["items"]
                    .as_array()
                    .ok_or(SourceError::Empty)?
                    .iter()
                    .map(|r| {
                        format!(
                            "{} ★{}: {}",
                            r["full_name"].as_str().unwrap_or(""),
                            r["stargazers_count"],
                            r["description"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "https://api.github.com/search/repositories".to_string(),
                    "GitHub repositories".to_string(),
                    "github.com".to_string(),
                    body,
                )
            }
            ProviderKind::StackExchange => {
                let v = get_json(
                    &client,
                    "https://api.stackexchange.com/2.3/search/advanced",
                    &[("q", query), ("site", "stackoverflow"), ("pagesize", "3")],
                )?;
                let body = v["items"]
                    .as_array()
                    .ok_or(SourceError::Empty)?
                    .iter()
                    .map(|r| {
                        format!(
                            "{} (score {}): {}",
                            r["title"].as_str().unwrap_or(""),
                            r["score"],
                            r["link"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "https://api.stackexchange.com/2.3/search/advanced".to_string(),
                    "Stack Exchange answers".to_string(),
                    "stackexchange.com".to_string(),
                    body,
                )
            }
            ProviderKind::Nvd => {
                let v = get_json(
                    &client,
                    "https://services.nvd.nist.gov/rest/json/cves/2.0",
                    &[("keywordSearch", query), ("resultsPerPage", "3")],
                )?;
                let body = v["vulnerabilities"]
                    .as_array()
                    .ok_or(SourceError::Empty)?
                    .iter()
                    .map(|r| {
                        let c = &r["cve"];
                        let desc = c["descriptions"]
                            .as_array()
                            .and_then(|d| d.first())
                            .and_then(|d| d["value"].as_str())
                            .unwrap_or("");
                        format!("{}: {}", c["id"].as_str().unwrap_or(""), desc)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    "https://services.nvd.nist.gov/rest/json/cves/2.0".to_string(),
                    "NVD vulnerabilities".to_string(),
                    "nvd.nist.gov".to_string(),
                    body,
                )
            }
            ProviderKind::RestCountries => {
                let url = format!(
                    "https://restcountries.com/v3.1/name/{}",
                    url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
                );
                let v: Value = client
                    .get(&url)
                    .send()
                    .map_err(map_error)?
                    .error_for_status()
                    .map_err(map_error)?
                    .json()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
                let body = v
                    .as_array()
                    .ok_or(SourceError::Empty)?
                    .iter()
                    .take(3)
                    .map(|c| {
                        format!(
                            "{}; capital: {}; population: {}; region: {}",
                            c["name"]["common"].as_str().unwrap_or(""),
                            c["capital"]
                                .as_array()
                                .and_then(|a| a.first())
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            c["population"],
                            c["region"].as_str().unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (
                    url,
                    "REST Countries".to_string(),
                    "restcountries.com".to_string(),
                    body,
                )
            }
            _ => return Err(SourceError::Empty),
        };
        if text.trim().is_empty() {
            return Err(SourceError::Empty);
        }
        Ok(RawEvidence {
            chunks: vec![EvidenceChunk {
                text: text.chars().take(4_000).collect(),
                source_url: url,
                source_title: title,
                host,
            }],
            source_kind: SourceKind::Dedicated(format!("{kind:?}")),
        })
    }
}

fn get_json(client: &Client, url: &str, params: &[(&str, &str)]) -> Result<Value, SourceError> {
    client
        .get(url)
        .query(params)
        .send()
        .map_err(map_error)?
        .error_for_status()
        .map_err(map_error)?
        .json()
        .map_err(|e| SourceError::FetchFailed(e.to_string()))
}
fn map_error(error: reqwest::Error) -> SourceError {
    if error.is_timeout() {
        SourceError::Timeout
    } else {
        SourceError::FetchFailed(error.to_string())
    }
}
fn strip_xml(value: &str) -> String {
    value
        .replace("<title>", "")
        .replace("</title>", " ")
        .replace("<summary>", "")
        .replace("</summary>", " ")
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
