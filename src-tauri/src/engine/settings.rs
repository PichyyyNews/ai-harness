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
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self { backend: BackendPreference::Auto, gpu_layers: -1 }
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app.path().app_data_dir().map_err(|error| format!("Could not resolve app data directory: {error}"))?;
    std::fs::create_dir_all(&directory).map_err(|error| format!("Could not create app data directory: {error}"))?;
    Ok(directory.join("engine-settings.json"))
}

pub fn load(app: &AppHandle) -> Result<EngineSettings, String> {
    let path = settings_path(app)?;
    if !path.is_file() { return Ok(EngineSettings::default()); }
    let bytes = std::fs::read(path).map_err(|error| format!("Could not read engine settings: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Could not read engine settings: {error}"))
}

pub fn save(app: &AppHandle, settings: &EngineSettings) -> Result<EngineSettings, String> {
    if settings.gpu_layers < -1 || settings.gpu_layers > 200 { return Err("GPU layer offload must be between 0 and 200, or -1 for full offload.".to_string()); }
    let path = settings_path(app)?;
    let temporary = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(settings).map_err(|error| format!("Could not prepare engine settings: {error}"))?;
    std::fs::write(&temporary, content).map_err(|error| format!("Could not write engine settings: {error}"))?;
    if path.is_file() { std::fs::remove_file(&path).map_err(|error| format!("Could not replace engine settings: {error}"))?; }
    std::fs::rename(temporary, path).map_err(|error| format!("Could not finalize engine settings: {error}"))?;
    Ok(settings.clone())
}

pub fn backend_cache_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app.path().app_data_dir().map_err(|error| format!("Could not resolve app data directory: {error}"))?.join("backends");
    std::fs::create_dir_all(&directory).map_err(|error| format!("Could not create backend cache directory: {error}"))?;
    Ok(directory)
}
