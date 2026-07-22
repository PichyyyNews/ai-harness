use super::SearchResult;
use std::time::Duration;

/// Bing's documented RSS presentation is a keyless, language-neutral search
/// surface. It is considerably more stable for a desktop client than scraping
/// an interactive HTML results page that can return bot challenges.
pub fn search(query: &str) -> Result<Vec<SearchResult>, String> {
    let feed = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(6))
        .user_agent("Mozilla/5.0 AI Harness web grounding")
        .build()
        .map_err(|error| format!("Could not create Bing RSS client: {error}"))?
        .get("https://www.bing.com/search")
        .query(&[("q", query), ("format", "rss")])
        .send()
        .map_err(|error| format!("Bing RSS request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Bing RSS returned an error: {error}"))?
        .text()
        .map_err(|error| format!("Could not read Bing RSS results: {error}"))?;

    let results = feed
        .split("<item>")
        .skip(1)
        .filter_map(|entry| entry.split_once("</item>").map(|(item, _)| item))
        .filter_map(|item| {
            let title = decode_entities(&tag_value(item, "title")?);
            let url = decode_entities(&tag_value(item, "link")?);
            let snippet = decode_entities(&tag_value(item, "description").unwrap_or_default());
            if title.trim().is_empty() || !super::scraper::is_safe_external_url(&url) {
                return None;
            }
            Some(SearchResult {
                title,
                url,
                snippet,
                content: String::new(),
            })
        })
        .take(12)
        .collect::<Vec<_>>();

    if results.is_empty() {
        Err("Bing RSS returned no usable results.".to_string())
    } else {
        Ok(results)
    }
}

fn tag_value(item: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let value = item.split_once(&start)?.1.split_once(&end)?.0.trim();
    Some(
        value
            .strip_prefix("<![CDATA[")
            .and_then(|text| text.strip_suffix("]]>"))
            .unwrap_or(value)
            .trim()
            .to_string(),
    )
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bing_rss_items() {
        let item = "<title>Rust &amp; Web</title><link>https://example.com/rust</link><description>Useful result text for a query.</description>";
        assert_eq!(
            decode_entities(&tag_value(item, "title").unwrap()),
            "Rust & Web"
        );
        assert_eq!(
            tag_value(item, "link").as_deref(),
            Some("https://example.com/rust")
        );
    }
}
