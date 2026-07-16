use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::dto::{workflow_to_dto, WorkflowDto, WorkflowSummaryDto};
use crate::usecase::workflow::ports::WorkflowSourceSaveError;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SaveWorkflowSourceResultDto {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow: Option<WorkflowDto>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tauri::command]
pub async fn list_workflows(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WorkflowSummaryDto>, String> {
    let read = state.workflow_usecase.read_usecase();
    tokio::task::spawn_blocking(move || read.list_workflow_summaries().map_err(|e| e.to_string()))
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
pub async fn get_workflow_source(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<String, String> {
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .get_workflow_source(&name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("ワークフロー '{name}' が見つかりません"))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn save_workflow_source(
    state: tauri::State<'_, AppState>,
    source: String,
    original_name: Option<String>,
) -> Result<SaveWorkflowSourceResultDto, String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        match usecase.save_workflow_source_with_diagnostics(&source, original_name.as_deref()) {
            Ok(workflow) => Ok(SaveWorkflowSourceResultDto {
                ok: true,
                workflow: Some(workflow_to_dto(&workflow)),
                diagnostics: Vec::new(),
                error: None,
            }),
            Err(WorkflowSourceSaveError::Diagnostics(diagnostics)) => {
                Ok(SaveWorkflowSourceResultDto {
                    ok: false,
                    workflow: None,
                    diagnostics,
                    error: Some("workflow_diagnostics".to_string()),
                })
            }
            Err(WorkflowSourceSaveError::Workflow(error)) => Err(error.to_string()),
        }
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
