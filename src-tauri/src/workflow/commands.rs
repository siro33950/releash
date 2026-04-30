use super::schema::{Summary, Workflow};
use super::storage;
use crate::config::AppConfig;
use std::sync::Arc;

#[tauri::command]
pub async fn list_workflows() -> Result<Vec<Summary>, String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || storage::list_workflows(&dir).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow(name: String) -> Result<Workflow, String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        super::validation::validate_name(&name).map_err(|e| e.to_string())?;
        let file_path = dir.join(format!("{name}.yml"));
        storage::load_workflow(&file_path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_workflow(workflow: Workflow) -> Result<(), String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        storage::save_workflow(&dir, &workflow).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_workflow(name: String) -> Result<(), String> {
    let dir = storage::workflows_dir();
    tokio::task::spawn_blocking(move || {
        storage::delete_workflow(&dir, &name).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn open_workflow_in_editor(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppConfig>>,
    name: String,
) -> Result<(), String> {
    let dir = storage::workflows_dir();
    let file_path = storage::resolve_workflow_path(&dir, &name).map_err(|e| e.to_string())?;

    let path_str = file_path.to_string_lossy().to_string();
    let config = state.get_config()?;
    crate::external_editor::open_path_with_opener(
        &app,
        &path_str,
        &config.app.external_editor,
        "ワークフロー",
    )
}

#[tauri::command]
pub async fn list_prompt_templates() -> Result<Vec<Summary>, String> {
    let dir = storage::prompts_dir();
    tokio::task::spawn_blocking(move || storage::list_prompts(&dir).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}
