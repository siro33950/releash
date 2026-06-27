use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::agent_message_dispatcher::{
    dispatch_agent_message, AgentMessageDispatchContext, AgentMessageDispatchRequest,
};
use crate::infrastructure::agent_session::runtime::{
    AgentBackendRegistry, AgentProcessMap, ImageAttachment, SendMessageResponse,
};
use crate::infrastructure::agent_session::runtime_gateway::AgentRuntimeGateway;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::session::{ChatSession, OpenTabRegistry, SessionStore};
use crate::usecase::workflow::step_lifecycle::ResolvedWorkflowStepSession;
use crate::usecase::workflow::{WorkflowRuntimeUsecase, WorkflowStepLifecycleUsecase};

pub(crate) type AgentProcessMapState = Arc<Mutex<AgentProcessMap>>;
pub(crate) type AgentBackendRegistryState = Arc<AgentBackendRegistry>;
pub(crate) type AgentImageAttachment = ImageAttachment;
pub(crate) type AgentSendMessageResponse = SendMessageResponse;
pub(crate) type OpenTabRegistryState = Arc<OpenTabRegistry>;
pub(crate) type SessionStoreState = Arc<SessionStore>;
pub(crate) type WorkflowStepLifecycleUsecaseState = Arc<WorkflowStepLifecycleUsecase>;

pub(crate) async fn build_workflow_state_view(
    state: crate::domain::workflow::WorkflowStateSnapshot,
    handles: &AgentProcessMapState,
    open_tabs: &OpenTabRegistryState,
) -> WorkflowStateView {
    crate::adaptor::gateway::workflow::build_workflow_state_view_from_snapshot(
        state, handles, open_tabs,
    )
    .await
}

pub(crate) async fn dispatch_agent_message_with_runtime(
    app: &tauri::AppHandle,
    branch_diff_context: &Arc<dyn BranchDiffContextPort>,
    session_store: &SessionStoreState,
    registry: &AgentBackendRegistryState,
    handles: &AgentProcessMapState,
    req: AgentMessageDispatchRequest,
) -> Result<AgentSendMessageResponse, String> {
    dispatch_agent_message(
        AgentMessageDispatchContext {
            gateway: AgentRuntimeGateway {
                app,
                branch_diff_context,
                session_store,
                registry,
                handles,
            },
        },
        req,
    )
    .await
}

pub(crate) async fn emit_after_workflow_step_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session: &ChatSession,
    handles: &AgentProcessMapState,
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
        handles,
        open_tabs,
    )
    .await;
}

pub(crate) async fn emit_workflow_step_target_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &ResolvedWorkflowStepSession,
    handles: &AgentProcessMapState,
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
        handles,
        open_tabs,
    )
    .await;
}
