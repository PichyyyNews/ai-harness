use super::SearchResult;
use scraper::{Html, Selector};
use std::{net::IpAddr, thread, time::Duration};

const MAX_DOCUMENT_CHARS: usize = 12_000;
const MAX_DOCUMENTS_TO_READ: usize = 6;

pub fn enrich_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let handles = results
        .into_iter()
        .take(MAX_DOCUMENTS_TO_READ)
        .map(|result| {
            thread::spawn(move || {
                let content =
                    fetch_and_extract(&result.url).unwrap_or_else(|_| result.snippet.clone());
                SearchResult { content, ..result }
            })
        })
        .collect::<Vec<_>>();
    handles
        .into_iter()
        .filter_map(|handle| handle.join().ok())
        .filter(|result| result.content.len() >= 80)
        .collect()
}

pub fn is_safe_external_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host() else {
        return false;
    };
    match host {
        url::Host::Domain(domain) => !matches!(
            domain.to_ascii_lowercase().as_str(),
            "localhost" | "localhost.localdomain"
        ),
        url::Host::Ipv4(address) => is_public_ip(IpAddr::V4(address)),
        url::Host::Ipv6(address) => is_public_ip(IpAddr::V6(address)),
    }
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified())
        }
        IpAddr::V6(address) => !(address.is_loopback() || address.is_unspecified()),
    }
}

fn fetch_and_extract(url: &str) -> Result<String, String> {
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .user_agent("Mozilla/5.0 AI Harness web grounding")
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .map_err(|error| format!("Could not fetch {url}: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Could not fetch {url}: {error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > 1_500_000)
    {
        return Err("Document is too large.".to_string());
    }
    let raw = response
        .text()
        .map_err(|error| format!("Could not read {url}: {error}"))?;
    let document = Html::parse_document(&raw);
    let selector = Selector::parse("article p, main p, article li, main li, p, h1, h2, h3, li")
        .map_err(|error| error.to_string())?;
    let mut text = String::new();
    for element in document.select(&selector) {
        let value = element
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if value.len() >= 40 {
            text.push_str(&value);
            text.push('\n');
        }
        if text.len() >= MAX_DOCUMENT_CHARS {
            break;
        }
    }
    if text.len() < 80 {
        return Err("No readable article text was found.".to_string());
    }
    Ok(text.chars().take(MAX_DOCUMENT_CHARS).collect())
}
