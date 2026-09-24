use crate::other::AppError;
#[path = "workspace_tree_shared.rs"]
mod shared;
pub(crate) use shared::register_shared;

use std::sync::Arc;

use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::{
    ApproveWorkspaceNodeCommand, RenameWorkspaceSessionNodeCommand,
    ResumeWorkspaceSessionNodeCommand, RetryWorkspaceNodeCommand, WorkspaceNodeCommandUsecase,
    WorkspaceNodeDetailDto, WorkspaceTreeSelectionSnapshotDto, WorkspaceTreeSnapshotDto,
    WorkspaceWorkflowHistoryItemDto,
};

pub(crate) async fn list_workspace_worktree_nodes_shared(
    app_state: &AppState,
    worktree_path: String,
) -> Result<WorkspaceTreeSnapshotDto, AppError> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .list_workspace_tree_nodes(&worktree_path)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn get_workspace_tree_selection_reconciliation_shared(
    app_state: &AppState,
    worktree_path: String,
    selected_node_id: String,
) -> Result<WorkspaceTreeSelectionSnapshotDto, AppError> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .get_workspace_tree_selection_reconciliation(&worktree_path, &selected_node_id)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn list_workspace_workflow_history_shared(
    app_state: &AppState,
    worktree_path: String,
) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, AppError> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    let history = tokio::task::spawn_blocking(move || {
        workflow_usecase
            .list_workspace_workflow_history(&worktree_path)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))??;
    Ok(history)
}

pub(crate) async fn get_workspace_node_detail_shared(
    app_state: &AppState,
    worktree_path: String,
    node_id: String,
) -> Result<Option<WorkspaceNodeDetailDto>, AppError> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .get_workspace_node_detail(&worktree_path, &node_id)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn get_workspace_session_node_id_shared(
    app_state: &AppState,
    worktree_path: String,
    session_id: String,
) -> Result<Option<String>, AppError> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .get_workspace_session_node_id(&worktree_path, &session_id)
            .map_err(AppError::from_failure)
    })
    .await
    .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn approve_workspace_node_shared(
    usecase: &Arc<WorkspaceNodeCommandUsecase>,
    worktree_path: String,
    node_id: String,
) -> Result<(), AppError> {
    usecase
        .approve_workspace_node(ApproveWorkspaceNodeCommand {
            worktree_path,
            node_id,
        })
        .await
        .map_err(AppError::from_failure)
}

pub(crate) async fn retry_workspace_node_shared(
    usecase: &Arc<WorkspaceNodeCommandUsecase>,
    worktree_path: String,
    node_id: String,
) -> Result<(), AppError> {
    usecase
        .retry_workspace_node(RetryWorkspaceNodeCommand {
            worktree_path,
            node_id,
        })
        .await
        .map_err(AppError::from_failure)
}

pub(crate) async fn resume_workspace_session_node_shared(
    usecase: &Arc<WorkspaceNodeCommandUsecase>,
    worktree_path: String,
    node_id: String,
) -> Result<(), AppError> {
    usecase
        .resume_workspace_session_node(ResumeWorkspaceSessionNodeCommand {
            worktree_path,
            node_id,
        })
        .await
        .map_err(AppError::from_failure)
}

pub(crate) async fn rename_workspace_session_node_shared(
    usecase: &Arc<WorkspaceNodeCommandUsecase>,
    worktree_path: String,
    node_id: String,
    name: String,
) -> Result<(), AppError> {
    usecase
        .rename_workspace_session_node(RenameWorkspaceSessionNodeCommand {
            worktree_path,
            node_id,
            name,
        })
        .await
        .map_err(AppError::from_failure)
}

pub(crate) async fn archive_workspace_workflow_execution_shared(
    app_state: &AppState,
    runtime: &crate::usecase::workflow::WorkflowRuntimeUsecase,
    worktree_path: String,
    execution_id: String,
) -> Result<(), AppError> {
    app_state
        .workflow_usecase
        .archive_workspace_workflow_execution(runtime, &worktree_path, &execution_id)
        .await
        .map_err(AppError::from_failure)
}

pub(crate) async fn restore_workspace_workflow_execution_shared(
    app_state: &AppState,
    runtime: &crate::usecase::workflow::WorkflowRuntimeUsecase,
    worktree_path: String,
    execution_id: String,
) -> Result<(), AppError> {
    app_state
        .workflow_usecase
        .restore_workspace_workflow_execution(runtime, &worktree_path, &execution_id)
        .await
        .map_err(AppError::from_failure)
}
