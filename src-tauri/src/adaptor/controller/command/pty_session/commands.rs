use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway;
use crate::usecase::pty_session::dto::{
    GetOrSpawnPtyResult, GetPtyBufferedOutputResult, PtySessionAvailability, PtySessionInfo,
};
use crate::usecase::pty_session::error::UsecaseError;

const PTY_ERROR_CODE_CAP_REACHED: &str = "CAP_REACHED";
const PTY_ERROR_CODE_GENERIC: &str = "PTY_ERROR";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PtyCommandError {
    pub code: String,
    pub message: String,
}

impl From<UsecaseError> for PtyCommandError {
    fn from(error: UsecaseError) -> Self {
        let code = match error {
            UsecaseError::CapReached(_) => PTY_ERROR_CODE_CAP_REACHED,
            UsecaseError::Gateway(_) => PTY_ERROR_CODE_GENERIC,
        };
        Self {
            code: code.to_string(),
            message: error.to_string(),
        }
    }
}

#[tauri::command]
pub fn write_pty(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    pty_id: u64,
    data: String,
) -> Result<(), String> {
    crate::usecase::pty_session::io_usecase::write(state.inner().as_ref(), pty_id, &data)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_paths_to_pty(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    pty_id: u64,
    paths: Vec<String>,
) -> Result<(), String> {
    crate::usecase::pty_session::io_usecase::write_paths(state.inner().as_ref(), pty_id, &paths)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resize_pty(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    pty_id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    crate::usecase::pty_session::io_usecase::resize(state.inner().as_ref(), pty_id, rows, cols)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_pty_sessions(state: State<'_, AppState>) -> Vec<PtySessionInfo> {
    state.pty_session_read_usecase.list()
}

#[tauri::command]
pub fn reconcile_pty_sessions(
    state: State<'_, AppState>,
    session_keys: Vec<String>,
) -> PtySessionAvailability {
    state
        .pty_session_read_usecase
        .reconcile_unavailable(&session_keys)
}

#[tauri::command]
pub fn get_pty_buffered_output(
    state: State<'_, AppState>,
    session_key: String,
    worktree_path: String,
) -> Result<GetPtyBufferedOutputResult, PtyCommandError> {
    state
        .pty_session_read_usecase
        .get_buffered_output(&session_key, &worktree_path)
        .map_err(PtyCommandError::from)
}

#[tauri::command]
pub fn kill_pty(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    pty_id: u64,
) -> Result<(), String> {
    crate::usecase::pty_session::lifecycle_usecase::kill(state.inner().as_ref(), pty_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    session_key: Option<String>,
    worktree_path: String,
    label: Option<String>,
) -> Result<GetOrSpawnPtyResult, PtyCommandError> {
    crate::usecase::pty_session::spawn_usecase::get_or_spawn(
        state.inner().as_ref(),
        &app,
        rows,
        cols,
        cwd,
        session_key,
        worktree_path,
        label,
    )
    .map_err(PtyCommandError::from)
}

#[tauri::command]
pub fn kill_ptys_by_worktree(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    worktree_path: String,
) -> Result<(), String> {
    crate::usecase::pty_session::lifecycle_usecase::kill_by_worktree(
        state.inner().as_ref(),
        &worktree_path,
    );
    Ok(())
}

#[tauri::command]
pub fn gc_ptys_for_worktree(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    worktree_path: String,
    keep_session_keys: Vec<String>,
) -> Result<(), String> {
    crate::usecase::pty_session::lifecycle_usecase::gc_by_worktree(
        state.inner().as_ref(),
        &worktree_path,
        &keep_session_keys,
    );
    Ok(())
}

#[tauri::command]
pub fn register_active_terminal(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    worktree_path: String,
    session_key: String,
    active_token: String,
) -> Result<(), String> {
    crate::usecase::pty_session::lifecycle_usecase::register_active_terminal(
        state.inner().as_ref(),
        &worktree_path,
        &session_key,
        &active_token,
    );
    Ok(())
}

#[tauri::command]
pub fn unregister_active_terminal(
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    worktree_path: String,
    session_key: String,
    active_token: String,
) -> Result<(), String> {
    crate::usecase::pty_session::lifecycle_usecase::unregister_active_terminal(
        state.inner().as_ref(),
        &worktree_path,
        &session_key,
        &active_token,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty_session::entities::PtySpawnReservationError;

    #[test]
    fn cap_reached_errors_map_to_stable_command_code() {
        let worktree_error = UsecaseError::from(PtySpawnReservationError::WorktreeCapReached(
            "/repo".to_string(),
        ));
        let total_error = UsecaseError::from(PtySpawnReservationError::TotalCapReached);

        assert_eq!(
            PtyCommandError::from(worktree_error).code,
            PTY_ERROR_CODE_CAP_REACHED
        );
        assert_eq!(
            PtyCommandError::from(total_error).code,
            PTY_ERROR_CODE_CAP_REACHED
        );
    }
}
