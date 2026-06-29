use std::sync::Mutex;

use crate::usecase::agent_session::status::{
    AgentStatusCenter, AgentStatusChanges, AgentStatusNotifier, WorktreeStepStatusView,
};
use tauri::Emitter;

static WORKFLOW_STEP_STATUS_EMIT_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct TauriAgentStatusNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriAgentStatusNotifier<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> AgentStatusNotifier for TauriAgentStatusNotifier<R> {
    fn status_changed(&self, changes: AgentStatusChanges) {
        emit_agent_status_changes(&self.app, changes);
    }
}

#[derive(Clone, serde::Serialize)]
struct AgentStateChangedPayload {
    worktree_path: String,
    state: crate::usecase::agent_session::status::AgentState,
    exit_code: Option<i32>,
    timestamp: f64,
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pty_id: Option<String>,
}

pub(crate) fn emit_agent_status_changes<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    changes: AgentStatusChanges,
) {
    if changes.is_empty() {
        return;
    }

    if let Some(session) = changes.session {
        let _ = app.emit("session-status-changed", session);
    }
    if let Some(workspace) = changes.workspace {
        let _ = app.emit("workspace-status-changed", workspace);
    }
    emit_worktree_step_status_views(app, changes.workflow_step_views);
    if let Some(agent_state) = changes.agent_state {
        let payload = AgentStateChangedPayload {
            worktree_path: agent_state.worktree_path,
            state: agent_state.state,
            exit_code: None,
            timestamp: agent_state.timestamp,
            session_id: agent_state.session_id,
            pty_id: agent_state.pty_id,
        };
        let _ = app.emit("agent-state-changed", &payload);
    }
}

pub(crate) fn emit_worktree_step_status_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    center: &AgentStatusCenter,
    worktree_path: &str,
) {
    emit_worktree_step_status_snapshot_with(center, worktree_path, |workflow_step_view| {
        emit_worktree_step_status_view(app, workflow_step_view);
    });
}

fn emit_worktree_step_status_views<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    workflow_step_views: Vec<WorktreeStepStatusView>,
) {
    emit_worktree_step_status_views_with(workflow_step_views, |workflow_step_view| {
        emit_worktree_step_status_view(app, workflow_step_view);
    });
}

fn emit_worktree_step_status_snapshot_with(
    center: &AgentStatusCenter,
    worktree_path: &str,
    emit: impl FnOnce(WorktreeStepStatusView),
) {
    with_workflow_step_status_emit_order(|| {
        let workflow_step_view = center.query_worktree_step_statuses(worktree_path);
        emit(workflow_step_view);
    });
}

fn emit_worktree_step_status_views_with(
    workflow_step_views: Vec<WorktreeStepStatusView>,
    mut emit: impl FnMut(WorktreeStepStatusView),
) {
    if workflow_step_views.is_empty() {
        return;
    }

    with_workflow_step_status_emit_order(|| {
        for workflow_step_view in workflow_step_views {
            emit(workflow_step_view);
        }
    });
}

fn with_workflow_step_status_emit_order<T>(f: impl FnOnce() -> T) -> T {
    let _guard = WORKFLOW_STEP_STATUS_EMIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f()
}

fn emit_worktree_step_status_view<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    workflow_step_view: WorktreeStepStatusView,
) {
    let _ = app.emit("workflow-step-status-changed", workflow_step_view);
}

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;

    use crate::domain::workflow::status_aggregation::StepProgress;
    use crate::usecase::agent_session::session::SessionState;
    use crate::usecase::agent_session::status::{
        AgentState, AgentStatusCenter, SessionStatus, TurnPhaseRepr,
    };

    use super::{emit_worktree_step_status_snapshot_with, emit_worktree_step_status_views_with};

    fn workflow_session(
        session_id: &str,
        progress: StepProgress,
        agent_state: AgentState,
    ) -> SessionStatus {
        SessionStatus {
            chat_session_id: session_id.to_string(),
            worktree_id: "/repo".to_string(),
            worktree_path: "/repo".to_string(),
            pty_id: None,
            agent_state,
            turn_phase: TurnPhaseRepr::Idle,
            session_state: SessionState::Done,
            pending_permission: false,
            pending_permission_request: None,
            last_activity_at: 0.0,
            workflow_step: Some("build".to_string()),
            workflow_execution_state: Some("running".to_string()),
            workflow_execution_id: Some("exec-1".to_string()),
            workflow_run_index: Some(1),
            workflow_step_progress: Some(progress),
        }
    }

    #[test]
    fn workflow_step_sync_snapshot_is_serialized_with_live_emits() {
        let center = Arc::new(AgentStatusCenter::new());
        let initial = center.update_session(workflow_session(
            "step-a",
            StepProgress::Queued,
            AgentState::Done,
        ));
        let initial_version = initial.workflow_step_views[0].version;

        let emitted_versions = Arc::new(Mutex::new(Vec::new()));
        let (snapshot_read_tx, snapshot_read_rx) = mpsc::channel();
        let (allow_sync_emit_tx, allow_sync_emit_rx) = mpsc::channel();

        let sync_center = center.clone();
        let sync_emitted_versions = emitted_versions.clone();
        let sync_handle = thread::spawn(move || {
            emit_worktree_step_status_snapshot_with(&sync_center, "/repo", |view| {
                snapshot_read_tx.send(view.version).unwrap();
                allow_sync_emit_rx.recv().unwrap();
                sync_emitted_versions.lock().unwrap().push(view.version);
            });
        });

        let snapshot_version = snapshot_read_rx.recv().unwrap();
        assert_eq!(snapshot_version, initial_version);

        let live = center.update_session(workflow_session(
            "step-a",
            StepProgress::Queued,
            AgentState::Running,
        ));
        let live_view = live
            .workflow_step_views
            .into_iter()
            .next()
            .expect("live update emits step status view");
        let live_version = live_view.version;
        assert!(live_version > snapshot_version);

        let (live_started_tx, live_started_rx) = mpsc::channel();
        let live_emitted_versions = emitted_versions.clone();
        let live_handle = thread::spawn(move || {
            live_started_tx.send(()).unwrap();
            emit_worktree_step_status_views_with(vec![live_view], |view| {
                live_emitted_versions.lock().unwrap().push(view.version);
            });
        });
        live_started_rx.recv().unwrap();

        allow_sync_emit_tx.send(()).unwrap();
        sync_handle.join().unwrap();
        live_handle.join().unwrap();

        let emitted_versions = emitted_versions.lock().unwrap().clone();
        assert_eq!(emitted_versions, vec![snapshot_version, live_version]);
    }
}
