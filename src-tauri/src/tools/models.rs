use crate::models;
use serde_json::json;
use std::time::Duration;
use tauri::AppHandle;

pub fn list_installed_models(app: &AppHandle) -> Result<String, String> {
    let installed = models::registry::list(app)?;
    let items: Vec<_> = installed
        .into_iter()
        .map(|model| {
            json!({
                "fileName": model.file_name,
                "repoId": model.repo_id,
                "sizeBytes": model.size,
                "sizeMb": model.size / 1024 / 1024,
                "sha256": model.sha256
            })
        })
        .collect();

    Ok(json!({
        "count": items.len(),
        "models": items
    })
    .to_string())
}

pub fn search_huggingface_models(query: &str) -> Result<String, String> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Err("Search query cannot be empty.".to_string());
    }

    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&limit=8&sort=downloads&direction=-1",
        urlencoding::encode(clean_query)
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("Mozilla/5.0 AI Harness desktop catalog")
        .build()
        .map_err(|e| format!("Could not create request client: {e}"))?;

    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Hugging Face API request failed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("Hugging Face returned HTTP {}", response.status()));
    }

    let raw_list: serde_json::Value = response
        .json()
        .map_err(|e| format!("Could not parse Hugging Face response: {e}"))?;

    let mut results = Vec::new();
    if let Some(arr) = raw_list.as_array() {
        for item in arr {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            let downloads = item.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
            let likes = item.get("likes").and_then(|v| v.as_u64()).unwrap_or(0);
            let pipeline_tag = item
                .get("pipeline_tag")
                .and_then(|v| v.as_str())
                .unwrap_or("text-generation");

            if !id.is_empty() {
                results.push(json!({
                    "modelId": id,
                    "downloads": downloads,
                    "likes": likes,
                    "pipelineTag": pipeline_tag,
                    "url": format!("https://huggingface.co/{id}")
                }));
            }
        }
    }

    Ok(json!({
        "query": clean_query,
        "count": results.len(),
        "models": results
    })
    .to_string())
}
