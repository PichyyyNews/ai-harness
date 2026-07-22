use super::{RawEvidence, SourceError, SubQuestion};
use crate::web_search::{EvidenceChunk, SourceKind};
use reqwest::blocking::Client;
use std::time::Duration;

pub struct NewsProvider;

impl NewsProvider {
    /// Google News RSS is a keyless web-search fallback. It gives current
    /// headlines and summaries directly, so retrieval still produces evidence
    /// when an HTML search engine or a news site refuses article scraping.
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        let feed = Client::builder()
            .timeout(Duration::from_secs(4))
            .user_agent("AI Harness retrieval/1.0")
            .build()
            .map_err(|error| SourceError::FetchFailed(error.to_string()))?
            .get("https://news.google.com/rss/search")
            .query(&[("q", sub_q.text.trim())])
            .send()
            .map_err(map_error)?
            .error_for_status()
            .map_err(map_error)?
            .text()
            .map_err(|error| SourceError::FetchFailed(error.to_string()))?;

        let chunks = item_blocks(&feed)
            .into_iter()
            .filter_map(|item| {
                let title = decode_entities(&tag_value(item, "title")?);
                let description = strip_html(&decode_entities(
                    &tag_value(item, "description").unwrap_or_default(),
                ));
                let url = decode_entities(&tag_value(item, "link").unwrap_or_default());
                if title.trim().is_empty() || url.trim().is_empty() {
                    return None;
                }
                Some(EvidenceChunk {
                    text: format!("{}\n{}", title.trim(), description.trim())
                        .chars()
                        .take(2_000)
                        .collect(),
                    source_url: url,
                    source_title: title,
                    host: "news.google.com".to_string(),
                })
            })
            .take(6)
            .collect::<Vec<_>>();
        if chunks.is_empty() {
            return Err(SourceError::Empty);
        }
        Ok(RawEvidence {
            chunks,
            source_kind: SourceKind::Dedicated("GoogleNews".to_string()),
        })
    }
}

fn item_blocks(feed: &str) -> Vec<&str> {
    feed.split("<item>")
        .skip(1)
        .filter_map(|entry| entry.split_once("</item>").map(|(item, _)| item))
        .collect()
}

fn tag_value(item: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let value = item.split_once(&start)?.1.split_once(&end)?.0;
    Some(
        value
            .trim()
            .strip_prefix("<![CDATA[")
            .and_then(|text| text.strip_suffix("]]>"))
            .unwrap_or(value)
            .trim()
            .to_string(),
    )
}

fn strip_html(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn map_error(error: reqwest::Error) -> SourceError {
    if error.is_timeout() {
        SourceError::Timeout
    } else {
        SourceError::FetchFailed(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rss_items_without_an_xml_dependency() {
        let feed = "<rss><item><title><![CDATA[Headline]]></title><link>https://example.com/a</link><description><![CDATA[<b>Summary</b>]]></description></item></rss>";
        let item = item_blocks(feed)[0];
        assert_eq!(tag_value(item, "title").as_deref(), Some("Headline"));
        assert_eq!(strip_html("<b>Summary</b>"), "Summary");
    }
}
