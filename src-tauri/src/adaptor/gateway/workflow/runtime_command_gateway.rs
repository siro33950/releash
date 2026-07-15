use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::agent_session::PermissionMode;
use crate::domain::app_config::ConfigRepository;
use crate::domain::workflow::{WorkflowDefinition, WorkflowError, WorkflowRuntimeSnapshot};
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{MessagePart, OpenTabRegistry, SessionStore};
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::command::{
    AbortExecutionCommand, ApprovalCommand, ResolvedStartExecutionCommand, SubmitOutputCommand,
};
use crate::usecase::workflow::ports::{
    ApprovalChatTarget, WorkflowAbortExecutionGateway, WorkflowApprovalChatGateway,
    WorkflowApprovalGateway, WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway,
    WorkflowStallObservedCommand, WorkflowStallObservedGateway, WorkflowStartExecutionGateway,
    WorkflowSubmitOutputGateway, WorkflowTurnCompleteCommand, WorkflowTurnCompleteGateway,
    WorkflowTurnFailureSignal,
};

use super::engine_error::WorkflowEngineError;
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
            .map_err(workflow_engine_error_to_workflow_error)
    }

    async fn resolve_start_execution_workflow(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let workflow = self
            .engine
            .resolve_start_execution_workflow(workflow_name)
            .await
            .map_err(workflow_engine_error_to_workflow_error)?;
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
            .map_err(workflow_engine_error_to_workflow_error)
    }
}

fn workflow_engine_error_to_workflow_error(error: WorkflowEngineError) -> WorkflowError {
    match error {
        WorkflowEngineError::InvalidWorkflow(message)
        | WorkflowEngineError::ValidationError(message) => WorkflowError::validation(message),
        error @ WorkflowEngineError::ExecutionNotFound(_)
        | error @ WorkflowEngineError::SessionNotFound(_) => {
            WorkflowError::NotFound(error.to_string())
        }
        error @ WorkflowEngineError::AlreadyActive(_) => {
            WorkflowError::InvalidState(error.to_string())
        }
        WorkflowEngineError::InvalidState(message) => WorkflowError::InvalidState(message),
        WorkflowEngineError::UnauthorizedWorktree(message) => WorkflowError::validation(message),
        WorkflowEngineError::UnauthorizedApprovalTarget(message) => {
            WorkflowError::UnauthorizedApprovalTarget(message)
        }
        WorkflowEngineError::SessionStore(message) | WorkflowEngineError::AgentSession(message) => {
            WorkflowError::external(message)
        }
        WorkflowEngineError::AgentRuntime { message, .. } => WorkflowError::external(message),
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
            .map_err(workflow_engine_error_to_workflow_error)
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
            .map_err(workflow_engine_error_to_workflow_error)
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
            )
            .await
            .map_err(workflow_engine_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl WorkflowTurnCompleteGateway for TauriWorkflowRuntimeCommandGateway {
    async fn is_session_running(&self, chat_session_id: &str) -> bool {
        self.engine.is_running(chat_session_id).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_name_resolution_diagnostics_remain_validation_errors() {
        let error = workflow_engine_error_to_workflow_error(WorkflowEngineError::InvalidWorkflow(
            "workflow_diagnostics: WFS006: duplicate workflow name".to_string(),
        ));

        assert!(matches!(
            error,
            WorkflowError::Validation(message)
                if message.contains("WFS006") && message.contains("duplicate workflow name")
        ));
    }

    #[test]
    fn runtime_command_error_mapping_preserves_domain_variants() {
        assert!(matches!(
            workflow_engine_error_to_workflow_error(WorkflowEngineError::ExecutionNotFound(
                "missing".to_string()
            )),
            WorkflowError::NotFound(message)
                if message == "No workflow execution found for session 'missing'"
        ));
        assert!(matches!(
            workflow_engine_error_to_workflow_error(WorkflowEngineError::InvalidState(
                "terminal".to_string()
            )),
            WorkflowError::InvalidState(message) if message == "terminal"
        ));
        assert!(matches!(
            workflow_engine_error_to_workflow_error(
                WorkflowEngineError::UnauthorizedApprovalTarget("wrong target".to_string())
            ),
            WorkflowError::UnauthorizedApprovalTarget(message) if message == "wrong target"
        ));
        assert!(matches!(
            workflow_engine_error_to_workflow_error(WorkflowEngineError::ValidationError(
                "bad output".to_string()
            )),
            WorkflowError::Validation(message) if message == "bad output"
        ));
        assert!(matches!(
            workflow_engine_error_to_workflow_error(WorkflowEngineError::SessionStore(
                "io".to_string()
            )),
            WorkflowError::External(message) if message == "io"
        ));
    }
}
