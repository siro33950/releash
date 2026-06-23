use crate::protocol::{AgentStateSync, WsMessage};
use crate::usecase::agent_session::status::{AgentState, AgentStatusChanges};
use crate::ws_bridge::WsBroadcaster;

fn protocol_agent_state(state: AgentState) -> crate::protocol::AgentState {
    match state {
        AgentState::Running => crate::protocol::AgentState::Running,
        AgentState::Done => crate::protocol::AgentState::Done,
        AgentState::Error => crate::protocol::AgentState::Error,
        AgentState::Waiting => crate::protocol::AgentState::Waiting,
    }
}

pub(crate) fn emit_agent_status_changes<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    broadcaster: Option<&WsBroadcaster>,
    changes: AgentStatusChanges,
) {
    if changes.is_empty() {
        return;
    }

    use tauri::Emitter;

    if let Some(session) = changes.session {
        let _ = app.emit("session-status-changed", session);
    }
    if let Some(workspace) = changes.workspace {
        let _ = app.emit("workspace-status-changed", workspace);
    }
    for workflow_step in changes.workflow_steps {
        let _ = app.emit("workflow-step-status-changed", workflow_step);
    }
    if let Some(agent_state) = changes.agent_state {
        let payload = AgentStateSync {
            worktree_path: agent_state.worktree_path,
            state: protocol_agent_state(agent_state.state),
            exit_code: None,
            timestamp: agent_state.timestamp,
            session_id: agent_state.session_id,
            pty_id: agent_state.pty_id,
        };
        let _ = app.emit("agent-state-changed", &payload);
        if let Some(broadcaster) = broadcaster {
            broadcaster.try_send(WsMessage::AgentStateSync(payload));
        }
    }
}
