use super::SourceError;
use crate::web_search::{EvidenceChunk, RawEvidence, SourceHint, SourceKind, SubQuestion};

pub struct StocksProvider;

impl StocksProvider {
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        let ticker = match &sub_q.source_hint {
            SourceHint::StockOrCrypto { ticker } => ticker.to_lowercase(),
            _ => return Err(SourceError::Empty),
        };

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

        let url = format!(
            "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd",
            ticker
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

        let body: std::collections::HashMap<String, std::collections::HashMap<String, f64>> = resp
            .json()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

        if let Some(prices) = body.get(&ticker) {
            if let Some(usd) = prices.get("usd") {
                let chunk = EvidenceChunk {
                    text: format!(
                        "Current price of {} is ${:.2} USD",
                        ticker.to_uppercase(),
                        usd
                    ),
                    source_url: format!(
                        "https://api.coingecko.com/api/v3/simple/price?ids={}",
                        ticker
                    ),
                    source_title: format!("{} Price Quote", ticker.to_uppercase()),
                    host: "api.coingecko.com".to_string(),
                };

                return Ok(RawEvidence {
                    chunks: vec![chunk],
                    source_kind: SourceKind::Dedicated("StockOrCrypto".to_string()),
                });
            }
        }

        Err(SourceError::Empty)
    }
}
