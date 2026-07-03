use std::sync::Arc;

use async_trait::async_trait;

use super::runtime_engine_impl::WorkflowRuntimeService;
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::route_context::CommandCommitContext;
use crate::adaptor::gateway::workflow::runtime_state::ApprovalDecision as RuntimeApprovalDecision;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::SessionStore;

#[allow(clippy::too_many_arguments)]
#[async_trait]
pub(crate) trait PendingCommandRuntime<R: tauri::Runtime>: Send + Sync {
    fn output_submitted_already_recorded(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError>;

    fn cli_mutation_already_recorded(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError>;

    async fn ensure_execution_loaded_for_external(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        run_id: &str,
    ) -> Result<(), WorkflowEngineError>;

    async fn resolve_workflow_approval_with_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
        node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError>;

    async fn abort_workflow_run_with_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        expected_node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError>;

    async fn submit_workflow_output(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
        request_id: Option<String>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError>;

    async fn append_command_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        commit_context: CommandCommitContext,
    ) -> Result<(), WorkflowEngineError>;

    async fn append_cli_mutation_rejected_for_submit_output(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        commit_context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError>;

    async fn append_cli_mutation_rejected(
        &self,
        app: &tauri::AppHandle<R>,
        commit_context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError>;
}

#[async_trait]
impl<R, T> PendingCommandRuntime<R> for Arc<T>
where
    R: tauri::Runtime,
    T: PendingCommandRuntime<R> + ?Sized,
{
    fn output_submitted_already_recorded(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError> {
        self.as_ref()
            .output_submitted_already_recorded(app, run_id, request_id)
    }

    fn cli_mutation_already_recorded(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError> {
        self.as_ref()
            .cli_mutation_already_recorded(app, run_id, request_id)
    }

    async fn ensure_execution_loaded_for_external(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        run_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .ensure_execution_loaded_for_external(app, session_store, run_id)
            .await
    }

    async fn resolve_workflow_approval_with_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
        node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .resolve_workflow_approval_with_commit_context(
                app,
                session_store,
                agent_runtime,
                run_id,
                decision,
                approval_comment,
                node_name,
                commit_context,
            )
            .await
    }

    async fn abort_workflow_run_with_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        expected_node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .abort_workflow_run_with_commit_context(
                app,
                session_store,
                agent_runtime,
                run_id,
                expected_node_name,
                commit_context,
            )
            .await
    }

    async fn submit_workflow_output(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
        request_id: Option<String>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .submit_workflow_output(
                app,
                session_store,
                agent_runtime,
                run_id,
                step_name,
                contract,
                structured_output,
                request_id,
                submitted_at,
            )
            .await
    }

    async fn append_command_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        commit_context: CommandCommitContext,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .append_command_commit_context(app, commit_context)
            .await
    }

    async fn append_cli_mutation_rejected_for_submit_output(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        commit_context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .append_cli_mutation_rejected_for_submit_output(app, run_id, commit_context, error)
            .await
    }

    async fn append_cli_mutation_rejected(
        &self,
        app: &tauri::AppHandle<R>,
        commit_context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError> {
        self.as_ref()
            .append_cli_mutation_rejected(app, commit_context, error)
            .await
    }
}

#[async_trait]
impl<R: tauri::Runtime> PendingCommandRuntime<R> for WorkflowRuntimeService {
    fn output_submitted_already_recorded(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError> {
        WorkflowRuntimeService::output_submitted_already_recorded(self, app, run_id, request_id)
    }

    fn cli_mutation_already_recorded(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError> {
        WorkflowRuntimeService::cli_mutation_already_recorded(self, app, run_id, request_id)
    }

    async fn ensure_execution_loaded_for_external(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        run_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::ensure_execution_loaded_for_external(
            self,
            app,
            session_store,
            run_id,
        )
        .await
    }

    async fn resolve_workflow_approval_with_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
        node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::resolve_workflow_approval_with_commit_context(
            self,
            app,
            session_store,
            agent_runtime,
            run_id,
            decision,
            approval_comment,
            node_name,
            commit_context,
        )
        .await
    }

    async fn abort_workflow_run_with_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        expected_node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::abort_workflow_run_with_commit_context(
            self,
            app,
            session_store,
            agent_runtime,
            run_id,
            expected_node_name,
            commit_context,
        )
        .await
    }

    async fn submit_workflow_output(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
        request_id: Option<String>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::submit_workflow_output(
            self,
            app,
            session_store,
            agent_runtime,
            run_id,
            step_name,
            contract,
            structured_output,
            request_id,
            submitted_at,
        )
        .await
    }

    async fn append_command_commit_context(
        &self,
        app: &tauri::AppHandle<R>,
        commit_context: CommandCommitContext,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::append_command_commit_context(self, app, commit_context).await
    }

    async fn append_cli_mutation_rejected_for_submit_output(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        commit_context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::append_cli_mutation_rejected_for_submit_output(
            self,
            app,
            run_id,
            commit_context,
            error,
        )
        .await
    }

    async fn append_cli_mutation_rejected(
        &self,
        app: &tauri::AppHandle<R>,
        commit_context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::append_cli_mutation_rejected(self, app, commit_context, error).await
    }
}
