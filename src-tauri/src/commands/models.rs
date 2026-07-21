use crate::models::{self, CatalogModel, InstallRequest, InstalledModel};
use tauri::AppHandle;

#[tauri::command]
pub fn model_storage_path(app: AppHandle) -> Result<String, String> {
    Ok(models::paths::models_directory(&app)?.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn search_models(query: String) -> Result<Vec<CatalogModel>, String> {
    models::catalog::search(query).await
}

#[tauri::command]
pub async fn get_model_details(repo_id: String) -> Result<CatalogModel, String> {
    models::catalog::details(repo_id).await
}

#[tauri::command]
pub fn list_installed_models(app: AppHandle) -> Result<Vec<InstalledModel>, String> {
    models::registry::list(&app)
}

#[tauri::command]
pub async fn install_model(app: AppHandle, request: InstallRequest) -> Result<InstalledModel, String> {
    let local_file = models::paths::local_file_name(&request.repo_id, &request.file_name)?;
    let target = models::paths::model_path(&app, &local_file)?;
    let installed = models::installer::install(&app, request, &target).await?;
    models::registry::upsert(&app, installed.clone())?;
    Ok(installed)
}
