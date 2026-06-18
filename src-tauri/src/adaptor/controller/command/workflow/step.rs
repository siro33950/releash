use crate::adaptor::controller::command::workflow::session_errors::redacted_workflow_tab_error;
use crate::adaptor::controller_support::{
    open_workflow_step_tab as open_step_tab, AgentProcessMapState, OpenTabRegistryState,
    WorkflowStepLifecycleUsecaseState,
};

#[tauri::command]
pub async fn open_workflow_step_tab(
    app: tauri::AppHandle,
    handles: tauri::State<'_, AgentProcessMapState>,
    open_tabs: tauri::State<'_, OpenTabRegistryState>,
    step_lifecycle: tauri::State<'_, WorkflowStepLifecycleUsecaseState>,
    chat_session_id: String,
) -> Result<(), String> {
    let target = open_step_tab(step_lifecycle.inner(), &chat_session_id)
        .await
        .map_err(|_| redacted_workflow_tab_error("workflow_step_session_rejected"))?;
    crate::adaptor::controller_support::emit_workflow_step_target_state(
        &app,
        &target,
        handles.inner(),
        open_tabs.inner(),
    )
    .await;
    Ok(())
}
