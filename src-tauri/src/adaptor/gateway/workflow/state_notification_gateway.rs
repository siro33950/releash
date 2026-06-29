use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::adaptor::presenter::workflow::WorkflowStateProjection;
use crate::adaptor::protocol::workflow::{WorkflowStateView, WorkflowStepRuntimeState};
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::OpenTabRegistry;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowStateChangedPayload<'a> {
    worktree_path: &'a str,
    workflow_state: &'a WorkflowStateView,
}

fn emit_workflow_state_view<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    view: &WorkflowStateView,
) {
    let _ = app.emit(
        "workflow-state-changed",
        WorkflowStateChangedPayload {
            worktree_path,
            workflow_state: view,
        },
    );
}

async fn collect_runtime_session_sets(
    state: &crate::domain::workflow::WorkflowStateSnapshot,
    handles: Option<&Arc<Mutex<AgentProcessMap>>>,
    open_tabs: Option<&OpenTabRegistry>,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let session_ids =
        crate::domain::workflow::services::session_projection::collect_step_session_ids(state);
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

fn workflow_state_view_from_projection(projection: WorkflowStateProjection) -> WorkflowStateView {
    let runtime_states = projection
        .runtime_states
        .into_iter()
        .map(|(session_id, state)| {
            (
                session_id,
                WorkflowStepRuntimeState {
                    runtime_active: state.runtime_active,
                    tab_open: state.tab_open,
                },
            )
        })
        .collect();
    WorkflowStateView::from_parts(
        crate::adaptor::presenter::workflow::workflow_state_to_view(projection.state),
        runtime_states,
    )
}

pub(crate) async fn build_workflow_state_view_from_snapshot(
    state: crate::domain::workflow::WorkflowStateSnapshot,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) -> WorkflowStateView {
    workflow_state_view_from_projection(
        build_workflow_state_projection_from_snapshot(state, Some(handles), Some(open_tabs)).await,
    )
}

pub(crate) async fn build_workflow_state_projection_from_snapshot(
    state: crate::domain::workflow::WorkflowStateSnapshot,
    handles: Option<&Arc<Mutex<AgentProcessMap>>>,
    open_tabs: Option<&OpenTabRegistry>,
) -> WorkflowStateProjection {
    let (active_sessions, open_sessions) =
        collect_runtime_session_sets(&state, handles, open_tabs).await;
    crate::adaptor::presenter::workflow::build_workflow_state_projection_from_sets(
        state,
        &active_sessions,
        &open_sessions,
    )
}

pub(crate) async fn emit_workflow_state_from_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    state: crate::domain::workflow::WorkflowStateSnapshot,
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
) {
    let execution_id = state.execution_id.clone();
    let workflow_execution_state = state.state.as_str().to_string();
    let step_session_projections =
        crate::domain::workflow::services::session_projection::collect_step_session_projections(
            &state,
        );
    let workflow_agent_state =
        crate::usecase::agent_session::status::AgentStatusCenter::workflow_execution_state_to_agent_state(
            &state.state,
        );
    let updated_at = state.updated_at;
    let view = build_workflow_state_view_from_snapshot(state, handles, open_tabs).await;
    emit_workflow_state_view(app, worktree_path, &view);
    if let Some(center) =
        app.try_state::<Arc<crate::usecase::agent_session::status::AgentStatusCenter>>()
    {
        for changes in center.sync_workflow_step_session_statuses(
            worktree_path,
            &execution_id,
            &workflow_execution_state,
            step_session_projections,
        ) {
            crate::adaptor::presenter::agent_status::emit_agent_status_changes(app, changes);
        }
        let changes = center.update_workflow_snapshot(
            worktree_path,
            &execution_id,
            workflow_agent_state,
            updated_at,
        );
        crate::adaptor::presenter::agent_status::emit_agent_status_changes(app, changes);
    }
}

pub(crate) async fn emit_workflow_state_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    workflow_state: crate::domain::workflow::WorkflowStateSnapshot,
) {
    let Some(center) =
        app.try_state::<Arc<crate::usecase::agent_session::status::AgentStatusCenter>>()
    else {
        return;
    };
    let handles = app.try_state::<Arc<Mutex<AgentProcessMap>>>();
    let open_tabs = app.try_state::<Arc<OpenTabRegistry>>();
    let (active_sessions, open_sessions) = collect_runtime_session_sets(
        &workflow_state,
        handles.as_deref(),
        open_tabs.as_deref().map(Arc::as_ref),
    )
    .await;
    let projection = crate::adaptor::presenter::workflow::build_workflow_state_projection_from_sets(
        workflow_state.clone(),
        &active_sessions,
        &open_sessions,
    );
    let view = workflow_state_view_from_projection(projection);
    emit_workflow_state_view(app, worktree_path, &view);
    let workflow_execution_state = workflow_state.state.as_str().to_string();
    let step_session_projections =
        crate::domain::workflow::services::session_projection::collect_step_session_projections(
            &workflow_state,
        );
    for changes in center.sync_workflow_step_session_statuses(
        worktree_path,
        &workflow_state.execution_id,
        &workflow_execution_state,
        step_session_projections,
    ) {
        crate::adaptor::presenter::agent_status::emit_agent_status_changes(app, changes);
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
    crate::adaptor::presenter::agent_status::emit_agent_status_changes(app, changes);
}
