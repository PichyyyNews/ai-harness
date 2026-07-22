use super::SearchResult;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

const CRAWL4AI_ENDPOINT: &str = "http://127.0.0.1:11235/crawl";
const CRAWL4AI_HEALTH: &str = "http://127.0.0.1:11235/health";
const MAX_CACHE_AGE_SECS: u64 = 86400; // 24 hours

#[derive(Serialize)]
struct CrawlRequest<'a> {
    urls: &'a [String],
    word_count_threshold: usize,
    extract_tables: bool,
}

#[derive(Deserialize)]
struct CrawlResultItem {
    url: String,
    markdown: String,
    success: bool,
}

#[derive(Deserialize)]
struct CrawlResponse {
    results: Vec<CrawlResultItem>,
}

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

static SPAWN_ATTEMPTED: AtomicBool = AtomicBool::new(false);

/// Checks whether the local Crawl4AI sidecar service is alive and healthy.
pub fn is_service_healthy() -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    client
        .get(CRAWL4AI_HEALTH)
        .send()
        .is_ok_and(|res| res.status().is_success())
}

/// Automatically spawns the Python Crawl4AI sidecar service if not already running.
pub fn ensure_sidecar_running() -> bool {
    if is_service_healthy() {
        return true;
    }

    if SPAWN_ATTEMPTED.swap(true, Ordering::Relaxed) {
        return is_service_healthy();
    }

    let script_path = std::env::current_dir()
        .map(|mut p| {
            p.push("sidecars");
            p.push("crawl4ai_service");
            p.push("main.py");
            p
        })
        .ok();

    if let Some(path) = script_path {
        if path.exists() {
            let _ = Command::new("python")
                .arg(&path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            std::thread::sleep(Duration::from_millis(1200));
        }
    }

    is_service_healthy()
}

/// Attempts to enrich search results using the Crawl4AI REST service.
/// If the service is unavailable or fails, returns `None` to allow fallback.
pub fn enrich_with_crawl4ai(results: &[SearchResult]) -> Option<Vec<SearchResult>> {
    if results.is_empty() {
        return Some(vec![]);
    }

    if !ensure_sidecar_running() {
        return None;
    }

    let urls: Vec<String> = results.iter().map(|r| r.url.clone()).collect();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("Mozilla/5.0 AI Harness Crawl4AI Integration")
        .build()
        .ok()?;

    let payload = CrawlRequest {
        urls: &urls,
        word_count_threshold: 10,
        extract_tables: true,
    };

    let response = client
        .post(CRAWL4AI_ENDPOINT)
        .json(&payload)
        .send()
        .ok()?
        .error_for_status()
        .ok()?;

    let crawl_res: CrawlResponse = response.json().ok()?;
    let mut enriched = Vec::new();

    for result in results {
        if let Some(item) = crawl_res.results.iter().find(|i| i.url == result.url) {
            if item.success && !item.markdown.trim().is_empty() {
                // Save to local .md cache for offline auditing
                save_to_markdown_cache(&result.url, &item.markdown);

                enriched.push(SearchResult {
                    content: item.markdown.clone(),
                    ..result.clone()
                });
                continue;
            }
        }

        // Try reading from disk cache if network call failed for specific URL
        if let Some(cached_md) = read_from_markdown_cache(&result.url) {
            enriched.push(SearchResult {
                content: cached_md,
                ..result.clone()
            });
        } else {
            enriched.push(result.clone());
        }
    }

    Some(enriched)
}

fn get_cache_dir() -> Option<PathBuf> {
    let mut dir = std::env::temp_dir();
    dir.push("ai-harness-cache");
    dir.push("crawl4ai");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn hash_url(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn save_to_markdown_cache(url: &str, content: &str) {
    if let Some(mut path) = get_cache_dir() {
        path.push(format!("{}.md", hash_url(url)));
        let _ = fs::write(path, content);
    }
}

fn read_from_markdown_cache(url: &str) -> Option<String> {
    let mut path = get_cache_dir()?;
    path.push(format!("{}.md", hash_url(url)));
    let metadata = fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?;
    if SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        .as_secs()
        > MAX_CACHE_AGE_SECS
    {
        let _ = fs::remove_file(path);
        return None;
    }
    fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip_works() {
        let test_url = "https://example.com/test-crawl4ai-page";
        let test_md = "# Sample Markdown\n| Col 1 | Col 2 |\n|---|---|\n| 100 | 200 |";

        save_to_markdown_cache(test_url, test_md);
        let cached = read_from_markdown_cache(test_url);
        assert_eq!(cached.as_deref(), Some(test_md));
    }
}
