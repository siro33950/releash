use std::sync::Arc;

use serde::Deserialize;

use crate::adaptor::controller::agent_session_operation_wiring::{
    CanonicalSendCommandV1, CanonicalSendTargetV1,
};
use crate::adaptor::controller::command::agent_session::session::dispatch_durable_send;
use crate::adaptor::controller::command::workflow::{
    parse_workflow_approval_permission_mode, parse_workflow_start_permission_mode,
    validate_execution_id,
};
use crate::adaptor::controller_support::{AgentImageAttachment, AgentSessionRuntimeState};
use crate::adaptor::protocol::agent_session_v1::{SendCommandErrorDtoV1, SendCommandOutcomeDtoV1};
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, ResumeExecutionCommand, StartExecutionCommand,
    StopExecutionCommand,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

fn parse_execution_origin(
    value: Option<String>,
) -> Result<crate::domain::workflow::ExecutionOrigin, String> {
    value
        .as_deref()
        .map(crate::domain::workflow::ExecutionOrigin::from_public_value)
        .unwrap_or(Ok(crate::domain::workflow::ExecutionOrigin::DesktopUi))
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApproveWorkflowNodeArgs {
    pub execution_id: String,
    pub node_name: String,
    #[serde(default)]
    pub node_execution_id: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
}

#[cfg(test)]
pub(crate) fn parse_approve_workflow_node_args(
    value: &serde_json::Value,
) -> Result<ApproveWorkflowNodeArgs, serde_json::Error> {
    serde_json::from_value::<ApproveWorkflowNodeArgs>(value.clone())
}

#[tauri::command]
pub async fn start_workflow(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    workflow_name: String,
    worktree_path: String,
    request: Option<String>,
    created_from: Option<String>,
    permission_mode: Option<String>,
) -> Result<String, String> {
    let created_from = parse_execution_origin(created_from)?;
    let permission_mode = parse_workflow_start_permission_mode(permission_mode)?;
    runtime
        .start_execution(StartExecutionCommand {
            workflow_name,
            worktree_path,
            request,
            created_from,
            permission_mode: permission_mode.to_string(),
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn abort_workflow(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    execution_id: String,
) -> Result<(), String> {
    validate_execution_id(&execution_id)?;
    runtime
        .abort_execution(AbortExecutionCommand {
            execution_id,
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
pub async fn stop_workflow(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    execution_id: String,
) -> Result<(), String> {
    validate_execution_id(&execution_id)?;
    runtime
        .stop_execution(StopExecutionCommand { execution_id })
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("stop_workflow failed: code=STOP_WORKFLOW_FAILED");
            msg
        })
}

#[tauri::command]
pub async fn resume_workflow(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    execution_id: String,
) -> Result<(), String> {
    validate_execution_id(&execution_id)?;
    runtime
        .resume_execution(ResumeExecutionCommand { execution_id })
        .await
        .map_err(|e| {
            let msg = e.to_string();
            log::error!("resume_workflow failed: code=RESUME_WORKFLOW_FAILED");
            msg
        })
}

#[tauri::command]
pub async fn approve_workflow_node(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    args: ApproveWorkflowNodeArgs,
) -> Result<(), String> {
    approve_workflow_node_with_runtime(runtime.inner().as_ref(), args).await
}

/// Tauri wrapper と transport-independent boundary test が共有する production adapter。
/// state transition は持たず、typed command を `WorkflowRuntimeUsecase` へ渡すだけに保つ。
pub(crate) async fn approve_workflow_node_with_runtime(
    runtime: &WorkflowRuntimeUsecase,
    args: ApproveWorkflowNodeArgs,
) -> Result<(), String> {
    let ApproveWorkflowNodeArgs {
        execution_id,
        node_name,
        node_execution_id,
        comment,
    } = args;
    validate_execution_id(&execution_id)?;
    runtime
        .resolve_approval(ApprovalCommand {
            execution_id,
            node_name,
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
    store: tauri::State<'_, Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    send_operation: tauri::State<
        '_,
        Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    >,
    journal: tauri::State<'_, Arc<crate::usecase::agent_session::operation::CallerAttemptJournal>>,
    operation_id: String,
    execution_id: String,
    content: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
    images: Option<Vec<AgentImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
) -> Result<SendCommandOutcomeDtoV1, SendCommandErrorDtoV1> {
    // Spec issues-1011 line 121: 起動以外の workflow 操作 API は execution_id を主語に取る。
    // chat_session_id / worktree_path は execution_id から workflow runtime usecase が解決する。
    validate_execution_id(&execution_id).map_err(|_| SendCommandErrorDtoV1::InvalidRequest)?;
    let permission_mode = parse_workflow_approval_permission_mode(permission_mode)
        .map_err(|_| SendCommandErrorDtoV1::InvalidRequest)?;
    let outcome = dispatch_durable_send(
        store.inner().as_ref(),
        send_operation.inner().as_ref(),
        journal.inner().as_ref(),
        operation_id,
        CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::WorkflowApproval { execution_id },
            content,
            permission_mode: permission_mode.as_str().to_string(),
            plan_mode: plan_mode.unwrap_or(false),
            backend_id: None,
            model_id: None,
            images: images.unwrap_or_default(),
            mentions: mentions.unwrap_or_default(),
            editor_context: None,
        },
    )
    .await?;
    if let SendCommandOutcomeDtoV1::Accepted { operation } = &outcome {
        if let Ok(Some(readback)) = agent_runtime
            .get_session(&operation.receipt.session_id)
            .await
        {
            crate::adaptor::controller_support::emit_after_workflow_node_message(
                &app,
                &readback.session,
            )
            .await;
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approve_args_accept_optional_node_execution_address() {
        let args = parse_approve_workflow_node_args(&serde_json::json!({
            "executionId": "00000000-0000-0000-0000-000000000001",
            "nodeName": "review",
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
        let args = parse_approve_workflow_node_args(&serde_json::json!({
            "executionId": "00000000-0000-0000-0000-000000000001",
            "nodeName": "review",
        }))
        .unwrap();

        assert!(args.node_execution_id.is_none());
    }
}
