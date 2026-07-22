use super::SourceError;
use crate::web_search::{
    Ecosystem, EvidenceChunk, RawEvidence, SourceHint, SourceKind, SubQuestion,
};

pub struct RegistryProvider;

impl RegistryProvider {
    pub fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError> {
        let (ecosystem, package) = match &sub_q.source_hint {
            SourceHint::PackageRegistry { ecosystem, package } => (ecosystem, package),
            _ => return Err(SourceError::Empty),
        };

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

        match ecosystem {
            Ecosystem::Rust => {
                let url = format!("https://crates.io/api/v1/crates/{}", package);
                let resp = client
                    .get(&url)
                    .header("User-Agent", "AntigravityHarness/1.0 (contact@example.com)")
                    .send()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

                if !resp.status().is_success() {
                    return Err(SourceError::Empty);
                }

                #[derive(serde::Deserialize)]
                struct CrateResp {
                    krate: CrateMeta,
                }
                #[derive(serde::Deserialize)]
                struct CrateMeta {
                    name: String,
                    max_version: String,
                    description: Option<String>,
                }

                let body: CrateResp = resp
                    .json()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
                let chunk = EvidenceChunk {
                    text: format!(
                        "Rust crate '{}' (version {}): {}",
                        body.krate.name,
                        body.krate.max_version,
                        body.krate.description.unwrap_or_default()
                    ),
                    source_url: format!("https://crates.io/crates/{}", package),
                    source_title: format!("crates.io: {}", package),
                    host: "crates.io".to_string(),
                };

                Ok(RawEvidence {
                    chunks: vec![chunk],
                    source_kind: SourceKind::Dedicated("PackageRegistry".to_string()),
                })
            }
            Ecosystem::Npm => {
                let url = format!("https://registry.npmjs.org/{}", package);
                let resp = client
                    .get(&url)
                    .send()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

                if !resp.status().is_success() {
                    return Err(SourceError::Empty);
                }

                #[derive(serde::Deserialize)]
                struct NpmResp {
                    name: String,
                    description: Option<String>,
                    #[serde(rename = "dist-tags")]
                    dist_tags: Option<std::collections::HashMap<String, String>>,
                }

                let body: NpmResp = resp
                    .json()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
                let latest = body
                    .dist_tags
                    .as_ref()
                    .and_then(|t| t.get("latest"))
                    .cloned()
                    .unwrap_or_default();

                let chunk = EvidenceChunk {
                    text: format!(
                        "npm package '{}' (latest {}): {}",
                        body.name,
                        latest,
                        body.description.unwrap_or_default()
                    ),
                    source_url: format!("https://www.npmjs.com/package/{}", package),
                    source_title: format!("npm: {}", package),
                    host: "npmjs.com".to_string(),
                };

                Ok(RawEvidence {
                    chunks: vec![chunk],
                    source_kind: SourceKind::Dedicated("PackageRegistry".to_string()),
                })
            }
            Ecosystem::PyPI => {
                let url = format!("https://pypi.org/pypi/{}/json", package);
                let resp = client
                    .get(&url)
                    .send()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;

                if !resp.status().is_success() {
                    return Err(SourceError::Empty);
                }

                #[derive(serde::Deserialize)]
                struct PyPiResp {
                    info: PyPiInfo,
                }
                #[derive(serde::Deserialize)]
                struct PyPiInfo {
                    name: String,
                    version: String,
                    summary: Option<String>,
                }

                let body: PyPiResp = resp
                    .json()
                    .map_err(|e| SourceError::FetchFailed(e.to_string()))?;
                let chunk = EvidenceChunk {
                    text: format!(
                        "PyPI package '{}' (version {}): {}",
                        body.info.name,
                        body.info.version,
                        body.info.summary.unwrap_or_default()
                    ),
                    source_url: format!("https://pypi.org/project/{}/", package),
                    source_title: format!("PyPI: {}", package),
                    host: "pypi.org".to_string(),
                };

                Ok(RawEvidence {
                    chunks: vec![chunk],
                    source_kind: SourceKind::Dedicated("PackageRegistry".to_string()),
                })
            }
        }
    }
}
