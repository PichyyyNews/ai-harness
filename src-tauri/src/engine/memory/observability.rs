use super::MemoryLayerCounts;
use chrono::Utc;
use serde::Serialize;
use std::{fs::OpenOptions, io::Write};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemoryDebugEvent<'a> {
    timestamp_utc: String,
    session_id: &'a str,
    event: &'a str,
    counts: MemoryLayerCounts,
    primary_tokens: u32,
    reminder_tokens: u32,
    primary_preview: &'a str,
    reminder_preview: &'a str,
}

pub fn log_prompt_assembly(
    app: &AppHandle,
    session_id: &str,
    counts: MemoryLayerCounts,
    primary_tokens: u32,
    reminder_tokens: u32,
    primary: &str,
    reminder: &str,
) {
    let event = MemoryDebugEvent {
        timestamp_utc: Utc::now().to_rfc3339(),
        session_id,
        event: "prompt_assembly",
        counts,
        primary_tokens,
        reminder_tokens,
        primary_preview: truncate(primary, 2_400),
        reminder_preview: truncate(reminder, 800),
    };
    let Ok(line) = serde_json::to_string(&event) else {
        return;
    };
    let Ok(directory) = app.path().app_data_dir().map(|path| path.join("logs")) else {
        return;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("memory-debug.jsonl"))
    {
        let _ = writeln!(file, "{line}");
    }
}

pub fn log_extraction_counts(
    app: &AppHandle,
    session_id: &str,
    proposed_constraints: usize,
    mid_term_updates: usize,
    proposed_facts: usize,
    stored_facts: usize,
) {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ExtractionEvent<'a> {
        timestamp_utc: String,
        session_id: &'a str,
        event: &'a str,
        proposed_constraints: usize,
        mid_term_updates: usize,
        proposed_facts: usize,
        stored_facts: usize,
    }
    let event = ExtractionEvent {
        timestamp_utc: Utc::now().to_rfc3339(),
        session_id,
        event: "extraction",
        proposed_constraints,
        mid_term_updates,
        proposed_facts,
        stored_facts,
    };
    let Ok(line) = serde_json::to_string(&event) else {
        return;
    };
    let Ok(directory) = app.path().app_data_dir().map(|path| path.join("logs")) else {
        return;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("memory-debug.jsonl"))
    {
        let _ = writeln!(file, "{line}");
    }
}

pub fn log_error(app: &AppHandle, session_id: &str, stage: &str, detail: &str) {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ErrorEvent<'a> {
        timestamp_utc: String,
        session_id: &'a str,
        event: &'static str,
        stage: &'a str,
        detail: &'a str,
    }
    let event = ErrorEvent {
        timestamp_utc: Utc::now().to_rfc3339(),
        session_id,
        event: "memory_error",
        stage,
        detail,
    };
    let Ok(line) = serde_json::to_string(&event) else {
        return;
    };
    let Ok(directory) = app.path().app_data_dir().map(|path| path.join("logs")) else {
        return;
    };
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("memory-debug.jsonl"))
    {
        let _ = writeln!(file, "{line}");
    }
}

fn truncate(value: &str, max_chars: usize) -> &str {
    value
        .char_indices()
        .nth(max_chars)
        .map(|(index, _)| &value[..index])
        .unwrap_or(value)
}
