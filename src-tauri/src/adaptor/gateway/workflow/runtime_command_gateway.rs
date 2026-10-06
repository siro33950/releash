use super::workflow_host::WorkflowRuntimeDependencies;
use std::sync::Arc;

use crate::domain::app_config::ConfigRepository;
#[cfg(test)]
use crate::domain::workflow::WorkflowRuntimeSnapshot;
use crate::domain::workflow::{WorkflowDefinition, WorkflowError};
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::command::{AbortExecutionCommand, ResolvedStartExecutionCommand};
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneCommit, WorkflowControlPlaneGateway,
};
use crate::usecase::workflow::ports::{
    WorkflowAbortExecutionGateway, WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway,
    WorkflowStartExecutionGateway,
};

use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;

#[derive(Clone)]
pub struct WorkflowRuntimeCommandGateway {
    app: WorkflowRuntimeDependencies,
    driver: Arc<WorkflowRuntimeHost>,
}

pub struct WorkflowRuntimeCommandGatewayDeps {
    pub node_processes: Arc<super::node_process::WorkflowNodeProcesses>,
    pub isolated_worktrees: Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway>,
    pub repository_usecase: Arc<RepositoryUsecase>,
    pub app_config: Arc<dyn ConfigRepository>,
    pub workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    pub agent_session_launch: Arc<crate::usecase::agent_session::AgentSessionLaunchUsecase>,
    pub agent_session_initial_instruction:
        Arc<crate::usecase::agent_session::AgentSessionInitialInstructionUsecase>,
    pub agent_session_lifecycle: Arc<crate::usecase::agent_session::AgentSessionLifecycleUsecase>,
    pub provider_availability: Arc<dyn crate::domain::agent_session::ProviderAvailabilityReader>,
}

impl WorkflowRuntimeCommandGateway {
    pub fn new_with_driver(
        app: WorkflowRuntimeDependencies,
        driver: Arc<WorkflowRuntimeHost>,
    ) -> Self {
        Self { app, driver }
    }
}

#[async_trait::async_trait]
impl WorkflowStartExecutionGateway for WorkflowRuntimeCommandGateway {
    async fn resolve_start_execution_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowError> {
        self.driver
            .resolve_start_execution_worktree(worktree_path)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn resolve_start_execution_workflow(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowError> {
        let workflow = self
            .driver
            .resolve_start_execution_workflow(workflow_name)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)?;
        super::mapper::schema_workflow_to_domain(workflow)
    }

    async fn start_resolved_execution(
        &self,
        command: ResolvedStartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        let workflow = super::mapper::domain_workflow_to_schema(&command.workflow)?;
        self.driver
            .start_resolved_workflow(
                &self.app,
                workflow,
                command.worktree_path,
                command.request,
                command.created_from,
            )
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

fn workflow_runtime_error_to_workflow_error(error: WorkflowRuntimeError) -> WorkflowError {
    match error {
        WorkflowRuntimeError::Store(kind) => WorkflowError::Store(kind),
        WorkflowRuntimeError::Technical(stopped) => WorkflowError::Technical(stopped),
        WorkflowRuntimeError::InvalidWorkflow(message)
        | WorkflowRuntimeError::ValidationError(message) => WorkflowError::validation(message),
        error @ WorkflowRuntimeError::ExecutionNotFound(_)
        | error @ WorkflowRuntimeError::SessionNotFound(_) => {
            WorkflowError::NotFound(error.to_string())
        }
        error @ WorkflowRuntimeError::AlreadyActive(_) => {
            WorkflowError::InvalidState(error.to_string())
        }
        WorkflowRuntimeError::InvalidState(message) => WorkflowError::InvalidState(message),
        WorkflowRuntimeError::Conflict(message) => WorkflowError::Conflict(message),
        WorkflowRuntimeError::UnauthorizedWorktree(message) => WorkflowError::validation(message),
        WorkflowRuntimeError::UnauthorizedApprovalTarget(message) => {
            WorkflowError::UnauthorizedApprovalTarget(message)
        }
        WorkflowRuntimeError::SessionStore(message)
        | WorkflowRuntimeError::AgentSession(message) => WorkflowError::external(message),
    }
}

#[async_trait::async_trait]
impl WorkflowAbortExecutionGateway for WorkflowRuntimeCommandGateway {
    async fn abort_execution(&self, command: AbortExecutionCommand) -> Result<(), WorkflowError> {
        self.driver
            .abort_workflow_execution(
                &self.app,
                &command.execution_id,
                command.expected_node_name.as_deref(),
            )
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl WorkflowControlPlaneGateway for WorkflowRuntimeCommandGateway {
    fn current_timestamp(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64())
    }

    fn new_node_execution_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    async fn resolve_workflow_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        let store = self.app.store.clone().ok_or_else(|| {
            WorkflowError::external("workflow SQLite event authority is not managed")
        })?;
        super::fact_log::FactLogReadBackend::Live(store)
            .tree_id_for_node(node_execution_id)
            .await
            .map_err(WorkflowError::from)
    }

    async fn load_active_execution(
        &self,
        execution_id: &str,
    ) -> Result<
        Option<crate::domain::workflow::entities::workflow_execution::ExecutionTree>,
        WorkflowError,
    > {
        self.driver
            .load_control_plane_execution(&self.app, execution_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    fn node_process_presence(
        &self,
        execution: &crate::domain::workflow::entities::workflow_execution::ExecutionTree,
        node_execution_id: &str,
    ) -> Result<crate::domain::workflow::NodeProcessPresence, WorkflowError> {
        use crate::domain::workflow::NodeProcessReader;
        let node = execution
            .node_execution(node_execution_id)
            .ok_or_else(|| WorkflowError::NotFound(node_execution_id.into()))?;
        self.driver.node_processes.presence(
            &execution.workspace_identity,
            node_execution_id,
            node.kind,
            node.session_id.as_deref(),
        )
    }

    fn worktree_exists(&self, worktree_path: &str) -> Result<bool, WorkflowError> {
        match std::fs::metadata(worktree_path) {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(WorkflowError::external(format!(
                "read execution worktree: {error}"
            ))),
        }
    }

    async fn session_conversation_exists(&self, session_id: &str) -> Result<bool, WorkflowError> {
        self.driver
            .session_conversation_exists(session_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn resume_session_process(
        &self,
        execution_id: &str,
        node_execution_id: &str,
        session_id: &str,
    ) -> Result<(), WorkflowError> {
        self.driver
            .resume_session_process(&self.app, execution_id, node_execution_id, session_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn register_started_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        self.driver
            .register_started_execution_tree(&self.app, tree_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn release_deleted_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        self.driver
            .release_deleted_execution_tree(tree_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn approval_persisted(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<bool, WorkflowError> {
        let store = self.app.store.clone().ok_or_else(|| {
            WorkflowError::external("workflow SQLite event authority is not managed")
        })?;
        let records = super::fact_log::read_tree_records(&store, execution_id)
            .await
            .map_err(WorkflowError::from)?;
        Ok(records.iter().any(|record| {
            matches!(
                record.fact,
                crate::domain::workflow::NodeFact::ApprovalGranted(_)
            ) && record.meta.node_name == node_name
                && node_execution_id
                    .is_none_or(|expected| expected == record.meta.node_execution_id)
        }))
    }

    fn configured_secret_values(&self) -> Vec<String> {
        super::secret_source::collect_configured_secret_values(&self.app)
    }

    fn approval_auto_approve_enabled(&self) -> bool {
        super::workflow_host::approval_runtime::workflow_approval_auto_approve_enabled(&self.app)
    }

    async fn commit_control_plane(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot, WorkflowError>
    {
        self.driver
            .commit_workflow_control_plane(&self.app, commit)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }

    async fn finish_control_plane_commit(
        &self,
        worktree_path: &str,
        snapshot: &crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot,
        outcome: Option<crate::usecase::workflow::runtime_driver::NodeOutcome>,
    ) -> Result<(), WorkflowError> {
        self.driver
            .finish_workflow_control_plane_commit(&self.app, worktree_path, snapshot, outcome)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl crate::usecase::workflow::ports::ExecutionTreeProcessGateway
    for WorkflowRuntimeCommandGateway
{
    async fn stop_execution_tree_processes(&self, execution_id: &str) -> Result<(), WorkflowError> {
        self.driver
            .stop_execution_tree_processes(&self.app, execution_id)
            .await
            .map_err(workflow_runtime_error_to_workflow_error)
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeStateGateway for WorkflowRuntimeCommandGateway {
    #[cfg(test)]
    async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        Ok(self
            .driver
            .get_state_by_execution_id(&self.app, execution_id)
            .await
            .map(
            crate::adaptor::gateway::workflow::state::runtime_commit_snapshot_to_domain_snapshot,
        ))
    }
}

#[async_trait::async_trait]
impl WorkflowRuntimeShutdownGateway for WorkflowRuntimeCommandGateway {
    async fn shutdown_active_commands(&self) {
        self.driver.shutdown_all_active_commands().await;
    }
}

#[cfg(test)]
#[path = "runtime_command_gateway_test.rs"]
mod runtime_command_gateway_tests;

#[cfg(feature = "test-support")]
impl WorkflowRuntimeCommandGateway {
    pub fn test_app(&self) -> &WorkflowRuntimeDependencies {
        &self.app
    }
}
