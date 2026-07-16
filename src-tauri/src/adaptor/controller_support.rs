use std::sync::Arc;

use tauri::Manager;

use crate::usecase::agent_session::runtime::{AgentSessionRuntimeUsecase, SendMessageResponse};
use crate::usecase::agent_session::session::{ChatSession, ImageAttachment};
use crate::usecase::workflow::node_lifecycle::ResolvedWorkflowNodeSession;
use crate::usecase::workflow::{NodeExecutionLifecycleUsecase, WorkflowRuntimeUsecase};

pub(crate) type AgentSessionRuntimeState = Arc<AgentSessionRuntimeUsecase>;
pub(crate) type AgentImageAttachment = ImageAttachment;
pub(crate) type AgentSendMessageResponse = SendMessageResponse;
pub(crate) type NodeExecutionLifecycleUsecaseState = Arc<NodeExecutionLifecycleUsecase>;

pub(crate) async fn emit_after_workflow_node_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session: &ChatSession,
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
    crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
        app,
        &session.worktree_path,
        state,
    )
    .await;
}

pub(crate) async fn emit_workflow_node_target_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &ResolvedWorkflowNodeSession,
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
    crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
        app,
        &target.worktree_path,
        state,
    )
    .await;
}
