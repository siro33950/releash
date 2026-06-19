use std::sync::Arc;

use tokio::sync::Mutex;

use crate::adaptor::controller::handler::shared::{join_error_msg, no_worktree_selected_error};
use crate::adaptor::protocol::pty::{
    PtyInput, PtyKillRequest, PtyKillResponse, PtyOutputMsg, PtyOutputRequest, PtyReady,
    PtySpawnRequest, PtySpawnResponse,
};
use crate::domain::pty_session::PtyKind;
use crate::protocol::{ErrorMsg, WsMessage};
use crate::usecase::pty_session::{io_usecase, lifecycle_usecase, query_service, spawn_usecase};
use crate::ws_server::WsServerState;

pub(crate) fn handle_pty_input(input: &PtyInput, state: &WsServerState) -> Option<WsMessage> {
    if let Some(gateway) = state.pty_session_runtime_gateway() {
        if let Err(e) = io_usecase::write(gateway.as_ref(), input.pty_id, &input.data) {
            return Some(WsMessage::Error(ErrorMsg {
                code: "PTY_WRITE_ERROR".to_string(),
                message: e.to_string(),
            }));
        }
    }
    None
}

pub(crate) async fn handle_pty_spawn_request(
    req: &PtySpawnRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_path = {
        let wt = selected_worktree.lock().await;
        match wt.as_ref() {
            Some(p) => p.clone(),
            None => return Some(no_worktree_selected_error()),
        }
    };
    let (gateway, app) = match (state.pty_session_runtime_gateway(), state.app_handle()) {
        (Some(gateway), Some(app)) => (Arc::clone(gateway), app.clone()),
        _ => {
            return Some(WsMessage::PtySpawnResponse(PtySpawnResponse {
                success: false,
                pty_id: None,
                error: Some("PTY manager が利用できません".to_string()),
            }))
        }
    };

    let rows = req.rows;
    let cols = req.cols;
    let label = req.label.clone();
    let broadcaster = Arc::clone(state.broadcaster());
    let wt_path_for_ready = worktree_path.clone();
    let label_for_ready = label.clone();
    match tokio::task::spawn_blocking(move || {
        spawn_usecase::spawn(
            gateway.as_ref(),
            &app,
            rows,
            cols,
            Some(worktree_path.clone()),
            Some(worktree_path),
            label,
            PtyKind::Terminal,
        )
    })
    .await
    {
        Ok(Ok((pty_id, _session_key))) => {
            broadcaster.try_send(WsMessage::PtyReady(PtyReady {
                pty_id,
                cols,
                rows,
                label: label_for_ready,
                worktree_path: Some(wt_path_for_ready),
            }));
            let startup_cmd = state.get_terminal_startup_command();
            let trimmed_cmd = startup_cmd.trim();
            if !trimmed_cmd.is_empty() {
                if let Some(gateway) = state.pty_session_runtime_gateway() {
                    let data = format!("{}\n", trimmed_cmd);
                    if let Err(e) = io_usecase::write(gateway.as_ref(), pty_id, &data) {
                        log::warn!("Failed to write startup command to PTY {}: {}", pty_id, e);
                    }
                }
            }
            Some(WsMessage::PtySpawnResponse(PtySpawnResponse {
                success: true,
                pty_id: Some(pty_id),
                error: None,
            }))
        }
        Ok(Err(e)) => Some(WsMessage::PtySpawnResponse(PtySpawnResponse {
            success: false,
            pty_id: None,
            error: Some(e.to_string()),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

pub(crate) fn handle_pty_output_request(
    req: &PtyOutputRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    if let Some(gateway) = state.pty_session_runtime_gateway() {
        let sessions = query_service::list(gateway.as_ref());
        if sessions.iter().any(|s| s.pty_id == req.pty_id) {
            let buffered = state.broadcaster().get_pty_output_buffer(req.pty_id);
            if !buffered.is_empty() {
                state
                    .broadcaster()
                    .send_without_buffer(WsMessage::PtyOutput(PtyOutputMsg {
                        pty_id: req.pty_id,
                        data: buffered,
                    }));
            }
            None
        } else {
            Some(WsMessage::Error(ErrorMsg {
                code: "PTY_NOT_FOUND".to_string(),
                message: format!("PTY {} が見つかりません", req.pty_id),
            }))
        }
    } else {
        Some(WsMessage::Error(ErrorMsg {
            code: "NO_PTY".to_string(),
            message: "デスクトップのターミナルがまだ起動していません".to_string(),
        }))
    }
}

pub(crate) async fn handle_pty_kill_request(
    req: &PtyKillRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    let pty_id = req.pty_id;
    if let Some(gateway) = state.pty_session_runtime_gateway() {
        let gateway = Arc::clone(gateway);
        match tokio::task::spawn_blocking(move || lifecycle_usecase::kill(gateway.as_ref(), pty_id))
            .await
        {
            Ok(Ok(())) => {
                state.broadcaster().remove_pty_output_buffer(pty_id);
                Some(WsMessage::PtyKillResponse(PtyKillResponse {
                    success: true,
                    pty_id,
                    error: None,
                }))
            }
            Ok(Err(e)) => Some(WsMessage::PtyKillResponse(PtyKillResponse {
                success: false,
                pty_id,
                error: Some(e.to_string()),
            })),
            Err(e) => Some(join_error_msg(e)),
        }
    } else {
        Some(WsMessage::PtyKillResponse(PtyKillResponse {
            success: false,
            pty_id,
            error: Some("PTY manager が利用できません".to_string()),
        }))
    }
}
