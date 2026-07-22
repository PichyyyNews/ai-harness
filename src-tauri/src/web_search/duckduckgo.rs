use super::SearchResult;
use scraper::{Html, Selector};
use std::time::Duration;

pub fn search(query: &str) -> Result<Vec<SearchResult>, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("Mozilla/5.0 AI Harness web grounding")
        .build().map_err(|error| error.to_string())?
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send().map_err(|error| format!("DuckDuckGo request failed: {error}"))?
        .error_for_status().map_err(|error| format!("DuckDuckGo returned an error: {error}"))?
        .text().map_err(|error| format!("Could not read DuckDuckGo results: {error}"))?;
    let document = Html::parse_document(&response);
    let result_selector = Selector::parse(".result").map_err(|error| error.to_string())?;
    let title_selector = Selector::parse(".result__a").map_err(|error| error.to_string())?;
    let snippet_selector = Selector::parse(".result__snippet").map_err(|error| error.to_string())?;
    let mut results = Vec::new();

    for result in document.select(&result_selector) {
        let Some(title) = result.select(&title_selector).next() else { continue; };
        let Some(href) = title.value().attr("href") else { continue; };
        let url = destination_url(href).unwrap_or_else(|| href.to_string());
        if !super::scraper::is_safe_external_url(&url) { continue; }
        let snippet = result.select(&snippet_selector).next().map(element_text).unwrap_or_default();
        results.push(SearchResult { title: element_text(title), url, snippet, content: String::new() });
        if results.len() == 8 { break; }
    }
    if results.is_empty() { Err("DuckDuckGo returned no usable results.".to_string()) } else { Ok(results) }
}

fn destination_url(href: &str) -> Option<String> {
    let url = url::Url::parse(href).or_else(|_| url::Url::parse(&format!("https:{href}"))).ok()?;
    url.query_pairs().find(|(key, _)| key == "uddg").map(|(_, value)| value.into_owned())
}

fn element_text(element: scraper::ElementRef<'_>) -> String {
    element.text().collect::<Vec<_>>().join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}
