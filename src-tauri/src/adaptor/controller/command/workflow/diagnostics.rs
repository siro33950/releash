use std::collections::HashMap;
use std::sync::Arc;

use crate::adaptor::controller::state::AppState;

#[tauri::command]
pub async fn diagnose_all_cmd(
    state: tauri::State<'_, AppState>,
    dir: Option<String>,
) -> Result<serde_json::Value, String> {
    diagnose_all_impl(&state.workflow_usecase, dir).await
}

/// 内部経路。Tauri command 側は injected state を受け取り本関数に委譲する。
pub(crate) async fn diagnose_all_impl(
    usecase: &Arc<crate::usecase::workflow::WorkflowUsecase>,
    dir: Option<String>,
) -> Result<serde_json::Value, String> {
    let target =
        crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::from_optional_directory(dir)
            .map_err(|e| e.to_string())?;
    let usecase = usecase.clone();
    tokio::task::spawn_blocking(move || usecase.diagnose_all(target).map_err(|e| e.to_string()))
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
