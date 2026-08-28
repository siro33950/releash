use std::sync::Arc;

use tauri::State;

use crate::adaptor::controller::state::AppState;
use crate::usecase::workflow::{
    ApproveWorkspaceNodeCommand, RetryWorkspaceNodeCommand, WorkspaceNodeCommandUsecase,
    WorkspaceNodeDetailDto, WorkspaceTreeSelectionSnapshotDto, WorkspaceTreeSnapshotDto,
    WorkspaceWorkflowHistoryItemDto,
};

pub(super) const COMMAND_NAMES: &[&str] = &[
    "list_workspace_worktree_nodes",
    "get_workspace_tree_selection_reconciliation",
    "list_workspace_workflow_history",
    "get_workspace_node_detail",
    "get_workspace_session_node_id",
    "approve_workspace_node",
    "retry_workspace_node",
    "archive_workspace_workflow_execution",
    "restore_workspace_workflow_execution",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        list_workspace_worktree_nodes,
        get_workspace_tree_selection_reconciliation,
        list_workspace_workflow_history,
        get_workspace_node_detail,
        get_workspace_session_node_id,
        approve_workspace_node,
        retry_workspace_node,
        archive_workspace_workflow_execution,
        restore_workspace_workflow_execution,
    ]
}

#[tauri::command]
pub async fn list_workspace_worktree_nodes(
    app_state: State<'_, AppState>,
    worktree_path: String,
) -> Result<WorkspaceTreeSnapshotDto, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .list_workspace_tree_nodes(&worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workspace_tree_selection_reconciliation(
    app_state: State<'_, AppState>,
    worktree_path: String,
    selected_node_id: String,
) -> Result<WorkspaceTreeSelectionSnapshotDto, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .get_workspace_tree_selection_reconciliation(&worktree_path, &selected_node_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn list_workspace_workflow_history(
    app_state: State<'_, AppState>,
    worktree_path: String,
) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    let history = tokio::task::spawn_blocking(move || {
        workflow_usecase
            .list_workspace_workflow_history(&worktree_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))??;
    Ok(history)
}

#[tauri::command]
pub async fn get_workspace_node_detail(
    app_state: State<'_, AppState>,
    worktree_path: String,
    node_id: String,
) -> Result<Option<WorkspaceNodeDetailDto>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .get_workspace_node_detail(&worktree_path, &node_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn get_workspace_session_node_id(
    app_state: State<'_, AppState>,
    worktree_path: String,
    session_id: String,
) -> Result<Option<String>, String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .get_workspace_session_node_id(&worktree_path, &session_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn approve_workspace_node(
    usecase: State<'_, Arc<WorkspaceNodeCommandUsecase>>,
    worktree_path: String,
    node_id: String,
) -> Result<(), String> {
    usecase
        .approve_workspace_node(ApproveWorkspaceNodeCommand {
            worktree_path,
            node_id,
        })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn retry_workspace_node(
    usecase: State<'_, Arc<WorkspaceNodeCommandUsecase>>,
    worktree_path: String,
    node_id: String,
) -> Result<(), String> {
    usecase
        .retry_workspace_node(RetryWorkspaceNodeCommand {
            worktree_path,
            node_id,
        })
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn archive_workspace_workflow_execution(
    app_state: State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .archive_workspace_workflow_execution(&worktree_path, &execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

#[tauri::command]
pub async fn restore_workspace_workflow_execution(
    app_state: State<'_, AppState>,
    worktree_path: String,
    execution_id: String,
) -> Result<(), String> {
    let workflow_usecase = app_state.workflow_usecase.clone();
    tokio::task::spawn_blocking(move || {
        workflow_usecase
            .restore_workspace_workflow_execution(&worktree_path, &execution_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}
