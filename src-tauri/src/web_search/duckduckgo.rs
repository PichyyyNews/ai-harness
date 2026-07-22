use super::SearchResult;
use scraper::{Html, Selector};
use std::time::Duration;

pub fn search(query: &str) -> Result<Vec<SearchResult>, String> {
    // Try Instant Answer API first for fast, structured keyless response
    if let Ok(results) = search_api(query) {
        if !results.is_empty() {
            return Ok(results);
        }
    }

    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent("Mozilla/5.0 AI Harness web grounding")
        .build()
        .map_err(|error| error.to_string())?
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .map_err(|error| format!("DuckDuckGo request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("DuckDuckGo returned an error: {error}"))?
        .text()
        .map_err(|error| format!("Could not read DuckDuckGo results: {error}"))?;
    parse_results(&response)
}

fn search_api(query: &str) -> Result<Vec<SearchResult>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent("Mozilla/5.0 AI Harness web grounding")
        .build()
        .map_err(|e| e.to_string())?;

    let value: serde_json::Value = client
        .get("https://api.duckduckgo.com/")
        .query(&[("q", query), ("format", "json"), ("no_html", "1")])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    let mut results = Vec::new();

    if let Some(abstract_text) = value.get("AbstractText").and_then(|v| v.as_str()) {
        if !abstract_text.trim().is_empty() {
            let heading = value
                .get("Heading")
                .and_then(|v| v.as_str())
                .unwrap_or(query);
            let url = value
                .get("AbstractURL")
                .and_then(|v| v.as_str())
                .unwrap_or("https://duckduckgo.com")
                .to_string();

            results.push(SearchResult {
                title: heading.to_string(),
                url,
                snippet: abstract_text.to_string(),
                content: String::new(),
            });
        }
    }

    if let Some(related) = value.get("RelatedTopics").and_then(|v| v.as_array()) {
        for item in related.iter().take(5) {
            let text = item.get("Text").and_then(|v| v.as_str());
            let url = item.get("FirstURL").and_then(|v| v.as_str());
            if let (Some(t), Some(u)) = (text, url) {
                if !t.trim().is_empty() {
                    results.push(SearchResult {
                        title: t.chars().take(60).collect(),
                        url: u.to_string(),
                        snippet: t.to_string(),
                        content: String::new(),
                    });
                }
            }
        }
    }

    if results.is_empty() {
        Err("Instant Answer returned no results".to_string())
    } else {
        Ok(results)
    }
}

fn parse_results(response: &str) -> Result<Vec<SearchResult>, String> {
    let document = Html::parse_document(response);
    let result_selector = Selector::parse(".result").map_err(|error| error.to_string())?;
    let title_selector = Selector::parse(".result__a").map_err(|error| error.to_string())?;
    let snippet_selector =
        Selector::parse(".result__snippet").map_err(|error| error.to_string())?;
    let mut results = Vec::new();

    for result in document.select(&result_selector) {
        // Sponsored cards use a DuckDuckGo redirect rather than the publisher
        // URL. They are not search evidence and can otherwise occupy the
        // first, most relevant-looking retrieval slot.
        if result
            .value()
            .classes()
            .any(|class_name| class_name.eq_ignore_ascii_case("result--ad"))
        {
            continue;
        }
        let Some(title) = result.select(&title_selector).next() else {
            continue;
        };
        let Some(href) = title.value().attr("href") else {
            continue;
        };
        let url = destination_url(href).unwrap_or_else(|| href.to_string());
        if !super::scraper::is_safe_external_url(&url) {
            continue;
        }
        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(element_text)
            .unwrap_or_default();
        results.push(SearchResult {
            title: element_text(title),
            url,
            snippet,
            content: String::new(),
        });
        if results.len() == 12 {
            break;
        }
    }
    if results.is_empty() {
        Err("DuckDuckGo returned no usable results.".to_string())
    } else {
        Ok(results)
    }
}

fn destination_url(href: &str) -> Option<String> {
    let url = url::Url::parse(href)
        .or_else(|_| url::Url::parse(&format!("https:{href}")))
        .ok()?;
    url.query_pairs()
        .find(|(key, _)| key == "uddg")
        .map(|(_, value)| value.into_owned())
}

fn element_text(element: scraper::ElementRef<'_>) -> String {
    element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_ads_and_keeps_direct_result_urls() {
        let html = r#"
            <article class="result result--ad">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fads.example%2Foffer">Advertisement</a>
              <span class="result__snippet">Sponsored result that must not become evidence.</span>
            </article>
            <article class="result results_links">
              <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgithub.com%2Fexample%2Fproject">Useful library</a>
              <span class="result__snippet">A useful open source library for an AI harness.</span>
            </article>
        "#;

        let results = parse_results(html).expect("one organic result");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Useful library");
        assert_eq!(results[0].url, "https://github.com/example/project");
    }

    #[test]
    fn decodes_protocol_relative_redirects() {
        assert_eq!(
            destination_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs"),
            Some("https://example.com/docs".to_string())
        );
    }
}
