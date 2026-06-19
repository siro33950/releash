use std::sync::Arc;

use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
use crate::usecase::workspace_state::dto::WorkspaceStateDto;

#[tauri::command]
pub fn load_workspace_state(
    store: tauri::State<'_, Arc<WorkspaceStateStore>>,
    worktree_name: String,
    worktree_root: String,
) -> Option<WorkspaceStateDto> {
    crate::usecase::workspace_state::query_service::load_workspace_state(
        store.inner().as_ref(),
        &worktree_name,
        &worktree_root,
    )
    .map(WorkspaceStateDto::from)
}

#[tauri::command]
pub fn save_workspace_state(
    store: tauri::State<'_, Arc<WorkspaceStateStore>>,
    worktree_name: String,
    state: WorkspaceStateDto,
) -> Result<(), String> {
    crate::usecase::workspace_state::usecase::save_workspace_state(
        store.inner().as_ref(),
        &worktree_name,
        state.into(),
    )
    .map_err(|e| e.to_string())
}
