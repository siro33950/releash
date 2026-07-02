use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::workflow::pending_command::{
    PendingCommand, PendingCommandPayload, PendingCommandStore, DEFAULT_PENDING_TTL_SECS,
};
use crate::adaptor::gateway::workflow::runtime_state::ApprovalDecision as RuntimeApprovalDecision;
use crate::domain::agent_session::PermissionMode;
use crate::domain::app_config::ConfigRepository;
use crate::domain::workflow::{
    ApprovalDecision, TriggerSource, WorkflowDefinition, WorkflowError, WorkflowStateSnapshot,
};
use crate::infrastructure::platform::app_data_dir::resolve_data_dir;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{MessagePart, OpenTabRegistry, SessionStore};
use crate::usecase::agent_session::status::current_timestamp;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::command::{
    AbortRunCommand, ApprovalCommand, ResolvedStartRunCommand, SubmitOutputCommand,
};
use crate::usecase::workflow::ports::{
    ApprovalChatTarget, PendingRuntimeCommand, PendingRuntimeCommandOutcome,
    PendingRuntimeCommandPayload, WorkflowAbortRunGateway, WorkflowApprovalChatGateway,
    WorkflowApprovalGateway, WorkflowPendingRuntimeCommandGateway, WorkflowRuntimeStateGateway,
    WorkflowStartRunGateway, WorkflowSubmitOutputGateway, WorkflowTurnCompleteCommand,
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
                engine_for_init.set_run_store_data_dir(data_dir).await;
                let _ = engine_for_init
                    .recover_orphan_runs(&app_handle_for_init)
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
impl WorkflowStartRunGateway for TauriWorkflowRuntimeCommandGateway {
    async fn resolve_start_run_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowError> {
        self.engine
            .resolve_start_run_worktree(worktree_path)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }

    async fn resolve_start_run_workflow(
        &self,
        workflow_file_stem: &str,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let workflow = self
            .engine
            .resolve_start_run_workflow(workflow_file_stem)
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))?;
        super::mapper::legacy_workflow_to_domain(workflow)
    }

    async fn start_resolved_run(
        &self,
        command: ResolvedStartRunCommand,
    ) -> Result<String, WorkflowError> {
        let permission_mode = PermissionMode::parse(&command.permission_mode)
            .map_err(|err| WorkflowError::validation(err.to_string()))?;
        let workflow = super::mapper::domain_workflow_to_legacy(&command.workflow)?;
        self.engine
            .start_resolved_workflow(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                workflow,
                command.worktree_path,
                &command.workflow_file_stem,
                command.task,
                domain_trigger_source_to_legacy(command.trigger_source),
                permission_mode,
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowAbortRunGateway for TauriWorkflowRuntimeCommandGateway {
    async fn abort_run(&self, command: AbortRunCommand) -> Result<(), WorkflowError> {
        self.engine
            .abort_workflow_run(
                &self.app,
                &self.session_store,
                &self.agent_runtime,
                &command.run_id,
                command.expected_node_name.as_deref(),
            )
            .await
            .map_err(|err| WorkflowError::external(err.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowApprovalGateway for TauriWorkflowRuntimeCommandGateway {
    async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        match approval_command_to_runtime_resolution(command) {
            RuntimeApprovalResolution::Decision {
                run_id,
                node_name,
                decision,
                approval_comment,
            } => self
                .engine
                .resolve_workflow_approval(
                    &self.app,
                    &self.session_store,
                    &self.agent_runtime,
                    &run_id,
                    decision,
                    approval_comment,
                    node_name.as_deref(),
                )
                .await
                .map_err(|err| WorkflowError::external(err.to_string())),
            RuntimeApprovalResolution::Abort {
                run_id,
                expected_node_name,
            } => self
                .engine
                .abort_workflow_run(
                    &self.app,
                    &self.session_store,
                    &self.agent_runtime,
                    &run_id,
                    expected_node_name.as_deref(),
                )
                .await
                .map_err(|err| WorkflowError::external(err.to_string())),
        }
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
                &command.run_id,
                command.step_name,
                command.contract,
                command.structured_output,
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
            run_id: command.run_id,
            requested_at: command.requested_at,
            payload: pending_runtime_payload_to_legacy(command.payload),
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
impl WorkflowRuntimeStateGateway for TauriWorkflowRuntimeCommandGateway {
    async fn get_state_by_run_id(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        Ok(self
            .engine
            .get_state_by_run_id(run_id)
            .await
            .map(crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot))
    }

    async fn get_state_by_worktree(
        &self,
        worktree_path: &str,
    ) -> Result<Option<WorkflowStateSnapshot>, WorkflowError> {
        Ok(self
            .engine
            .get_state(worktree_path)
            .await
            .map(crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot))
    }
}

#[async_trait::async_trait]
impl WorkflowApprovalChatGateway for TauriWorkflowRuntimeCommandGateway {
    async fn resolve_approval_chat_target(
        &self,
        run_id: &str,
    ) -> Result<ApprovalChatTarget, WorkflowError> {
        let (chat_session_id, worktree_path) = self
            .engine
            .resolve_chat_session_for_approval(run_id)
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

fn domain_trigger_source_to_legacy(
    source: TriggerSource,
) -> crate::adaptor::gateway::workflow::run::TriggerSource {
    match source {
        TriggerSource::DesktopUi => {
            crate::adaptor::gateway::workflow::run::TriggerSource::DesktopUi
        }
        TriggerSource::Remote => crate::adaptor::gateway::workflow::run::TriggerSource::Remote,
        TriggerSource::Cli => crate::adaptor::gateway::workflow::run::TriggerSource::Cli,
        TriggerSource::Agent => crate::adaptor::gateway::workflow::run::TriggerSource::Agent,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum RuntimeApprovalResolution {
    Decision {
        run_id: String,
        node_name: Option<String>,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
    },
    Abort {
        run_id: String,
        expected_node_name: Option<String>,
    },
}

fn approval_command_to_runtime_resolution(command: ApprovalCommand) -> RuntimeApprovalResolution {
    match command.decision {
        ApprovalDecision::Approve { comment } => RuntimeApprovalResolution::Decision {
            run_id: command.run_id,
            node_name: command.node_name,
            decision: RuntimeApprovalDecision::Approve,
            approval_comment: comment,
        },
        ApprovalDecision::Reject { reason } => RuntimeApprovalResolution::Decision {
            run_id: command.run_id,
            node_name: command.node_name,
            decision: RuntimeApprovalDecision::Reject {
                comment: reason.clone(),
            },
            approval_comment: Some(reason),
        },
        ApprovalDecision::Abort => RuntimeApprovalResolution::Abort {
            run_id: command.run_id,
            expected_node_name: command.node_name,
        },
    }
}

fn pending_runtime_payload_to_legacy(
    payload: PendingRuntimeCommandPayload,
) -> PendingCommandPayload {
    match payload {
        PendingRuntimeCommandPayload::Approve { node_name, comment } => {
            PendingCommandPayload::Approve { node_name, comment }
        }
        PendingRuntimeCommandPayload::Reject { node_name, reason } => {
            PendingCommandPayload::Reject { node_name, reason }
        }
        PendingRuntimeCommandPayload::Abort { node_name } => {
            PendingCommandPayload::Abort { node_name }
        }
        PendingRuntimeCommandPayload::SubmitOutput {
            step_name,
            contract,
            structured_output,
        } => PendingCommandPayload::SubmitOutput {
            step_name,
            contract,
            structured_output,
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
    fn maps_trigger_source_to_legacy_vocab() {
        assert_eq!(
            domain_trigger_source_to_legacy(TriggerSource::DesktopUi),
            crate::adaptor::gateway::workflow::run::TriggerSource::DesktopUi
        );
        assert_eq!(
            domain_trigger_source_to_legacy(TriggerSource::Remote),
            crate::adaptor::gateway::workflow::run::TriggerSource::Remote
        );
        assert_eq!(
            domain_trigger_source_to_legacy(TriggerSource::Cli),
            crate::adaptor::gateway::workflow::run::TriggerSource::Cli
        );
        assert_eq!(
            domain_trigger_source_to_legacy(TriggerSource::Agent),
            crate::adaptor::gateway::workflow::run::TriggerSource::Agent
        );
    }

    #[test]
    fn maps_approval_decision_to_runtime_resolution() {
        let approve = approval_command_to_runtime_resolution(ApprovalCommand {
            run_id: "00000000-0000-0000-0000-000000000001".to_string(),
            node_name: Some("review".to_string()),
            decision: ApprovalDecision::Approve {
                comment: Some("ok".to_string()),
            },
        });
        match approve {
            RuntimeApprovalResolution::Decision {
                run_id,
                node_name,
                decision,
                approval_comment,
            } => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000001");
                assert_eq!(node_name.as_deref(), Some("review"));
                assert_eq!(decision, RuntimeApprovalDecision::Approve);
                assert_eq!(approval_comment.as_deref(), Some("ok"));
            }
            other => panic!("expected approve decision, got {other:?}"),
        }

        let reject = approval_command_to_runtime_resolution(ApprovalCommand {
            run_id: "00000000-0000-0000-0000-000000000001".to_string(),
            node_name: Some("review".to_string()),
            decision: ApprovalDecision::Reject {
                reason: "no".to_string(),
            },
        });
        match reject {
            RuntimeApprovalResolution::Decision {
                decision,
                approval_comment,
                ..
            } => {
                assert_eq!(
                    decision,
                    RuntimeApprovalDecision::Reject {
                        comment: "no".to_string()
                    }
                );
                assert_eq!(approval_comment.as_deref(), Some("no"));
            }
            other => panic!("expected reject decision, got {other:?}"),
        }

        let abort = approval_command_to_runtime_resolution(ApprovalCommand {
            run_id: "00000000-0000-0000-0000-000000000001".to_string(),
            node_name: Some("review".to_string()),
            decision: ApprovalDecision::Abort,
        });
        match abort {
            RuntimeApprovalResolution::Abort {
                expected_node_name, ..
            } => assert_eq!(expected_node_name.as_deref(), Some("review")),
            other => panic!("expected abort resolution, got {other:?}"),
        }
    }
}
