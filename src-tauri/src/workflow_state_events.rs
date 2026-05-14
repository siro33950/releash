use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use crate::agent_sdk::AgentProcessMap;
use crate::session::OpenTabRegistry;
use crate::workflow_state_presenter::WorkflowStateProjection;

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
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) -> WorkflowStateProjection {
    let (active_sessions, open_sessions) =
        collect_runtime_session_sets(&state, Some(handles), Some(open_tabs)).await;
    crate::workflow_state_presenter::build_workflow_state_projection_from_sets(
        state,
        &active_sessions,
        &open_sessions,
    )
}

async fn build_workflow_state_projection_with_fallbacks(
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
        build_workflow_state_projection(state, handles, open_tabs).await,
    )
}

pub(crate) async fn emit_workflow_state(
    app: &tauri::AppHandle,
    worktree_path: &str,
    state: crate::workflow::state::WorkflowState,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) {
    let view = build_workflow_state_view(state, handles, open_tabs).await;
    if let Some(center) = app.try_state::<Arc<crate::agent_status::AgentStatusCenter>>() {
        center.emit_workflow_state_changed(worktree_path, &view);
    }
}

pub(crate) async fn emit_workflow_step_target_state(
    app: &tauri::AppHandle,
    engine: &crate::workflow::engine::WorkflowEngine,
    target: &crate::workflow_step_lifecycle::ResolvedWorkflowStepSession,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) {
    if let Some(state) = engine.get_state(&target.worktree_path).await {
        emit_workflow_state(app, &target.worktree_path, state, handles, open_tabs).await;
    }
}

pub(crate) async fn emit_after_workflow_step_message(
    app: &tauri::AppHandle,
    engine: &crate::workflow::engine::WorkflowEngine,
    session: &crate::session::ChatSession,
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

pub(crate) async fn emit_workflow_state_snapshot(
    app: &tauri::AppHandle,
    worktree_path: &str,
    workflow_state: crate::workflow::state::WorkflowState,
) {
    let Some(center) = app.try_state::<Arc<crate::agent_status::AgentStatusCenter>>() else {
        return;
    };
    let handles = app.try_state::<Arc<Mutex<AgentProcessMap>>>();
    let open_tabs = app.try_state::<Arc<OpenTabRegistry>>();
    let projection = build_workflow_state_projection_with_fallbacks(
        workflow_state.clone(),
        handles.as_deref(),
        open_tabs.as_deref().map(Arc::as_ref),
    )
    .await;
    let view = workflow_state_view_from_projection(projection);
    center.emit_workflow_state_changed(worktree_path, &view);

    for status in center.list_sessions() {
        if status.worktree_path == worktree_path {
            let mut updated = status;
            updated.workflow_step = Some(workflow_state.current_step_name.clone());
            updated.workflow_execution_state = Some(workflow_state.state.as_str().to_string());
            center.update_session(updated);
        }
    }
}
