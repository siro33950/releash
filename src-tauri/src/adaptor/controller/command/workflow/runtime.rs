use std::sync::Arc;

use serde::Deserialize;

use crate::adaptor::controller::command::workflow::{
    parse_workflow_approval_permission_mode, parse_workflow_start_permission_mode, validate_run_id,
};
use crate::adaptor::controller_support::{
    build_workflow_state_view, AgentImageAttachment, AgentSendMessageResponse,
    AgentSessionRuntimeState, OpenTabRegistryState,
};
use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::usecase::agent_session::runtime::SendAgentMessageRequest;
use crate::usecase::workflow::command::{AbortRunCommand, ApprovalCommand, StartRunCommand};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

fn parse_domain_trigger_source(
    value: Option<String>,
) -> Result<crate::domain::workflow::TriggerSource, String> {
    match value.as_deref() {
        Some("cli") => Ok(crate::domain::workflow::TriggerSource::Cli),
        Some("remote") => Ok(crate::domain::workflow::TriggerSource::Remote),
        Some("agent") => Ok(crate::domain::workflow::TriggerSource::Agent),
        Some("desktop_ui") | Some("desktop-ui") | None => {
            Ok(crate::domain::workflow::TriggerSource::DesktopUi)
        }
        Some(other) => Err(format!("unknown trigger_source: {other}")),
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApproveWorkflowStepArgs {
    pub run_id: String,
    pub step_name: String,
    #[serde(default)]
    pub node_execution_id: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[cfg(test)]
pub(crate) fn parse_approve_workflow_step_args(
    value: &serde_json::Value,
) -> Result<ApproveWorkflowStepArgs, serde_json::Error> {
    serde_json::from_value::<ApproveWorkflowStepArgs>(value.clone())
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn start_workflow(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    workflow_name: String,
    worktree_path: String,
    task: Option<String>,
    trigger_source: Option<String>,
    permission_mode: Option<String>,
) -> Result<String, String> {
    let trigger_source = parse_domain_trigger_source(trigger_source)?;
    let permission_mode = parse_workflow_start_permission_mode(permission_mode)?;
    runtime
        .start_run(StartRunCommand {
            workflow_file_stem: workflow_name,
            worktree_path,
            task,
            trigger_source,
            permission_mode: permission_mode.to_string(),
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn abort_workflow(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    run_id: String,
) -> Result<(), String> {
    validate_run_id(&run_id)?;
    runtime
        .abort_run(AbortRunCommand {
            run_id,
            expected_node_name: None,
        })
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("abort_workflow failed: code=ABORT_WORKFLOW_FAILED");
            msg
        })
}

#[tauri::command]
pub async fn get_workflow_state(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    agent_runtime: tauri::State<'_, AgentSessionRuntimeState>,
    open_tabs: tauri::State<'_, OpenTabRegistryState>,
    run_id: String,
) -> Result<Option<WorkflowStateView>, String> {
    validate_run_id(&run_id)?;
    match runtime
        .get_state_by_run_id(&run_id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(state) => Ok(Some(
            build_workflow_state_view(state, agent_runtime.inner(), open_tabs.inner()).await,
        )),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn approve_workflow_step(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    args: ApproveWorkflowStepArgs,
) -> Result<(), String> {
    let ApproveWorkflowStepArgs {
        run_id,
        step_name,
        node_execution_id,
        comment,
    } = args;
    validate_run_id(&run_id)?;
    runtime
        .resolve_approval(ApprovalCommand {
            run_id,
            node_name: step_name,
            node_execution_id,
            comment,
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_workflow_approval_chat_message(
    app: tauri::AppHandle,
    agent_runtime: tauri::State<'_, AgentSessionRuntimeState>,
    open_tabs: tauri::State<'_, OpenTabRegistryState>,
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    run_id: String,
    content: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
    images: Option<Vec<AgentImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
) -> Result<AgentSendMessageResponse, String> {
    // Spec issues-1011 line 121: 起動以外の workflow 操作 API は run_id を主語に取る。
    // chat_session_id / worktree_path は run_id から workflow runtime usecase が解決する。
    validate_run_id(&run_id)?;
    let permission_mode = parse_workflow_approval_permission_mode(permission_mode)?;
    let mentions = mentions.map(crate::adaptor::protocol::mention::into_domain_vec);

    let approval_target = runtime
        .prepare_approval_chat(&run_id, &content)
        .await
        .map_err(|e| e.to_string())?;

    let response = agent_runtime
        .send_message(SendAgentMessageRequest {
            chat_session_id: Some(approval_target.chat_session_id),
            worktree_path: approval_target.worktree_path,
            content,
            permission_mode,
            plan_mode: plan_mode.unwrap_or(false),
            backend_id: None,
            model_id: None,
            images,
            mentions,
            editor_context: None,
        })
        .await
        .map_err(|e| e.to_string())?;
    crate::adaptor::controller_support::emit_after_workflow_step_message(
        &app,
        &response.session,
        agent_runtime.inner(),
        open_tabs.inner(),
    )
    .await;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approve_args_accept_optional_node_execution_address() {
        let args = parse_approve_workflow_step_args(&serde_json::json!({
            "runId": "00000000-0000-0000-0000-000000000001",
            "stepName": "review",
            "nodeExecutionId": "node-execution-review",
        }))
        .unwrap();

        assert_eq!(
            args.node_execution_id.as_deref(),
            Some("node-execution-review")
        );
    }

    #[test]
    fn approve_args_keep_single_name_fallback_when_address_is_omitted() {
        let args = parse_approve_workflow_step_args(&serde_json::json!({
            "runId": "00000000-0000-0000-0000-000000000001",
            "stepName": "review",
        }))
        .unwrap();

        assert!(args.node_execution_id.is_none());
    }
}
