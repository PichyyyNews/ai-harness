#[tauri::command]
pub fn minimize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|error| format!("Could not minimize window: {error}"))
}

#[tauri::command]
pub fn toggle_maximize_window(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().map_err(|error| format!("Could not read window maximization state: {error}"))? {
        window.unmaximize().map_err(|error| format!("Could not restore window size: {error}"))
    } else {
        window.maximize().map_err(|error| format!("Could not maximize window: {error}"))
    }
}

#[tauri::command]
pub fn close_window(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::state::EngineState>,
) -> Result<(), String> {
    // Tauri can terminate the process immediately after the last window is
    // closed, before managed state destructors run. Stop both llama.cpp
    // sidecars explicitly so a restart cannot leave orphan inference servers.
    crate::commands::engine::stop_engine(state)?;
    window.close().map_err(|error| format!("Could not close window: {error}"))
}

#[tauri::command]
pub fn start_window_dragging(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|error| format!("Could not start window drag: {error}"))
}
