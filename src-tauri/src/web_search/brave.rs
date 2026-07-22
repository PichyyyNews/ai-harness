use super::SearchResult;
use scraper::{Html, Selector};
use std::time::Duration;

/// A keyless fallback for when DuckDuckGo throttles its HTML endpoint. The
/// application reads only public result titles, URLs and snippets; it does not
/// use Brave's answer-generation endpoint or send any local conversation data.
pub fn search(query: &str) -> Result<Vec<SearchResult>, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("Mozilla/5.0 AI Harness web grounding")
        .build()
        .map_err(|error| error.to_string())?
        .get("https://search.brave.com/search")
        .query(&[("q", query)])
        .send()
        .map_err(|error| format!("Brave Search request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Brave Search returned an error: {error}"))?
        .text()
        .map_err(|error| format!("Could not read Brave Search results: {error}"))?;
    parse_results(&response)
}

fn parse_results(response: &str) -> Result<Vec<SearchResult>, String> {
    let document = Html::parse_document(response);
    let result_selector =
        Selector::parse(".snippet[data-type='web']").map_err(|error| error.to_string())?;
    let link_selector = Selector::parse("a[href]").map_err(|error| error.to_string())?;
    let title_selector = Selector::parse(".title").map_err(|error| error.to_string())?;
    let snippet_selector =
        Selector::parse(".generic-snippet .content").map_err(|error| error.to_string())?;
    let mut results = Vec::new();

    for result in document.select(&result_selector) {
        let Some(link) = result.select(&link_selector).next() else {
            continue;
        };
        let Some(url) = link.value().attr("href") else {
            continue;
        };
        if !super::scraper::is_safe_external_url(url) {
            continue;
        }
        let title = result
            .select(&title_selector)
            .next()
            .map(element_text)
            .unwrap_or_default();
        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(element_text)
            .unwrap_or_default();
        if title.trim().is_empty() || snippet.trim().is_empty() {
            continue;
        }
        results.push(SearchResult {
            title,
            url: url.to_string(),
            snippet,
            content: String::new(),
        });
        if results.len() == 12 {
            break;
        }
    }
    if results.is_empty() {
        Err("Brave Search returned no usable results.".to_string())
    } else {
        Ok(results)
    }
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
    fn parses_public_web_result_cards() {
        let html = r#"
            <div class="snippet" data-type="web">
              <a href="https://github.com/example/ai-harness">
                <div class="title">AI Harness library</div>
              </a>
              <div class="generic-snippet"><div class="content">A public library for building AI harnesses.</div></div>
            </div>
        "#;
        let results = parse_results(html).expect("one web result");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://github.com/example/ai-harness");
        assert_eq!(results[0].title, "AI Harness library");
    }
}
