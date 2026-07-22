use super::SourceError;
use crate::web_search::{EvidenceChunk, RawEvidence, SourceHint, SourceKind, SubQuestion};

pub struct CurrencyProvider;

impl CurrencyProvider {
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        let (from, to) = match &sub_q.source_hint {
            SourceHint::Currency { from, to } => (from.to_uppercase(), to.to_uppercase()),
            _ => ("USD".to_string(), "EUR".to_string()),
        };

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

        let url = format!("https://open.er-api.com/v6/latest/{}", from);
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
        struct RateResponse {
            rates: std::collections::HashMap<String, f64>,
        }

        let body: RateResponse = resp
            .json()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
        let rate = body.rates.get(&to).copied().ok_or(SourceError::Empty)?;

        let chunk = EvidenceChunk {
            text: format!("Exchange rate: 1 {} = {} {}", from, rate, to),
            source_url: format!("https://open.er-api.com/v6/latest/{}", from),
            source_title: format!("Currency exchange {} to {}", from, to),
            host: "open.er-api.com".to_string(),
        };

        Ok(RawEvidence {
            chunks: vec![chunk],
            source_kind: SourceKind::Dedicated("Currency".to_string()),
        })
    }
}
