use std::sync::Arc;

use tauri::State;

use crate::usecase::agent_session::status::{
    AgentStatusCenter, SessionStatus, WorkspaceStatus, WorktreeStepStatusView,
};

#[tauri::command]
pub fn get_session_status(
    chat_session_id: String,
    center: State<'_, Arc<AgentStatusCenter>>,
) -> Option<SessionStatus> {
    center.get_session(&chat_session_id)
}

#[tauri::command]
pub fn get_workspace_status(
    worktree_id: String,
    center: State<'_, Arc<AgentStatusCenter>>,
) -> Option<WorkspaceStatus> {
    center.get_workspace(&worktree_id)
}

#[tauri::command]
pub fn list_workspace_statuses(center: State<'_, Arc<AgentStatusCenter>>) -> Vec<WorkspaceStatus> {
    center.list_workspaces()
}

#[tauri::command]
pub fn list_session_statuses(center: State<'_, Arc<AgentStatusCenter>>) -> Vec<SessionStatus> {
    center.list_sessions()
}

#[tauri::command]
pub fn query_worktree_step_statuses(
    worktree_path: String,
    center: State<'_, Arc<AgentStatusCenter>>,
) -> WorktreeStepStatusView {
    center.query_worktree_step_statuses(&worktree_path)
}

#[tauri::command]
pub fn sync_worktree_step_statuses(
    worktree_path: String,
    app: tauri::AppHandle,
    center: State<'_, Arc<AgentStatusCenter>>,
) {
    crate::adaptor::presenter::agent_status::emit_worktree_step_status_snapshot(
        &app,
        center.inner().as_ref(),
        &worktree_path,
    );
}
