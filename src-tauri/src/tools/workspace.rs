use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

fn resolve_workspace_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn safe_canonical_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let clean = relative.trim_start_matches('/').trim_start_matches('\\');
    let target = root.join(clean);
    
    // Failsafe path traversal check
    if target.components().any(|c| c == std::path::Component::ParentDir) {
        return Err("Access denied: path traversal attempt.".to_string());
    }
    
    Ok(target)
}

pub fn list_workspace_files(_app: &AppHandle, subpath: Option<&str>) -> Result<String, String> {
    let root = resolve_workspace_root();
    let target_dir = if let Some(path) = subpath {
        safe_canonical_path(&root, path)?
    } else {
        root.clone()
    };

    if !target_dir.exists() {
        return Err(format!("Directory does not exist: {}", target_dir.display()));
    }

    let entries = fs::read_dir(&target_dir)
        .map_err(|e| format!("Could not read directory: {e}"))?;

    let mut items = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        // Hide node_modules and .git by default
        if name == "node_modules" || name == ".git" || name == "target" {
            continue;
        }

        let is_dir = path.is_dir();
        let size_bytes = if is_dir { 0 } else { fs::metadata(&path).map(|m| m.len()).unwrap_or(0) };

        items.push(json!({
            "name": name,
            "isDir": is_dir,
            "sizeBytes": size_bytes
        }));
    }

    Ok(json!({
        "root": root.display().to_string(),
        "directory": target_dir.display().to_string(),
        "count": items.len(),
        "items": items
    })
    .to_string())
}

pub fn read_workspace_file(_app: &AppHandle, relative_path: &str) -> Result<String, String> {
    let root = resolve_workspace_root();
    let target_file = safe_canonical_path(&root, relative_path)?;

    if !target_file.is_file() {
        return Err(format!("File not found or is a directory: {}", target_file.display()));
    }

    let content = fs::read_to_string(&target_file)
        .map_err(|e| format!("Could not read file (binary or unreadable): {e}"))?;

    let truncated = content.chars().take(8_000).collect::<String>();
    let was_truncated = content.len() > truncated.len();

    Ok(json!({
        "path": relative_path,
        "totalChars": content.len(),
        "wasTruncated": was_truncated,
        "content": truncated
    })
    .to_string())
}
