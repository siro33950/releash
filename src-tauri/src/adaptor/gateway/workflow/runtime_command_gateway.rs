use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::workflow::pending_command::{
    PendingCommand, PendingCommandPayload, PendingCommandStore, DEFAULT_PENDING_TTL_SECS,
};
use crate::domain::agent_session::PermissionMode;
use crate::domain::app_config::ConfigRepository;
use crate::domain::workflow::{WorkflowDefinition, WorkflowError, WorkflowRuntimeSnapshot};
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{MessagePart, OpenTabRegistry, SessionStore};
use crate::usecase::agent_session::status::current_timestamp;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, ResolvedStartExecutionCommand, SubmitOutputCommand,
};
use crate::usecase::workflow::ports::{
    ApprovalChatTarget, PendingRuntimeCommand, PendingRuntimeCommandOutcome,
    PendingRuntimeCommandPayload, WorkflowAbortExecutionGateway, WorkflowApprovalChatGateway,
    WorkflowApprovalGateway, WorkflowPendingRuntimeCommandGateway, WorkflowRuntimeShutdownGateway,
    WorkflowRuntimeStateGateway, WorkflowStallObservedCommand, WorkflowStallObservedGateway,
    WorkflowStartExecutionGateway, WorkflowSubmitOutputGateway, WorkflowTurnCompleteCommand,
    WorkflowTurnCompleteGateway, WorkflowTurnFailureSignal,
};

use super::pending_command_dispatcher::{
    dispatch_pending_command as dispatch_legacy_pending_command, process_pending_command_entry,
    PendingCommandDispatchOutcome,
};
use super::runtime_engine::{new_workflow_runtime_engine, WorkflowRuntimeEngine};
use super::runtime_resolver::{
    AppConfigManagedWorktreeResolver, DefaultWorkflowDefinitionResolver,
};

#[derive(Clone)]
pub(crate) struct TauriWorkflowRuntimeCommandGateway {
    app: tauri::AppHandle,
    engine: Arc<dyn WorkflowRuntimeEngine>,
    session_store: Arc<SessionStore>,
    agent_runtime: Arc<AgentSessionRuntimeUsecase>,
}

pub(crate) struct TauriWorkflowRuntimeCommandGatewayDeps {
    pub(crate) repository_usecase: Arc<RepositoryUsecase>,
    pub(crate) app_config: Arc<dyn ConfigRepository>,
    pub(crate) session_store: Arc<SessionStore>,
    pub(crate) agent_runtime: Arc<AgentSessionRuntimeUsecase>,
    pub(crate) open_tabs: Arc<OpenTabRegistry>,
    pub(crate) branch_diff_context: Arc<dyn BranchDiffContextPort>,
    pub(crate) data_dir: Option<PathBuf>,
}

impl TauriWorkflowRuntimeCommandGateway {
    pub(crate) fn new(
        app: tauri::AppHandle,
        engine: Arc<dyn WorkflowRuntimeEngine>,
        session_store: Arc<SessionStore>,
        agent_runtime: Arc<AgentSessionRuntimeUsecase>,
    ) -> Self {
        Self {
            app,
            engine,
            session_store,
            agent_runtime,
        }
    }

    pub(crate) fn new_with_default_engine(
        app: tauri::AppHandle,
        deps: TauriWorkflowRuntimeCommandGatewayDeps,
    ) -> Self {
        let TauriWorkflowRuntimeCommandGatewayDeps {
            repository_usecase,
            app_config,
            session_store,
            agent_runtime,
            open_tabs,
            branch_diff_context,
            data_dir,
        } = deps;
        let engine = new_workflow_runtime_engine(
            Arc::new(DefaultWorkflowDefinitionResolver),
            Arc::new(AppConfigManagedWorktreeResolver::new(
                repository_usecase,
                app_config,
            )),
            Some(branch_diff_context),
            open_tabs,
        );
        if let Some(data_dir) = data_dir {
            let engine_for_init = engine.clone();
            let app_handle_for_init = app.clone();
            tauri::async_runtime::block_on(async move {
                engine_for_init.set_execution_store_data_dir(data_dir).await;
                let _ = engine_for_init
                    .recover_orphan_executions(&app_handle_for_init)
                    .await;
            });
        }
        Self::new(app, engine, session_store, agent_runtime)
    }

    async fn process_pending_submit_output_pickup(&self, store: &PendingCommandStore) {
        if let Err(e) = store.cleanup_expired(current_timestamp(), DEFAULT_PENDING_TTL_SECS) {
            log::warn!("pending command cleanup_expired failed: {e}");
        }
        if let Err(e) =
            store.requeue_unexpired_processing(current_timestamp(), DEFAULT_PENDING_TTL_SECS)
        {
            log::warn!("pending command processing orphan requeue failed: {e}");
        }

        let entries = match store.list_pending() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("pending command list_pending failed: {e}");
                return;
            }
        };
        if entries.is_empty() {
            return;
        }

        for entry in entries {
            if matches!(
                entry.command.payload,
                PendingCommandPayload::SubmitOutput { .. }
            ) {
                process_pending_command_entry(
                    &self.app,
                    self.engine.as_ref(),
                    &self.session_store,
                    &self.agent_runtime,
                    store,
                    entry,
                )
                .await;
            }
        }
    }
}

#[async_trait::async_trait]
impl WorkflowStartExecutionGateway for TauriWorkflowRuntimeCommandGateway {
    async fn resolve_start_execution_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowError> {
        self.engine
            .resolve_start_execution_worktree(worktree_path)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }

    async fn resolve_start_execution_workflow(
        &self,
        workflow_file_stem: &str,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let workflow = self
            .engine
            .resolve_start_execution_workflow(workflow_file_stem)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))?;
        super::mapper::schema_workflow_to_domain(workflow)
    }

    async fn start_resolved_execution(
        &self,
        command: ResolvedStartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        let permission_mode = PermissionMode::parse(&command.permission_mode)
            .map_err(|err| WorkflowError::validation(err.to_string()))?;
        let workflow = super::mapper::domain_workflow_to_schema(&command.workflow)?;
        self.engine
            .start_resolved_workflow(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                workflow,
                command.worktree_path,
                command.request,
                command.created_from,
                permission_mode,
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowAbortExecutionGateway for TauriWorkflowRuntimeCommandGateway {
    async fn abort_execution(&self, command: AbortExecutionCommand) -> Result<(), WorkflowError> {
        self.engine
            .abort_workflow_execution(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                &command.execution_id,
                command.expected_node_name.as_deref(),
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowApprovalGateway for TauriWorkflowRuntimeCommandGateway {
    async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.engine
            .resolve_workflow_approval(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                &command.execution_id,
                command.comment,
                &command.node_name,
                command.node_execution_id.as_deref(),
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowSubmitOutputGateway for TauriWorkflowRuntimeCommandGateway {
    async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        self.engine
            .submit_workflow_output(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                &command.execution_id,
                command.node_name,
                command.node_execution_id,
                command.contract,
                command.artifact,
                None,
                None,
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowPendingRuntimeCommandGateway for TauriWorkflowRuntimeCommandGateway {
    async fn dispatch_pending_command(
        &self,
        command: PendingRuntimeCommand,
    ) -> PendingRuntimeCommandOutcome {
        let pending = PendingCommand {
            id: command.request_id,
            execution_id: command.execution_id,
            requested_at: command.requested_at,
            payload: pending_runtime_payload_to_gateway(command.payload),
        };
        dispatch_legacy_pending_command(
            &self.app,
            self.engine.as_ref(),
            &self.session_store,
            &self.agent_runtime,
            pending,
        )
        .await
        .into()
    }
}

#[async_trait::async_trait]
impl WorkflowTurnCompleteGateway for TauriWorkflowRuntimeCommandGateway {
    async fn is_session_running(&self, chat_session_id: &str) -> bool {
        self.engine.is_running(chat_session_id).await
    }

    async fn pickup_pending_submit_outputs(&self) {
        match resolve_data_dir(&self.app) {
            Ok(data_dir) => {
                let store = PendingCommandStore::new(&data_dir);
                self.process_pending_submit_output_pickup(&store).await;
            }
            Err(err) => {
                log::warn!("pending SubmitOutput pickup skipped: resolve_data_dir failed: {err}");
            }
        }
    }

    async fn complete_turn(
        &self,
        command: WorkflowTurnCompleteCommand,
    ) -> Result<(), WorkflowError> {
        let final_parts = command
            .final_text_parts
            .into_iter()
            .map(|content| MessagePart::Text {
                content,
                parent_tool_use_id: None,
            })
            .collect::<Vec<_>>();
        let token_usage = command
            .token_usage
            .map(|usage| (usage.input_tokens, usage.output_tokens));

        self.engine
            .on_turn_complete(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                &command.chat_session_id,
                command.exit_code,
                command.failure_signal.map(|signal| match signal {
                    WorkflowTurnFailureSignal::ModelRefusal => {
                        crate::domain::workflow::services::transition::SessionFailureSignal::ModelRefusal
                    }
                }),
                &final_parts,
                token_usage,
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowStallObservedGateway for TauriWorkflowRuntimeCommandGateway {
    async fn observe_stall(
        &self,
        command: WorkflowStallObservedCommand,
    ) -> Result<(), WorkflowError> {
        self.engine
            .on_agent_stall_observed(
                &self.app,
                &command.chat_session_id,
                command.turn_phase,
                command.idle_secs,
                command.signal_count,
                command.cap_reached,
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }

    async fn clear_stall(
        &self,
        command: crate::usecase::workflow::ports::WorkflowStallClearedCommand,
    ) -> Result<(), WorkflowError> {
        self.engine
            .on_agent_stall_cleared(&self.app, &command.chat_session_id)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeStateGateway for TauriWorkflowRuntimeCommandGateway {
    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        Ok(self
            .engine
            .get_state_by_execution_id(execution_id)
            .await
            .map(crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot))
    }

    async fn get_state_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        Ok(self
            .engine
            .get_state(worktree_path)
            .await
            .map(crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot))
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeShutdownGateway for TauriWorkflowRuntimeCommandGateway {
    async fn shutdown_active_commands(&self) {
        self.engine.shutdown_all_active_commands().await;
    }
}

#[async_trait::async_trait]
impl WorkflowApprovalChatGateway for TauriWorkflowRuntimeCommandGateway {
    async fn resolve_approval_chat_target(
        &self,
        execution_id: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError> {
        let (chat_session_id, worktree_path) = self
            .engine
            .resolve_chat_session_for_approval(execution_id)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))?;
        Ok(ApprovalChatTarget {
            chat_session_id,
            worktree_path,
        })
    }

    async fn validate_approval_chat_instruction(
        &self,
        chat_session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowError> {
        self.engine
            .validate_approval_chat_instruction(chat_session_id, content)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

fn pending_runtime_payload_to_gateway(
    payload: PendingRuntimeCommandPayload,
) -> PendingCommandPayload {
    match payload {
        PendingRuntimeCommandPayload::Approve {
            node_name,
            node_execution_id,
            comment,
        } => PendingCommandPayload::Approve {
            node_name,
            node_execution_id,
            comment,
        },
        PendingRuntimeCommandPayload::Abort { node_name } => {
            PendingCommandPayload::Abort { node_name }
        }
        PendingRuntimeCommandPayload::SubmitOutput {
            node_name,
            node_execution_id,
            contract,
            artifact,
        } => PendingCommandPayload::SubmitOutput {
            node_name,
            node_execution_id,
            contract,
            artifact,
        },
    }
}

impl From<PendingCommandDispatchOutcome> for PendingRuntimeCommandOutcome {
    fn from(outcome: PendingCommandDispatchOutcome) -> Self {
        match outcome {
            PendingCommandDispatchOutcome::Accepted => Self::Accepted,
            PendingCommandDispatchOutcome::RejectedFinal(reason) => Self::RejectedFinal(reason),
            PendingCommandDispatchOutcome::RetryableFailure(reason) => {
                Self::RetryableFailure(reason)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_payload_mapping_preserves_node_execution_address() {
        assert_eq!(
            pending_runtime_payload_to_gateway(PendingRuntimeCommandPayload::Approve {
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-review".to_string()),
                comment: None,
            }),
            PendingCommandPayload::Approve {
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-review".to_string()),
                comment: None,
            }
        );
        assert_eq!(
            pending_runtime_payload_to_gateway(PendingRuntimeCommandPayload::SubmitOutput {
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-review".to_string()),
                contract: "review-verdict".to_string(),
                artifact: serde_json::json!({"verdict": "LGTM"}),
            }),
            PendingCommandPayload::SubmitOutput {
                node_name: "review".to_string(),
                node_execution_id: Some("node-execution-review".to_string()),
                contract: "review-verdict".to_string(),
                artifact: serde_json::json!({"verdict": "LGTM"}),
            }
        );
    }
}
