use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use super::pending_runtime::PendingCommandRuntime;
use super::runtime_engine_impl::WorkflowRuntimeService;
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolver, WorkflowDefinitionResolver,
};
use crate::adaptor::gateway::workflow::run::TriggerSource;
use crate::adaptor::gateway::workflow::runtime_state::ApprovalDecision as RuntimeApprovalDecision;
use crate::adaptor::gateway::workflow::schema::Workflow;
use crate::adaptor::gateway::workflow::state::WorkflowState;
use crate::domain::agent_session::PermissionMode;
use crate::domain::workflow::services::transition::SessionFailureSignal;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{MessagePart, OpenTabRegistry, SessionStore};

#[allow(clippy::too_many_arguments)]
#[async_trait]
pub(crate) trait WorkflowRuntimeEngine: PendingCommandRuntime<tauri::Wry> {
    async fn set_run_store_data_dir(&self, dir: PathBuf);

    async fn recover_orphan_runs(&self, app: &tauri::AppHandle) -> Result<(), WorkflowEngineError>;

    async fn resolve_start_run_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowEngineError>;

    async fn resolve_start_run_workflow(
        &self,
        workflow_file_stem: &str,
    ) -> Result<Workflow, WorkflowEngineError>;

    async fn start_resolved_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        workflow: Workflow,
        worktree_path: String,
        file_stem: &str,
        task: Option<String>,
        trigger_source: TriggerSource,
        permission_mode: PermissionMode,
    ) -> Result<String, WorkflowEngineError>;

    async fn abort_workflow_run(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError>;

    async fn resolve_workflow_approval(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
        node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError>;

    async fn is_running(&self, session_id: &str) -> bool;

    async fn on_turn_complete(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        chat_session_id: &str,
        exit_code: i64,
        failure_signal: Option<SessionFailureSignal>,
        final_parts: &[MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowEngineError>;

    async fn on_agent_stall_observed(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
        turn_phase: String,
        idle_secs: u64,
        signal_count: u32,
        cap_reached: bool,
    ) -> Result<(), WorkflowEngineError>;

    async fn on_agent_stall_cleared(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
    ) -> Result<(), WorkflowEngineError>;

    async fn get_state_by_run_id(&self, run_id: &str) -> Option<WorkflowState>;

    async fn get_state(&self, worktree_path: &str) -> Option<WorkflowState>;

    async fn resolve_chat_session_for_approval(
        &self,
        run_id: &str,
    ) -> Result<(String, String), WorkflowEngineError>;

    async fn validate_approval_chat_instruction(
        &self,
        chat_session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowEngineError>;
}

pub(crate) fn new_workflow_runtime_engine(
    workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
    worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    open_tabs: Arc<OpenTabRegistry>,
) -> Arc<dyn WorkflowRuntimeEngine> {
    Arc::new(WorkflowRuntimeService::new(
        workflow_resolver,
        worktree_resolver,
        branch_diff_context,
        open_tabs,
    ))
}

#[async_trait]
impl WorkflowRuntimeEngine for WorkflowRuntimeService {
    async fn set_run_store_data_dir(&self, dir: PathBuf) {
        WorkflowRuntimeService::set_run_store_data_dir(self, dir).await;
    }

    async fn recover_orphan_runs(&self, app: &tauri::AppHandle) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::recover_orphan_runs(self, app).await;
        Ok(())
    }

    async fn resolve_start_run_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowEngineError> {
        WorkflowRuntimeService::resolve_start_run_worktree(self, worktree_path).await
    }

    async fn resolve_start_run_workflow(
        &self,
        workflow_file_stem: &str,
    ) -> Result<Workflow, WorkflowEngineError> {
        WorkflowRuntimeService::resolve_start_run_workflow(self, workflow_file_stem).await
    }

    async fn start_resolved_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        workflow: Workflow,
        worktree_path: String,
        file_stem: &str,
        task: Option<String>,
        trigger_source: TriggerSource,
        permission_mode: PermissionMode,
    ) -> Result<String, WorkflowEngineError> {
        WorkflowRuntimeService::start_resolved_workflow(
            self,
            app,
            session_store,
            agent_runtime,
            workflow,
            worktree_path,
            file_stem,
            task,
            trigger_source,
            permission_mode,
        )
        .await
    }

    async fn abort_workflow_run(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::abort_workflow_run(
            self,
            app,
            session_store,
            agent_runtime,
            run_id,
            expected_node_name,
        )
        .await
    }

    async fn resolve_workflow_approval(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        run_id: &str,
        decision: RuntimeApprovalDecision,
        approval_comment: Option<String>,
        node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::resolve_workflow_approval(
            self,
            app,
            session_store,
            agent_runtime,
            run_id,
            decision,
            approval_comment,
            node_name,
        )
        .await
    }

    async fn is_running(&self, session_id: &str) -> bool {
        WorkflowRuntimeService::is_running(self, session_id).await
    }

    async fn on_turn_complete(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        chat_session_id: &str,
        exit_code: i64,
        failure_signal: Option<SessionFailureSignal>,
        final_parts: &[MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::on_turn_complete(
            self,
            app,
            session_store,
            agent_runtime,
            chat_session_id,
            exit_code,
            failure_signal,
            final_parts,
            token_usage,
        )
        .await
    }

    async fn on_agent_stall_observed(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
        turn_phase: String,
        idle_secs: u64,
        signal_count: u32,
        cap_reached: bool,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::on_agent_stall_observed(
            self,
            app,
            session_id,
            turn_phase,
            idle_secs,
            signal_count,
            cap_reached,
        )
        .await
    }

    async fn on_agent_stall_cleared(
        &self,
        app: &tauri::AppHandle,
        session_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::on_agent_stall_cleared(self, app, session_id).await
    }

    async fn get_state_by_run_id(&self, run_id: &str) -> Option<WorkflowState> {
        WorkflowRuntimeService::get_state_by_run_id(self, run_id).await
    }

    async fn get_state(&self, worktree_path: &str) -> Option<WorkflowState> {
        WorkflowRuntimeService::get_state(self, worktree_path).await
    }

    async fn resolve_chat_session_for_approval(
        &self,
        run_id: &str,
    ) -> Result<(String, String), WorkflowEngineError> {
        WorkflowRuntimeService::resolve_chat_session_for_approval(self, run_id).await
    }

    async fn validate_approval_chat_instruction(
        &self,
        chat_session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowEngineError> {
        WorkflowRuntimeService::validate_approval_chat_instruction(self, chat_session_id, content)
            .await
    }
}
