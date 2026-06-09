use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::OpenTabRegistry;
use crate::workflow_state_presenter::WorkflowStateProjection;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowStateChangedPayload<'a> {
    worktree_path: &'a str,
    workflow_state: &'a crate::protocol::WorkflowStateView,
}

fn emit_workflow_state_view<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    view: &crate::protocol::WorkflowStateView,
) {
    let _ = app.emit(
        "workflow-state-changed",
        WorkflowStateChangedPayload {
            worktree_path,
            workflow_state: view,
        },
    );
    let broadcaster = app
        .try_state::<Arc<crate::ws_bridge::WsBroadcaster>>()
        .map(|state| state.inner().clone());
    if let Some(broadcaster) = &broadcaster {
        broadcaster.try_send(crate::protocol::WsMessage::WorkflowStateSync(Box::new(
            crate::protocol::WorkflowStateSync {
                worktree_path: worktree_path.to_string(),
                workflow_state: view.clone(),
            },
        )));
    }
}

async fn collect_runtime_session_sets(
    state: &crate::workflow::state::WorkflowState,
    handles: Option<&Arc<Mutex<AgentProcessMap>>>,
    open_tabs: Option<&OpenTabRegistry>,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let session_ids = crate::workflow::runtime_view::collect_step_session_ids(state);
    let active_sessions = if let Some(handles) = handles {
        let map = handles.lock().await;
        session_ids
            .iter()
            .filter(|session_id| map.contains_key(*session_id))
            .cloned()
            .collect()
    } else {
        std::collections::HashSet::new()
    };
    let open_sessions = if let Some(open_tabs) = open_tabs {
        session_ids
            .iter()
            .filter(|session_id| open_tabs.contains(session_id))
            .cloned()
            .collect()
    } else {
        std::collections::HashSet::new()
    };
    (active_sessions, open_sessions)
}

pub(crate) async fn build_workflow_state_projection(
    state: crate::workflow::state::WorkflowState,
    handles: Option<&Arc<Mutex<AgentProcessMap>>>,
    open_tabs: Option<&OpenTabRegistry>,
) -> WorkflowStateProjection {
    let (active_sessions, open_sessions) =
        collect_runtime_session_sets(&state, handles, open_tabs).await;
    crate::workflow_state_presenter::build_workflow_state_projection_from_sets(
        state,
        &active_sessions,
        &open_sessions,
    )
}

fn workflow_state_view_from_projection(
    projection: WorkflowStateProjection,
) -> crate::protocol::WorkflowStateView {
    let runtime_states = projection
        .runtime_states
        .into_iter()
        .map(|(session_id, state)| {
            (
                session_id,
                crate::protocol::WorkflowStepRuntimeState {
                    runtime_active: state.runtime_active,
                    tab_open: state.tab_open,
                },
            )
        })
        .collect();
    crate::protocol::WorkflowStateView::from_parts(
        crate::workflow_state_presenter::workflow_state_to_view(projection.state),
        runtime_states,
    )
}

pub(crate) async fn build_workflow_state_view(
    state: crate::workflow::state::WorkflowState,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) -> crate::protocol::WorkflowStateView {
    workflow_state_view_from_projection(
        build_workflow_state_projection(state, Some(handles), Some(open_tabs)).await,
    )
}

pub(crate) async fn emit_workflow_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    state: crate::workflow::state::WorkflowState,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) {
    let execution_id = state.execution_id.clone();
    let workflow_agent_state =
        crate::usecase::agent_session::status::AgentStatusCenter::workflow_execution_state_to_agent_state(
            &state.state,
        );
    let updated_at = state.updated_at;
    let view = build_workflow_state_view(state, handles, open_tabs).await;
    #[derive(Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct WorkflowStateChangedPayload<'a> {
        worktree_path: &'a str,
        workflow_state: &'a crate::protocol::WorkflowStateView,
    }
    let _ = app.emit(
        "workflow-state-changed",
        WorkflowStateChangedPayload {
            worktree_path,
            workflow_state: &view,
        },
    );
    let broadcaster = app
        .try_state::<Arc<crate::ws_bridge::WsBroadcaster>>()
        .map(|state| state.inner().clone());
    if let Some(broadcaster) = &broadcaster {
        broadcaster.try_send(crate::protocol::WsMessage::WorkflowStateSync(Box::new(
            crate::protocol::WorkflowStateSync {
                worktree_path: worktree_path.to_string(),
                workflow_state: view.clone(),
            },
        )));
    }
    if let Some(center) =
        app.try_state::<Arc<crate::usecase::agent_session::status::AgentStatusCenter>>()
    {
        let changes = center.update_workflow_snapshot(
            worktree_path,
            &execution_id,
            workflow_agent_state,
            updated_at,
        );
        crate::agent_status_events::emit_agent_status_changes(app, broadcaster.as_deref(), changes);
    }
}

pub(crate) async fn emit_workflow_step_target_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &crate::workflow::engine::WorkflowEngine,
    target: &crate::workflow_step_lifecycle::ResolvedWorkflowStepSession,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) {
    if let Some(state) = engine.get_state(&target.worktree_path).await {
        emit_workflow_state(app, &target.worktree_path, state, handles, open_tabs).await;
    }
}

pub(crate) async fn emit_after_workflow_step_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    engine: &crate::workflow::engine::WorkflowEngine,
    session: &crate::usecase::agent_session::session::ChatSession,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) {
    if !session.workflow_step_session {
        return;
    }
    if let Some(state) = engine.get_state(&session.worktree_path).await {
        emit_workflow_state(app, &session.worktree_path, state, handles, open_tabs).await;
    }
}

pub(crate) async fn emit_workflow_state_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    workflow_state: crate::workflow::state::WorkflowState,
) {
    let Some(center) =
        app.try_state::<Arc<crate::usecase::agent_session::status::AgentStatusCenter>>()
    else {
        return;
    };
    let broadcaster = app
        .try_state::<Arc<crate::ws_bridge::WsBroadcaster>>()
        .map(|state| state.inner().clone());
    let handles = app.try_state::<Arc<Mutex<AgentProcessMap>>>();
    let open_tabs = app.try_state::<Arc<OpenTabRegistry>>();
    let projection = build_workflow_state_projection(
        workflow_state.clone(),
        handles.as_deref(),
        open_tabs.as_deref().map(Arc::as_ref),
    )
    .await;
    let view = workflow_state_view_from_projection(projection);
    emit_workflow_state_view(app, worktree_path, &view);

    for status in center.list_sessions() {
        if status.worktree_path == worktree_path {
            let mut updated = status;
            updated.workflow_step = Some(workflow_state.current_step_name.clone());
            updated.workflow_execution_state = Some(workflow_state.state.as_str().to_string());
            let changes = center.update_session(updated);
            crate::agent_status_events::emit_agent_status_changes(
                app,
                broadcaster.as_deref(),
                changes,
            );
        }
    }

    let workflow_agent_state =
        crate::usecase::agent_session::status::AgentStatusCenter::workflow_execution_state_to_agent_state(
            &workflow_state.state,
        );
    let changes = center.update_workflow_snapshot(
        worktree_path,
        &workflow_state.execution_id,
        workflow_agent_state,
        workflow_state.updated_at,
    );
    crate::agent_status_events::emit_agent_status_changes(app, broadcaster.as_deref(), changes);
}
