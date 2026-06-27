use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adaptor::controller::agent_session::message_dispatch::AgentMessageDispatchRequest;
use crate::adaptor::controller::command::workflow::{
    parse_workflow_approval_permission_mode, parse_workflow_start_permission_mode, validate_run_id,
};
use crate::adaptor::controller_support::{
    build_workflow_state_view, dispatch_agent_message_with_runtime, AgentBackendRegistryState,
    AgentImageAttachment, AgentProcessMapState, AgentSendMessageResponse, OpenTabRegistryState,
    SessionStoreState,
};
use crate::adaptor::protocol::workflow::WorkflowStateView;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::workflow::command::{AbortRunCommand, ApprovalCommand, StartRunCommand};
use crate::usecase::workflow::WorkflowRuntimeUsecase;
use tauri::Manager;

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

/// approval UI / Tauri command 境界からの判断入力 DTO。
///
/// [04] Command / Event Boundary: engine 内部の `ApprovalDecision` には依存させず、
/// command 境界専用の DTO として usecase command への変換責務だけを担う。
/// wire 形式: `{"approve":{"comment":...}}` / `{"reject":{"reason":...}}` / `"abort"`。
/// 旧 unit variant `"approve"` は受理しない。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionInput {
    Approve {
        #[serde(default)]
        comment: Option<String>,
    },
    Reject {
        reason: String,
    },
    Abort,
}

impl ApprovalDecisionInput {
    pub(super) fn into_approval_command(
        self,
        run_id: String,
        step_name: String,
    ) -> ApprovalCommand {
        let decision = match self {
            Self::Approve { comment } => {
                crate::domain::workflow::ApprovalDecision::Approve { comment }
            }
            Self::Reject { reason } => crate::domain::workflow::ApprovalDecision::Reject { reason },
            Self::Abort => crate::domain::workflow::ApprovalDecision::Abort,
        };
        ApprovalCommand {
            run_id,
            node_name: Some(step_name),
            decision,
        }
    }
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
    handles: tauri::State<'_, AgentProcessMapState>,
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
            build_workflow_state_view(state, handles.inner(), open_tabs.inner()).await,
        )),
        None => Ok(None),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn approve_workflow_step(
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    run_id: String,
    decision: ApprovalDecisionInput,
    step_name: String,
) -> Result<(), String> {
    validate_run_id(&run_id)?;
    runtime
        .resolve_approval(decision.into_approval_command(run_id, step_name))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_workflow_approval_chat_message(
    app: tauri::AppHandle,
    handles: tauri::State<'_, AgentProcessMapState>,
    session_store: tauri::State<'_, SessionStoreState>,
    registry: tauri::State<'_, AgentBackendRegistryState>,
    branch_diff_context: tauri::State<'_, Arc<dyn BranchDiffContextPort>>,
    runtime: tauri::State<'_, Arc<WorkflowRuntimeUsecase>>,
    run_id: String,
    content: String,
    permission_mode: Option<String>,
    plan_mode: Option<bool>,
    images: Option<Vec<AgentImageAttachment>>,
    mentions: Option<Vec<crate::adaptor::protocol::mention::MentionReferenceInput>>,
    client_sent_at_ms: Option<f64>,
) -> Result<AgentSendMessageResponse, String> {
    let request_received_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs_f64() * 1000.0);
    // Spec issues-1011 line 121: 起動以外の workflow 操作 API は run_id を主語に取る。
    // chat_session_id / worktree_path は run_id から workflow runtime usecase が解決する。
    validate_run_id(&run_id)?;
    let permission_mode = parse_workflow_approval_permission_mode(permission_mode)?;
    let mentions = mentions.map(crate::adaptor::protocol::mention::into_domain_vec);

    let approval_target = runtime
        .prepare_approval_chat(&run_id, &content)
        .await
        .map_err(|e| e.to_string())?;

    let response = dispatch_agent_message_with_runtime(
        &app,
        branch_diff_context.inner(),
        session_store.inner(),
        registry.inner(),
        handles.inner(),
        AgentMessageDispatchRequest {
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
            client_sent_at_ms,
            request_received_at_ms,
        },
    )
    .await?;
    crate::adaptor::controller_support::emit_after_workflow_step_message(
        &app,
        &response.session,
        handles.inner(),
        app.state::<OpenTabRegistryState>().inner(),
    )
    .await;
    Ok(response)
}
