use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::dto::{
    workflow_from_dto, workflow_summary_to_dto, workflow_to_dto, WorkflowDto, WorkflowSummaryDto,
};

#[tauri::command]
pub async fn list_workflows(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WorkflowSummaryDto>, String> {
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        let running_names: Vec<String> = query
            .list_runs(crate::domain::workflow::RunListFilter {
                status: Some(crate::domain::workflow::RunStatusFilter::Active),
                worktree_path: None,
            })
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|run| run.workflow_name)
            .collect();
        query
            .list_workflows(&running_names)
            .map(|summaries| summaries.into_iter().map(workflow_summary_to_dto).collect())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workflow(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<WorkflowDto, String> {
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .get_workflow(&name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("ワークフロー '{name}' が見つかりません"))
            .map(|workflow| workflow_to_dto(&workflow))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_workflow(
    state: tauri::State<'_, AppState>,
    workflow: WorkflowDto,
    original_name: Option<String>,
) -> Result<(), String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        let workflow = workflow_from_dto(workflow);
        usecase
            .save_workflow(workflow, original_name.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn delete_workflow(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || usecase.delete_workflow(&name).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub fn open_workflow_in_editor(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    state
        .workflow_usecase
        .open_workflow_in_editor(&name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn duplicate_workflow(
    state: tauri::State<'_, AppState>,
    source_name: String,
    new_name: String,
) -> Result<(), String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        usecase
            .duplicate_workflow(&source_name, &new_name)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
