use std::sync::Arc;

use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
use crate::domain::workspace_state::WorkspaceState;

#[tauri::command]
pub fn load_workspace_state(
    store: tauri::State<'_, Arc<WorkspaceStateStore>>,
    worktree_name: String,
    worktree_root: String,
) -> Option<WorkspaceState> {
    crate::usecase::workspace_state::query_service::load_workspace_state(
        store.inner().as_ref(),
        &worktree_name,
        &worktree_root,
    )
}

#[tauri::command]
pub fn save_workspace_state(
    store: tauri::State<'_, Arc<WorkspaceStateStore>>,
    worktree_name: String,
    state: WorkspaceState,
) -> Result<(), String> {
    crate::usecase::workspace_state::usecase::save_workspace_state(
        store.inner().as_ref(),
        &worktree_name,
        state,
    )
}
