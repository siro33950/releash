use std::sync::Arc;

use crate::adaptor::gateway::shared::ws_broadcaster::WsBroadcaster;
use crate::adaptor::protocol::{AgentStateSync, WsMessage};
use crate::usecase::agent_session::status::{AgentStatusChanges, AgentStatusNotifier};

pub(crate) struct TauriAgentStatusNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    broadcaster: Arc<WsBroadcaster>,
}

impl<R: tauri::Runtime> TauriAgentStatusNotifier<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>, broadcaster: Arc<WsBroadcaster>) -> Self {
        Self { app, broadcaster }
    }
}

impl<R: tauri::Runtime> AgentStatusNotifier for TauriAgentStatusNotifier<R> {
    fn status_changed(&self, changes: AgentStatusChanges) {
        emit_agent_status_changes(&self.app, Some(&self.broadcaster), changes);
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
            state: agent_state.state.into(),
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
