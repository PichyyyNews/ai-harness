use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiderConfig {
    pub workspace_path: String,
    pub api_base: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
    pub auto_commits: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiderEventPayload {
    pub session_id: String,
    pub event_type: String, // "stdout", "stderr", "diff", "done", "error"
    pub content: String,
}

/// Executes a prompt using the embedded/cloned Aider engine,
/// streaming stdout/stderr back to the Tauri frontend in real time.
pub async fn execute_aider_prompt(
    app: AppHandle,
    session_id: String,
    config: AiderConfig,
    prompt: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let workspace = config.workspace_path.clone();
        if workspace.trim().is_empty() {
            return Err("Workspace path cannot be empty".to_string());
        }

        let api_base = config
            .api_base
            .unwrap_or_else(|| "http://127.0.0.1:8080/v1".to_string());
        let api_key = config
            .api_key
            .unwrap_or_else(|| "sk-dummy-key".to_string());
        let raw_model = config
            .model_name
            .unwrap_or_else(|| "local-model".to_string());

        let model_arg = if raw_model.starts_with("openai/") || raw_model.starts_with("ollama/") {
            raw_model
        } else {
            format!("openai/{}", raw_model)
        };

        let auto_commits = config.auto_commits.unwrap_or(true);

        // Path to embedded Aider repository
        let embedded_aider_dir = Path::new(&workspace).join("aider");
        let python_path = if embedded_aider_dir.exists() {
            embedded_aider_dir.to_string_lossy().to_string()
        } else {
            "c:\\Users\\Newsk\\Downloads\\Aphelion\\aider".to_string()
        };

        tracing::info!(
            session_id = %session_id,
            workspace = %workspace,
            model = %model_arg,
            api_base = %api_base,
            python_path = %python_path,
            "Launching Aider sidecar process"
        );

        // Ensure workspace is a git repository so Aider doesn't search parent directories
        let git_dir = Path::new(&workspace).join(".git");
        if !git_dir.exists() {
            let _ = Command::new("git")
                .arg("init")
                .current_dir(&workspace)
                .output();
        }

        let mut cmd = Command::new("python");
        cmd.current_dir(&workspace);
        cmd.env("OPENAI_API_BASE", &api_base);
        cmd.env("OPENAI_API_KEY", &api_key);
        cmd.env("PYTHONPATH", &python_path);
        cmd.env("PYTHONIOENCODING", "utf-8");
        cmd.env("PYTHONUTF8", "1");

        cmd.arg("-m").arg("aider.main");
        cmd.arg("--model").arg(&model_arg);
        cmd.arg("--edit-format").arg("diff");
        cmd.arg("--no-show-model-warnings");
        cmd.arg("--no-analytics");
        cmd.arg("--message").arg(&prompt);
        cmd.arg("--yes-always");

        if !auto_commits {
            cmd.arg("--no-auto-commits");
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Hide console window on Windows
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!(
                    "Failed to launch Aider engine: {}. Ensure Python is installed.",
                    e
                );
                let _ = app.emit(
                    "aider-event",
                    AiderEventPayload {
                        session_id: session_id.clone(),
                        event_type: "error".to_string(),
                        content: err_msg.clone(),
                    },
                );
                return Err(err_msg);
            }
        };

        let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
        let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

        let app_clone1 = app.clone();
        let session_id1 = session_id.clone();

        let stdout_handle = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut full_output = String::new();

            for line in reader.lines().flatten() {
                full_output.push_str(&line);
                full_output.push('\n');

                let _ = app_clone1.emit(
                    "aider-event",
                    AiderEventPayload {
                        session_id: session_id1.clone(),
                        event_type: "stdout".to_string(),
                        content: line,
                    },
                );
            }
            full_output
        });

        let app_clone2 = app.clone();
        let session_id2 = session_id.clone();

        let stderr_handle = thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                let _ = app_clone2.emit(
                    "aider-event",
                    AiderEventPayload {
                        session_id: session_id2.clone(),
                        event_type: "stderr".to_string(),
                        content: line,
                    },
                );
            }
        });

        let output_text = stdout_handle.join().unwrap_or_default();
        let _ = stderr_handle.join();

        let status = child.wait().map_err(|e| e.to_string())?;

        let event_type = if status.success() { "done" } else { "error" };

        let _ = app.emit(
            "aider-event",
            AiderEventPayload {
                session_id: session_id.clone(),
                event_type: event_type.to_string(),
                content: format!("Aider process exited with code: {:?}", status.code()),
            },
        );

        Ok(output_text)
    })
    .await
    .map_err(|e| e.to_string())?
}
