use super::settings::BackendPreference;
use serde::Deserialize;
use std::{fs::{self, File}, io::{self, Cursor, Write}, path::{Path, PathBuf}};
use zip::ZipArchive;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub struct RuntimeFiles {
    pub directory: PathBuf,
    pub server: PathBuf,
    pub release: String,
}

fn runtime_key(backend: BackendPreference) -> Result<&'static str, String> {
    match backend {
        BackendPreference::Cpu | BackendPreference::Auto => Ok("cpu"),
        BackendPreference::Cuda => Ok("cuda-12.4"),
        BackendPreference::Vulkan => Ok("vulkan"),
        BackendPreference::Sycl => Ok("sycl"),
    }
}

fn asset_suffix(backend: BackendPreference) -> Result<&'static str, String> {
    #[cfg(target_os = "windows")]
    match backend {
        BackendPreference::Cpu | BackendPreference::Auto => Ok("-bin-win-cpu-x64.zip"),
        BackendPreference::Cuda => Ok("-bin-win-cuda-12.4-x64.zip"),
        BackendPreference::Vulkan => Ok("-bin-win-vulkan-x64.zip"),
        BackendPreference::Sycl => Ok("-bin-win-sycl-x64.zip"),
    }
    #[cfg(not(target_os = "windows"))]
    { Err("Dynamic runtime downloads are currently implemented for Windows only.".to_string()) }
}

fn server_name() -> &'static str {
    if cfg!(windows) { "llama-server.exe" } else { "llama-server" }
}

fn has_required_backend_files(directory: &Path, backend: BackendPreference) -> bool {
    if !directory.join(server_name()).is_file() || !directory.join(if cfg!(windows) { "llama.dll" } else { "libllama.so" }).is_file() { return false; }
    match backend {
        BackendPreference::Cpu | BackendPreference::Auto => directory.read_dir().ok().into_iter().flatten().filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with("ggml-cpu")),
        BackendPreference::Cuda => directory.join("ggml-cuda.dll").is_file()
            && directory.read_dir().ok().into_iter().flatten().filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with("cudart64_")),
        BackendPreference::Vulkan => directory.join("ggml-vulkan.dll").is_file(),
        BackendPreference::Sycl => directory.read_dir().ok().into_iter().flatten().filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().contains("sycl")),
    }
}

fn fetch_latest_release() -> Result<Release, String> {
    reqwest::blocking::Client::builder()
        .user_agent("AI Harness local runtime manager")
        .build().map_err(|error| format!("Could not create the runtime downloader: {error}"))?
        .get("https://api.github.com/repos/ggml-org/llama.cpp/releases/latest")
        .send().map_err(|error| format!("Could not check the latest llama.cpp runtime: {error}"))?
        .error_for_status().map_err(|error| format!("Could not check the latest llama.cpp runtime: {error}"))?
        .json::<Release>().map_err(|error| format!("Could not read the latest llama.cpp runtime metadata: {error}"))
}

fn extract_runtime(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|error| format!("Could not open the downloaded runtime archive: {error}"))?;
    for index in 0..archive.len() {
        let mut item = archive.by_index(index).map_err(|error| format!("Could not read the runtime archive: {error}"))?;
        if item.is_dir() { continue; }
        let Some(name) = item.enclosed_name().and_then(|path| path.file_name().map(ToOwned::to_owned)) else { continue; };
        let output = destination.join(name);
        let mut file = File::create(output).map_err(|error| format!("Could not extract the runtime: {error}"))?;
        io::copy(&mut item, &mut file).map_err(|error| format!("Could not extract the runtime: {error}"))?;
        file.flush().map_err(|error| format!("Could not finalize the runtime: {error}"))?;
    }
    Ok(())
}

pub fn ensure(backend: BackendPreference, root: &Path) -> Result<RuntimeFiles, String> {
    let key = runtime_key(backend)?;
    let directory = root.join(key);
    if has_required_backend_files(&directory, backend) {
        return Ok(RuntimeFiles { server: directory.join(server_name()), directory, release: "cached".to_string() });
    }
    // Older AI Harness builds installed the CPU runtime directly in
    // `backends/`, before runtimes were split into backend-specific folders.
    // Reuse that verified installation instead of downloading the same
    // llama.cpp archive again just to start the embedding sidecar.
    if matches!(backend, BackendPreference::Cpu | BackendPreference::Auto)
        && has_required_backend_files(root, BackendPreference::Cpu)
    {
        return Ok(RuntimeFiles {
            server: root.join(server_name()),
            directory: root.to_path_buf(),
            release: "cached-legacy-cpu".to_string(),
        });
    }

    let release = fetch_latest_release()?;
    let suffix = asset_suffix(backend)?;
    // CUDA releases also publish a `cudart-...zip` with only NVIDIA runtime
    // dependencies. It has the same platform suffix as the actual llama.cpp
    // archive, so require the `llama-` package that contains llama-server and
    // the ggml backend DLLs.
    let llama_asset = release.assets.iter().find(|asset| asset.name.starts_with("llama-") && asset.name.ends_with(suffix)).ok_or_else(|| format!("llama.cpp {} does not provide a complete {} runtime for this computer.", release.tag_name, key))?;
    // Recent Windows CUDA releases intentionally split the inference package
    // from NVIDIA's redistributable DLL bundle. Both are required before the
    // server can enumerate a CUDA device.
    let assets = if backend == BackendPreference::Cuda {
        let cudart_asset = release.assets.iter().find(|asset| asset.name.starts_with("cudart-llama-") && asset.name.ends_with(suffix)).ok_or_else(|| format!("llama.cpp {} does not provide the CUDA runtime dependencies for this computer.", release.tag_name))?;
        vec![llama_asset, cudart_asset]
    } else {
        vec![llama_asset]
    };
    let staging = root.join(format!(".{key}-staging"));
    let client = reqwest::blocking::Client::builder().user_agent("AI Harness local runtime manager").build().map_err(|error| error.to_string())?;
    for attempt in 1..=2 {
        if staging.exists() { fs::remove_dir_all(&staging).map_err(|error| format!("Could not refresh the runtime staging directory: {error}"))?; }
        fs::create_dir_all(&staging).map_err(|error| format!("Could not prepare the runtime directory: {error}"))?;
        let result = (|| -> Result<(), String> {
            for asset in &assets {
                let response = client.get(&asset.browser_download_url).send().map_err(|error| format!("Could not download {}: {error}", asset.name))?.error_for_status().map_err(|error| format!("Could not download {}: {error}", asset.name))?;
                let bytes = response.bytes().map_err(|error| format!("Could not read {}: {error}", asset.name))?;
                extract_runtime(&bytes, &staging)?;
            }
            if !has_required_backend_files(&staging, backend) { return Err(format!("The {key} archive did not contain llama-server plus its required backend DLL.")); }
            Ok(())
        })();
        match result {
            Ok(()) => break,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                if attempt == 2 { return Err(format!("Could not verify the {key} runtime after one retry: {error}")); }
            }
        }
    }
    if directory.exists() { fs::remove_dir_all(&directory).map_err(|error| format!("Could not replace the incomplete {key} runtime: {error}"))?; }
    fs::rename(&staging, &directory).map_err(|error| format!("Could not activate the {key} runtime: {error}"))?;
    Ok(RuntimeFiles { server: directory.join(server_name()), directory, release: release.tag_name })
}
