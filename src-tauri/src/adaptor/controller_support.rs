use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::usecase::agent_session::runtime::{AgentSessionRuntimeUsecase, SendMessageResponse};
use crate::usecase::agent_session::session::{ChatSession, ImageAttachment, OpenTabRegistry};
use crate::usecase::workflow::step_lifecycle::ResolvedWorkflowStepSession;
use crate::usecase::workflow::{WorkflowRuntimeUsecase, WorkflowStepLifecycleUsecase};

pub(crate) type AgentSessionRuntimeState = Arc<AgentSessionRuntimeUsecase>;
pub(crate) type AgentImageAttachment = ImageAttachment;
pub(crate) type AgentSendMessageResponse = SendMessageResponse;
pub(crate) type OpenTabRegistryState = Arc<OpenTabRegistry>;
pub(crate) type WorkflowStepLifecycleUsecaseState = Arc<WorkflowStepLifecycleUsecase>;

pub(crate) async fn build_workflow_state_view(
    state: crate::domain::workflow::WorkflowStateSnapshot,
    agent_runtime: &AgentSessionRuntimeState,
    open_tabs: &OpenTabRegistryState,
) -> WorkflowStateView {
    crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
        state,
        agent_runtime.as_ref(),
        open_tabs,
    )
    .await
}

pub(crate) async fn emit_after_workflow_step_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session: &ChatSession,
    agent_runtime: &AgentSessionRuntimeState,
    open_tabs: &OpenTabRegistryState,
) {
    let Some(runtime) = app
        .try_state::<Arc<WorkflowRuntimeUsecase>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let Ok(Some(state)) = runtime.get_state_by_worktree(&session.worktree_path).await else {
        return;
    };
    crate::adaptor::gateway::workflow::emit_workflow_state_from_snapshot(
        app,
        &session.worktree_path,
        state,
        agent_runtime.as_ref(),
        open_tabs,
    )
    .await;
}

pub(crate) async fn emit_workflow_step_target_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &ResolvedWorkflowStepSession,
    agent_runtime: &AgentSessionRuntimeState,
    open_tabs: &OpenTabRegistryState,
) {
    let Some(runtime) = app
        .try_state::<Arc<WorkflowRuntimeUsecase>>()
        .map(|state| state.inner().clone())
    else {
        return;
    };
    let Ok(Some(state)) = runtime.get_state_by_worktree(&target.worktree_path).await else {
        return;
    };
    crate::adaptor::gateway::workflow::emit_workflow_state_from_snapshot(
        app,
        &target.worktree_path,
        state,
        agent_runtime.as_ref(),
        open_tabs,
    )
    .await;
}
