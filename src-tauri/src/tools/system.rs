use crate::engine::{hardware, settings};
use serde_json::json;
use tauri::AppHandle;

pub fn get_system_status(app: &AppHandle) -> Result<String, String> {
    let profile = hardware::detect();
    let current_settings = settings::load(app).unwrap_or_default();

    let vram_used_mb = profile.vram_used_mib.unwrap_or(0);
    let vram_total_mb = profile.vram_total_mib.unwrap_or(0);
    let vram_free_mb = vram_total_mb.saturating_sub(vram_used_mb);

    Ok(json!({
        "gpuName": profile.gpus.first().cloned().unwrap_or_else(|| "CPU Only".to_string()),
        "vram": {
            "totalMb": vram_total_mb,
            "usedMb": vram_used_mb,
            "freeMb": vram_free_mb,
        },
        "recommendedBackend": format!("{:?}", profile.recommended_backend),
        "configuredBackend": format!("{:?}", current_settings.backend),
        "configuredGpuLayers": current_settings.gpu_layers,
        "embeddingEnabled": current_settings.embedding_enabled,
        "memoryAgentEnabled": current_settings.memory_agent_enabled
    })
    .to_string())
}
