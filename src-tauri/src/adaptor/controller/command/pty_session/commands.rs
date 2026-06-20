use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway;
use crate::domain::pty_session::services::parse_pty_kind;
use crate::usecase::pty_session::dto::{GetOrSpawnPtyResult, PtySessionInfo};
use crate::ws_bridge::WsBroadcaster;

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn spawn_pty(
    app: AppHandle,
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    worktree_path: Option<String>,
    label: Option<String>,
    kind: Option<String>,
) -> Result<u64, String> {
    let pty_kind = parse_pty_kind(kind.as_deref());
    let (pty_id, _session_key) = crate::usecase::pty_session::spawn_usecase::spawn(
        state.inner().as_ref(),
        &app,
        rows,
        cols,
        cwd,
        worktree_path,
        label,
        pty_kind,
    )
    .map_err(|e| e.to_string())?;
    Ok(pty_id)
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
pub fn list_pty_sessions(state: State<'_, Arc<PtySessionRuntimeGateway>>) -> Vec<PtySessionInfo> {
    crate::usecase::pty_session::query_service::list(state.inner().as_ref())
}

#[tauri::command]
pub fn kill_pty(
    app: AppHandle,
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    pty_id: u64,
) -> Result<(), String> {
    crate::usecase::pty_session::lifecycle_usecase::kill(state.inner().as_ref(), pty_id)
        .map_err(|e| e.to_string())?;
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        ws.remove_pty_output_buffer(pty_id);
    }
    Ok(())
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
    kind: Option<String>,
) -> Result<GetOrSpawnPtyResult, String> {
    let pty_kind = parse_pty_kind(kind.as_deref());
    crate::usecase::pty_session::spawn_usecase::get_or_spawn(
        state.inner().as_ref(),
        &app,
        rows,
        cols,
        cwd,
        session_key,
        worktree_path,
        label,
        pty_kind,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn kill_ptys_by_worktree(
    app: AppHandle,
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    worktree_path: String,
) -> Result<(), String> {
    let killed_ids = crate::usecase::pty_session::lifecycle_usecase::kill_by_worktree(
        state.inner().as_ref(),
        &worktree_path,
    );
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        for pty_id in killed_ids {
            ws.remove_pty_output_buffer(pty_id);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn gc_ptys_for_worktree(
    app: AppHandle,
    state: State<'_, Arc<PtySessionRuntimeGateway>>,
    worktree_path: String,
    keep_session_keys: Vec<String>,
) -> Result<(), String> {
    let killed_ids = crate::usecase::pty_session::lifecycle_usecase::gc_by_worktree(
        state.inner().as_ref(),
        &worktree_path,
        &keep_session_keys,
    );
    if let Some(ws) = app.try_state::<Arc<WsBroadcaster>>() {
        for pty_id in killed_ids {
            ws.remove_pty_output_buffer(pty_id);
        }
    }
    Ok(())
}
