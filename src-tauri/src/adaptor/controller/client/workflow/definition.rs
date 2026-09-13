use crate::adaptor::controller::api::protocol::client::{
    save_workflow_source_result_dto::Variant, SaveWorkflowDiagnostics, SaveWorkflowSourceResultDto,
    SaveWorkflowSuccess,
};
use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::dto::{
    workflow_to_dto, workflow_to_dto_with_source_format, WorkflowDto, WorkflowSummaryDto,
};
use crate::usecase::workflow::ports::WorkflowSourceSaveError;

pub(crate) async fn list_workflows_shared(
    state: &AppState,
) -> Result<Vec<WorkflowSummaryDto>, String> {
    let read = state.workflow_usecase.read_usecase();
    tokio::task::spawn_blocking(move || read.list_workflow_summaries().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub(crate) async fn get_workflow_shared(
    state: &AppState,
    name: String,
) -> Result<WorkflowDto, String> {
    let query = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        query
            .get_workflow(&name)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("ワークフロー '{name}' が見つかりません"))
            .and_then(|workflow| {
                query
                    .get_workflow_source_format(&name)
                    .map(|format| workflow_to_dto_with_source_format(&workflow, format))
                    .map_err(|e| e.to_string())
            })
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

pub(crate) async fn get_workflow_source_shared(
    state: &AppState,
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

pub(crate) async fn save_workflow_source_shared(
    state: &AppState,
    source: String,
    original_name: Option<String>,
) -> Result<SaveWorkflowSourceResultDto, String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        match usecase.save_workflow_source_with_diagnostics(&source, original_name.as_deref()) {
            Ok(workflow) => Ok(SaveWorkflowSourceResultDto {
                variant: Some(Variant::Success(SaveWorkflowSuccess {
                    ok: Some(true),
                    workflow: Some(workflow_to_dto(&workflow).try_into()?),
                })),
            }),
            Err(WorkflowSourceSaveError::Diagnostics(diagnostics)) => {
                Ok(SaveWorkflowSourceResultDto {
                    variant: Some(Variant::Diagnostics(SaveWorkflowDiagnostics {
                        ok: Some(false),
                        diagnostics: Some(diagnostics.try_into()?),
                        error: Some("workflow_diagnostics".to_string()),
                    })),
                })
            }
            Err(WorkflowSourceSaveError::Workflow(error)) => Err(error.to_string()),
        }
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

pub(crate) async fn delete_workflow_shared(state: &AppState, name: String) -> Result<(), String> {
    let usecase = state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || usecase.delete_workflow(&name).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub(crate) fn open_workflow_in_editor_shared(state: &AppState, name: String) -> Result<(), String> {
    state
        .workflow_usecase
        .open_workflow_in_editor(&name)
        .map_err(|e| e.to_string())
}

pub(crate) async fn duplicate_workflow_shared(
    state: &AppState,
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
