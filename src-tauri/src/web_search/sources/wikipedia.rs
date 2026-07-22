use super::SourceError;
use crate::web_search::{EvidenceChunk, RawEvidence, SourceKind, SubQuestion};

pub struct WikipediaProvider;

impl WikipediaProvider {
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

        let encoded_text: String =
            url::form_urlencoded::byte_serialize(sub_q.text.as_bytes()).collect();
        let url = format!(
            "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
            encoded_text
        );

        let resp = client.get(&url).send().map_err(|e| {
            if e.is_timeout() {
                SourceError::Timeout
            } else {
                SourceError::FetchFailed(e.to_string())
            }
        })?;

        if !resp.status().is_success() {
            return Err(SourceError::Empty);
        }

        #[derive(serde::Deserialize)]
        struct WikiSummary {
            title: Option<String>,
            extract: Option<String>,
            content_urls: Option<ContentUrls>,
        }
        #[derive(serde::Deserialize)]
        struct ContentUrls {
            desktop: Option<DesktopUrl>,
        }
        #[derive(serde::Deserialize)]
        struct DesktopUrl {
            page: Option<String>,
        }

        let summary: WikiSummary = resp
            .json()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
        let extract = summary.extract.unwrap_or_default();
        if extract.trim().is_empty() {
            return Err(SourceError::Empty);
        }

        let page_title = summary.title.unwrap_or_else(|| sub_q.text.clone());
        let page_url = summary
            .content_urls
            .and_then(|u| u.desktop)
            .and_then(|d| d.page)
            .unwrap_or_else(|| "https://en.wikipedia.org".to_string());

        let chunk = EvidenceChunk {
            text: extract,
            source_url: page_url,
            source_title: page_title,
            host: "en.wikipedia.org".to_string(),
        };

        Ok(RawEvidence {
            chunks: vec![chunk],
            source_kind: SourceKind::Dedicated("Wikipedia".to_string()),
        })
    }
}
