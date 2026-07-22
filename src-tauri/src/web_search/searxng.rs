use super::SearchResult;
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_INSTANCES: [&str; 2] = ["https://searx.be", "https://search.ononoki.org"];

#[derive(Debug, Deserialize)]
struct SearxResponse {
    results: Vec<SearxItem>,
}

#[derive(Debug, Deserialize)]
struct SearxItem {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
}

pub fn search(query: &str) -> Result<Vec<SearchResult>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("AI Harness web grounding")
        .build()
        .map_err(|error| format!("Could not create SearXNG client: {error}"))?;
    let configured = std::env::var("AI_HARNESS_SEARXNG_URL").ok();
    let instances = configured
        .iter()
        .map(String::as_str)
        .chain(DEFAULT_INSTANCES.iter().copied());
    let mut last_error = "No SearXNG instance responded.".to_string();

    for instance in instances {
        let endpoint = format!("{}/search", instance.trim_end_matches('/'));
        match client
            .get(&endpoint)
            .query(&[("q", query), ("format", "json"), ("language", "all")])
            .send()
        {
            Ok(response) if response.status().is_success() => {
                match response.json::<SearxResponse>() {
                    Ok(payload) => {
                        let results = payload
                            .results
                            .into_iter()
                            .filter_map(|item| {
                                let url = item.url?;
                                if super::scraper::is_safe_external_url(&url) {
                                    Some(SearchResult {
                                        title: item.title.unwrap_or_default(),
                                        url,
                                        snippet: item.content.unwrap_or_default(),
                                        content: String::new(),
                                    })
                                } else {
                                    None
                                }
                            })
                            .take(12)
                            .collect::<Vec<_>>();
                        if !results.is_empty() {
                            return Ok(results);
                        }
                        last_error = format!("{instance} returned no usable results.");
                    }
                    Err(error) => {
                        last_error =
                            format!("Could not parse SearXNG response from {instance}: {error}")
                    }
                }
            }
            Ok(response) => last_error = format!("{instance} returned HTTP {}.", response.status()),
            Err(error) => last_error = format!("{instance} could not be reached: {error}"),
        }
    }
    Err(last_error)
}
