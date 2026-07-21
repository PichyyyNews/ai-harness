use super::settings::BackendPreference;
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub gpus: Vec<String>,
    pub recommended_backend: BackendPreference,
    pub recommendation_reason: String,
    pub vram_total_mib: Option<u64>,
    pub vram_used_mib: Option<u64>,
}

pub fn nvidia_vram_used_mib() -> Option<u64> {
    Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output().ok().filter(|output| output.status.success())
        .and_then(|output| String::from_utf8_lossy(&output.stdout).lines().next().and_then(|line| line.trim().parse::<u64>().ok()))
}

fn nvidia_memory() -> Option<(u64, u64)> {
    Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total,memory.used", "--format=csv,noheader,nounits"])
        .output().ok().filter(|output| output.status.success())
        .and_then(|output| {
            let values = String::from_utf8_lossy(&output.stdout).lines().next()?.split(',').map(str::trim).map(str::parse::<u64>).collect::<Result<Vec<_>, _>>().ok()?;
            Some((*values.first()?, *values.get(1)?))
        })
}

pub fn detect() -> HardwareProfile {
    #[cfg(target_os = "windows")]
    {
        let gpus = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).lines().map(str::trim).filter(|name| !name.is_empty()).map(ToOwned::to_owned).collect::<Vec<_>>())
            .unwrap_or_default();
        let names = gpus.join(" ").to_lowercase();
        let memory = nvidia_memory();
        if names.contains("nvidia") { return HardwareProfile { gpus, recommended_backend: BackendPreference::Cuda, recommendation_reason: "NVIDIA GPU detected; CUDA is the preferred backend and the runtime will maximize safe VRAM offload.".to_string(), vram_total_mib: memory.map(|value| value.0), vram_used_mib: memory.map(|value| value.1) }; }
        if names.contains("amd") || names.contains("radeon") || names.contains("intel") { return HardwareProfile { gpus, recommended_backend: BackendPreference::Vulkan, recommendation_reason: "A Vulkan-capable GPU may be available; Vulkan is selected with CPU fallback if it cannot load.".to_string(), vram_total_mib: None, vram_used_mib: None }; }
        return HardwareProfile { gpus, recommended_backend: BackendPreference::Cpu, recommendation_reason: "No supported GPU was detected; CPU is selected.".to_string(), vram_total_mib: None, vram_used_mib: None };
    }
    #[cfg(target_os = "macos")]
    { HardwareProfile { gpus: Vec::new(), recommended_backend: BackendPreference::Cpu, recommendation_reason: "Metal selection is reserved until a managed Metal runtime is available; CPU remains safe today.".to_string(), vram_total_mib: None, vram_used_mib: None } }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    { HardwareProfile { gpus: Vec::new(), recommended_backend: BackendPreference::Vulkan, recommendation_reason: "Vulkan is the preferred GPU backend on Linux when the runtime and driver are available; CPU is used as fallback.".to_string(), vram_total_mib: None, vram_used_mib: None } }
}
