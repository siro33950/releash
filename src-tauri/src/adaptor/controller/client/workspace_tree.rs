use crate::adaptor::presenter::error::AppError;
#[path = "workspace_tree_shared.rs"]
mod shared;
pub(crate) use shared::register_shared;

use std::sync::Arc;

use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::{
    ApproveWorkspaceNodeCommand, RenameWorkspaceSessionNodeCommand,
    ResumeWorkspaceSessionNodeCommand, RetryWorkspaceNodeCommand, WorkspaceNodeCommandUsecase,
};

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
