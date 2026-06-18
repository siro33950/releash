//! Workflow WebSocket handler helpers.

use crate::adaptor::controller_support::{AgentProcessMapState, OpenTabRegistryState};
use crate::usecase::agent_session::session::ChatSession;
use crate::usecase::workflow::step_lifecycle::ResolvedWorkflowStepSession;

pub(crate) async fn emit_after_workflow_step_message<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session: &ChatSession,
    handles: &AgentProcessMapState,
    open_tabs: &OpenTabRegistryState,
) {
    crate::adaptor::controller_support::emit_after_workflow_step_message(
        app, session, handles, open_tabs,
    )
    .await
}

pub(crate) async fn emit_workflow_step_target_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    target: &ResolvedWorkflowStepSession,
    handles: &AgentProcessMapState,
    open_tabs: &OpenTabRegistryState,
) {
    crate::adaptor::controller_support::emit_workflow_step_target_state(
        app, target, handles, open_tabs,
    )
    .await
}
