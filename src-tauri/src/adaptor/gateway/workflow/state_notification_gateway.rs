use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::adaptor::presenter::workflow::WorkflowStateProjection;
use crate::adaptor::protocol::workflow::{WorkflowStateView, WorkflowStepRuntimeState};
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::OpenTabRegistry;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowStateChangedPayload<'a> {
    worktree_path: &'a str,
    workflow_state: &'a WorkflowStateView,
}

fn optional_arc_state<R, T>(app: &tauri::AppHandle<R>) -> Option<Arc<T>>
where
    R: tauri::Runtime,
    T: Send + Sync + 'static,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.state::<Arc<T>>().inner().clone()
    }))
    .ok()
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
    runtime: Option<&AgentSessionRuntimeUsecase>,
    open_tabs: Option<&OpenTabRegistry>,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    let session_ids =
        crate::domain::workflow::services::session_projection::collect_step_session_ids(state);
    let active_sessions = if let Some(runtime) = runtime {
        let candidates = session_ids.iter().cloned().collect::<Vec<_>>();
        runtime.active_session_ids(&candidates).await
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
    runtime: &AgentSessionRuntimeUsecase,
    open_tabs: &OpenTabRegistry,
) -> WorkflowStateView {
    workflow_state_view_from_projection(
        build_workflow_state_projection_from_snapshot(state, Some(runtime), Some(open_tabs)).await,
    )
}

pub(crate) async fn build_workflow_state_projection_from_snapshot(
    state: crate::domain::workflow::WorkflowStateSnapshot,
    runtime: Option<&AgentSessionRuntimeUsecase>,
    open_tabs: Option<&OpenTabRegistry>,
) -> WorkflowStateProjection {
    let (active_sessions, open_sessions) =
        collect_runtime_session_sets(&state, runtime, open_tabs).await;
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
    runtime: &AgentSessionRuntimeUsecase,
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
    let view = build_workflow_state_view_from_snapshot(state, runtime, open_tabs).await;
    emit_workflow_state_view(app, worktree_path, &view);
    if let Some(center) =
        optional_arc_state::<_, crate::usecase::agent_session::status::AgentStatusCenter>(app)
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
    let center =
        optional_arc_state::<_, crate::usecase::agent_session::status::AgentStatusCenter>(app);
    let runtime = optional_arc_state::<_, AgentSessionRuntimeUsecase>(app);
    let open_tabs = optional_arc_state::<_, OpenTabRegistry>(app);
    let (active_sessions, open_sessions) =
        collect_runtime_session_sets(&workflow_state, runtime.as_deref(), open_tabs.as_deref())
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
    if let Some(center) = center {
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
}
