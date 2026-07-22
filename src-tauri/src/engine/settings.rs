use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendPreference {
    Auto,
    Cpu,
    Cuda,
    Vulkan,
    Sycl,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineSettings {
    pub backend: BackendPreference,
    /// -1 means full offload when a GPU backend is active; 0 means CPU-only layers.
    pub gpu_layers: i32,
    #[serde(default = "default_memory_agent_enabled")]
    pub memory_agent_enabled: bool,
    #[serde(default = "default_memory_injection_enabled")]
    pub memory_injection_enabled: bool,
    /// Optional localhost endpoint for a separately managed small memory model.
    /// When absent, the priority queue uses the main endpoint opportunistically.
    #[serde(default)]
    pub memory_agent_endpoint: Option<String>,
    #[serde(default = "default_embedding_enabled")]
    pub embedding_enabled: bool,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            backend: BackendPreference::Auto,
            gpu_layers: -1,
            memory_agent_enabled: true,
            memory_injection_enabled: true,
            memory_agent_endpoint: None,
            embedding_enabled: true,
        }
    }
}

fn default_memory_agent_enabled() -> bool {
    true
}
fn default_memory_injection_enabled() -> bool {
    true
}
fn default_embedding_enabled() -> bool {
    true
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve app data directory: {error}"))?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create app data directory: {error}"))?;
    Ok(directory.join("engine-settings.json"))
}

pub fn load(app: &AppHandle) -> Result<EngineSettings, String> {
    let path = settings_path(app)?;
    if !path.is_file() {
        return Ok(EngineSettings::default());
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not read engine settings: {error}"))?;
    let settings: EngineSettings = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not read engine settings: {error}"))?;
    validate(&settings)?;
    Ok(settings)
}

pub fn save(app: &AppHandle, settings: &EngineSettings) -> Result<EngineSettings, String> {
    validate(settings)?;
    let path = settings_path(app)?;
    let temporary = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Could not prepare engine settings: {error}"))?;
    std::fs::write(&temporary, content)
        .map_err(|error| format!("Could not write engine settings: {error}"))?;
    if path.is_file() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("Could not replace engine settings: {error}"))?;
    }
    std::fs::rename(temporary, path)
        .map_err(|error| format!("Could not finalize engine settings: {error}"))?;
    Ok(settings.clone())
}

fn validate(settings: &EngineSettings) -> Result<(), String> {
    if settings.gpu_layers < -1 || settings.gpu_layers > 200 {
        return Err(
            "GPU layer offload must be between 0 and 200, or -1 for full offload.".to_string(),
        );
    }
    if let Some(endpoint) = &settings.memory_agent_endpoint {
        let url = url::Url::parse(endpoint)
            .map_err(|_| "The memory-agent endpoint must be a valid localhost URL.".to_string())?;
        if !matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1")
        ) {
            return Err(
                "The memory-agent endpoint must remain local to this computer.".to_string(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_files_enable_the_memory_agent_by_default() {
        let settings: EngineSettings = serde_json::from_str(r#"{"backend":"auto","gpuLayers":-1}"#)
            .expect("backward-compatible settings");
        assert!(settings.memory_agent_enabled);
        assert!(settings.memory_injection_enabled);
        assert!(settings.memory_agent_endpoint.is_none());
    }
}

pub fn backend_cache_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve app data directory: {error}"))?
        .join("backends");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create backend cache directory: {error}"))?;
    Ok(directory)
}
