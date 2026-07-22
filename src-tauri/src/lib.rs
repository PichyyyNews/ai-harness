mod commands;
mod engine;
mod language_classifier;
mod models;
mod sessions;
mod state;
mod web_search;
pub mod tools;

pub fn run() {
    tauri::Builder::default()
        .manage(state::EngineState::default())
        .invoke_handler(tauri::generate_handler![
            commands::models::model_storage_path,
            commands::models::search_models,
            commands::models::get_model_details,
            commands::models::list_installed_models,
            commands::models::install_model,
            commands::engine::detect_hardware,
            commands::engine::get_engine_settings,
            commands::engine::save_engine_settings,
            commands::engine::start_engine,
            commands::engine::generate_chat,
            commands::engine::stop_generation,
            commands::engine::stop_engine,
            commands::engine::trigger_session_end_memory,
            commands::sessions::create_session,
            commands::sessions::list_sessions,
            commands::sessions::get_session,
            commands::sessions::rename_session,
            commands::sessions::delete_session,
            commands::sessions::generate_session_title,
            commands::window::minimize_window,
            commands::window::toggle_maximize_window,
            commands::window::close_window,
            commands::window::start_window_dragging,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AI Harness");
}
