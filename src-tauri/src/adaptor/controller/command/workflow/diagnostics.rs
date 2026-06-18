use std::collections::HashMap;

use crate::adaptor::controller::state::AppState;

#[tauri::command]
pub async fn diagnose_all_cmd(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || usecase.diagnose_all().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn render_facet_preview(
    state: tauri::State<'_, AppState>,
    content: String,
    sample_values: HashMap<String, String>,
) -> Result<String, String> {
    Ok(state
        .workflow_usecase
        .render_facet_preview(&content, &sample_values))
}

#[tauri::command]
pub fn get_automation_config_dir(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state
        .workflow_usecase
        .automation_config_dir()
        .map_err(|e| e.to_string())
}
