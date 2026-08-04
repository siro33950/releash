use serde::Serialize;
use tauri::{ipc::Channel, State};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::terminal::{
    GetOrSpawnTerminalV1, TerminalSurfaceAvailabilityV1, TerminalSurfaceInfoV1,
    TerminalSurfaceOwnerV1, TerminalSurfaceStreamItemV1, TerminalSurfaceV1,
};
use crate::usecase::terminal_surface::application::{
    TerminalSurfaceApplication, TerminalSurfaceAttachmentStream,
};
use crate::usecase::terminal_surface::error::UsecaseError;

const PTY_ERROR_CODE_CAP_REACHED: &str = "CAP_REACHED";
const PTY_ERROR_CODE_GENERIC: &str = "PTY_ERROR";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TerminalCommandError {
    pub code: String,
    pub message: String,
}

impl From<UsecaseError> for TerminalCommandError {
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
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
    data: String,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .write(&owner, &data)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn write_paths_to_pty(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
    paths: Vec<String>,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .write_paths(&owner, &paths)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resize_pty(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .resize(&owner, rows, cols)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_terminal_surfaces(state: State<'_, AppState>) -> Vec<TerminalSurfaceInfoV1> {
    state
        .terminal_surface
        .list()
        .into_iter()
        .map(Into::into)
        .collect()
}

#[tauri::command]
pub fn reconcile_terminal_surfaces(
    state: State<'_, AppState>,
    session_keys: Vec<String>,
) -> TerminalSurfaceAvailabilityV1 {
    TerminalSurfaceAvailabilityV1 {
        unavailable_session_keys: state.terminal_surface.reconcile_unavailable(&session_keys),
    }
}

#[tauri::command]
pub fn get_terminal_surface(
    state: State<'_, AppState>,
    owner: TerminalSurfaceOwnerV1,
) -> Result<TerminalSurfaceV1, TerminalCommandError> {
    let owner = owner
        .try_into()
        .map_err(UsecaseError::Gateway)
        .map_err(TerminalCommandError::from)?;
    state
        .terminal_surface
        .get(&owner)
        .map(Into::into)
        .map_err(TerminalCommandError::from)
}

pub(crate) async fn forward_terminal_surface_attachment<F>(
    application: std::sync::Arc<TerminalSurfaceApplication>,
    attachment_id: String,
    mut attachment: TerminalSurfaceAttachmentStream,
    mut send: F,
) where
    F: FnMut(TerminalSurfaceStreamItemV1) -> Result<(), String>,
{
    while let Some(item) = attachment.next().await {
        if send(item.into()).is_err() {
            break;
        }
    }
    application.detach(&attachment_id);
}

#[tauri::command]
pub fn attach_pty(
    state: State<'_, AppState>,
    attachment_id: String,
    owner: TerminalSurfaceOwnerV1,
    on_event: Channel<TerminalSurfaceStreamItemV1>,
) -> Result<(), TerminalCommandError> {
    let owner = owner
        .try_into()
        .map_err(UsecaseError::Gateway)
        .map_err(TerminalCommandError::from)?;
    let application = state.terminal_surface.clone();
    let attachment = application
        .attach(&attachment_id, &owner)
        .map_err(TerminalCommandError::from)?;
    tauri::async_runtime::spawn(forward_terminal_surface_attachment(
        application,
        attachment_id,
        attachment,
        move |item| on_event.send(item).map_err(|error| error.to_string()),
    ));
    Ok(())
}

#[tauri::command]
pub fn detach_pty(state: State<'_, AppState>, attachment_id: String) {
    state.terminal_surface.detach(&attachment_id);
}

#[tauri::command]
pub fn kill_pty(state: State<'_, AppState>, owner: TerminalSurfaceOwnerV1) -> Result<(), String> {
    let owner = owner.try_into()?;
    state
        .terminal_surface
        .kill(&owner)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn get_or_spawn_pty(
    state: State<'_, AppState>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    owner: TerminalSurfaceOwnerV1,
    label: Option<String>,
    startup_command: Option<String>,
) -> Result<GetOrSpawnTerminalV1, TerminalCommandError> {
    let owner = owner
        .try_into()
        .map_err(UsecaseError::Gateway)
        .map_err(TerminalCommandError::from)?;
    state
        .terminal_surface
        .get_or_spawn(rows, cols, cwd, owner, label, startup_command)
        .map(Into::into)
        .map_err(TerminalCommandError::from)
}

#[tauri::command]
pub fn kill_ptys_by_worktree(
    state: State<'_, AppState>,
    worktree_path: String,
) -> Result<(), String> {
    state.terminal_surface.kill_by_worktree(&worktree_path);
    Ok(())
}

#[tauri::command]
pub fn gc_ptys_for_worktree(
    state: State<'_, AppState>,
    worktree_path: String,
    keep_session_keys: Vec<String>,
) -> Result<(), String> {
    state
        .terminal_surface
        .gc_by_worktree(&worktree_path, &keep_session_keys);
    Ok(())
}

#[cfg(test)]
#[path = "commands_test.rs"]
mod commands_tests;
