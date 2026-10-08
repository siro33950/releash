use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
#[cfg(test)]
use crate::domain::workflow::WorkflowRuntimeSnapshot;

#[cfg(test)]
use super::command::ResolvedStartExecutionCommand;
#[cfg(test)]
use super::command::WorkflowRuntimeCommandPreflight;
use super::command::{
    AbortExecutionCommand, ApprovalCommand, ResumeSessionNodeCommand, RetryNodeCommand,
    StartExecutionCommand, SubmitOutputCommand, WorkflowAbortExecutionUsecase,
    WorkflowRetryNodeUsecase, WorkflowStartExecutionUsecase, WorkflowSubmitOutputUsecase,
};
use super::control_plane::{WorkflowControlPlaneGateway, WorkflowControlPlaneUsecase};
use super::ports::WorkflowRuntimeCommandGateway;
#[cfg(test)]
use super::ports::{
    WorkflowAbortExecutionGateway, WorkflowRuntimeShutdownGateway, WorkflowRuntimeStateGateway,
    WorkflowStartExecutionGateway,
};

#[derive(Clone)]
pub struct WorkflowRuntimeUsecase {
    pub(super) state_publisher:
        Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
    pub(super) worktree_operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    pub(super) execution_archives: Arc<dyn crate::domain::workflow::ExecutionTreeArchiveRepository>,
    pub(super) runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
    pub(super) archive_locks: Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
        >,
    >,
    start_execution: WorkflowStartExecutionUsecase,
    pub(super) abort_execution: WorkflowAbortExecutionUsecase,
    retry_node: WorkflowRetryNodeUsecase,
    submit_output: WorkflowSubmitOutputUsecase,
    control_plane: WorkflowControlPlaneUsecase,
    #[cfg(test)]
    preflight: WorkflowRuntimeCommandPreflight,
}

impl WorkflowRuntimeUsecase {
    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    pub(crate) fn new_with_worktree_operations(
        retrying: std::sync::Arc<crate::usecase::retry::Retrying>,
        runtime: Arc<dyn WorkflowRuntimeCommandGateway>,
        execution_archives: Arc<dyn crate::domain::workflow::ExecutionTreeArchiveRepository>,
        worktree_operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    ) -> Self {
        let control_plane_runtime: Arc<dyn WorkflowControlPlaneGateway> = runtime.clone();
        Self {
            state_publisher: None,
            execution_archives,
            worktree_operations,
            runtime: runtime.clone(),
            archive_locks: Default::default(),
            start_execution: WorkflowStartExecutionUsecase::new(runtime.clone()),
            abort_execution: WorkflowAbortExecutionUsecase::new(runtime.clone()),
            retry_node: WorkflowRetryNodeUsecase::new(
                retrying.clone(),
                control_plane_runtime.clone(),
            ),
            submit_output: WorkflowSubmitOutputUsecase::new(
                retrying.clone(),
                control_plane_runtime.clone(),
            ),
            control_plane: WorkflowControlPlaneUsecase::new(retrying, control_plane_runtime),
            #[cfg(test)]
            preflight: WorkflowRuntimeCommandPreflight,
        }
    }

    pub async fn start_execution(
        &self,
        command: StartExecutionCommand,
    ) -> Result<String, WorkflowError> {
        let _mutation = self.begin_worktree_mutation(&command.worktree_path)?;
        self.start_execution.execute(command).await
    }

    pub async fn abort_execution(
        &self,
        command: AbortExecutionCommand,
    ) -> Result<(), WorkflowError> {
        let _mutation = self
            .begin_execution_tree_mutation(&command.execution_id)
            .await?;
        self.abort_execution.execute(command).await
    }

    pub async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        let _mutation = self
            .begin_execution_tree_mutation(&command.execution_id)
            .await?;
        self.retry_node.execute(command).await
    }

    pub async fn resume_session_node(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        let _mutation = self
            .begin_execution_tree_mutation(&command.execution_id)
            .await?;
        self.control_plane.resume_session_node(command).await
    }

    pub async fn resolve_approval(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        let _mutation = self
            .begin_execution_tree_mutation(&command.execution_id)
            .await?;
        self.control_plane.resolve_approval(command).await
    }

    pub async fn submit_output(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        super::command::WorkflowRuntimeCommandPreflight.validate_submit_output(&command)?;
        let id = self
            .runtime
            .resolve_workflow_execution_id(&command.node_execution_id)
            .await?
            .ok_or_else(|| WorkflowError::NotFound(command.node_execution_id.clone()))?;
        let _mutation = self.begin_execution_tree_mutation(&id).await?;
        self.submit_output.execute(command).await
    }

    pub async fn record_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    ) -> Result<(), WorkflowError> {
        self.control_plane
            .record_provider_stop(command, lifecycle_events)
            .await
    }

    #[cfg(test)]
    pub async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowRuntimeSnapshot>, WorkflowError> {
        self.preflight.validate_execution_lookup(execution_id)?;
        self.runtime.get_state_by_execution_id(execution_id).await
    }

    pub async fn shutdown_active_commands(&self) {
        self.runtime.shutdown_active_commands().await;
    }
}

#[async_trait::async_trait]
impl super::WorkspaceNodeWorkflowCommandExecutor for WorkflowRuntimeUsecase {
    async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        self.resolve_approval(command).await
    }

    async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        self.retry_node(command).await
    }
    async fn resume_session_node(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        self.resume_session_node(command).await
    }
}

#[async_trait::async_trait]
impl crate::usecase::provider_lifecycle::ProviderExecutionTreeStopTransaction
    for WorkflowRuntimeUsecase
{
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeMutationGuard,
        crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    > {
        self.begin_worktree_mutation(path).map_err(|_| {
            crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Conflict
        })
    }

    async fn commit_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    ) -> Result<(), crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError> {
        self.record_provider_stop(command, lifecycle_events)
            .await
            .map_err(|error| match error {
                WorkflowError::Store(kind) => crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(kind),
                WorkflowError::Validation(_)
                | WorkflowError::InvalidState(_)
                | WorkflowError::NotFound(_)
                | WorkflowError::UnauthorizedApprovalTarget(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::InvalidInput
                }
                WorkflowError::Conflict(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Conflict
                }
                error @ (WorkflowError::Technical(_) | WorkflowError::External(_) | WorkflowError::Editor(_)) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Store(error.into())
                }
                WorkflowError::CorruptStoredState(_)
                | WorkflowError::IncompatibleStoredEvent(_) => {
                    crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError::Corrupt
                }
            })
    }
}

impl crate::usecase::agent_session::WorktreeMutationAdmission for WorkflowRuntimeUsecase {
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<crate::usecase::worktree_operation::WorktreeMutationGuard, WorkflowError> {
        self.begin_worktree_mutation(path)
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::ExecutionTreeCache for WorkflowRuntimeUsecase {
    async fn release_deleted_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::usecase::agent_session::ExecutionTreeCacheReleaseError> {
        self.runtime
            .release_deleted_execution_tree(tree_id)
            .await
            .map_err(|error| match error {
                WorkflowError::Store(kind) => {
                    crate::usecase::agent_session::ExecutionTreeCacheReleaseError::Store(kind)
                }
                error @ (WorkflowError::Technical(_)
                | WorkflowError::External(_)
                | WorkflowError::Editor(_)) => {
                    crate::usecase::agent_session::ExecutionTreeCacheReleaseError::Store(
                        error.into(),
                    )
                }
                WorkflowError::Validation(_)
                | WorkflowError::InvalidState(_)
                | WorkflowError::NotFound(_)
                | WorkflowError::UnauthorizedApprovalTarget(_)
                | WorkflowError::Conflict(_)
                | WorkflowError::CorruptStoredState(_)
                | WorkflowError::IncompatibleStoredEvent(_) => {
                    crate::usecase::agent_session::ExecutionTreeCacheReleaseError::Corrupt
                }
            })
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::StartedExecutionTreeRegistrar for WorkflowRuntimeUsecase {
    async fn register_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), crate::usecase::agent_session::StartedExecutionTreeRegistrationError> {
        self.runtime
            .register_started_execution_tree(tree_id)
            .await
            .map_err(map_started_execution_tree_error)
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::AgentSessionExecutionTreeLifecycle for WorkflowRuntimeUsecase {
    async fn lock_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, WorkflowError> {
        self.lock_execution_tree(tree_id).await
    }

    async fn archive_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        self.archive_execution_tree(tree_id, "manual").await
    }

    async fn restore_execution_tree(&self, tree_id: &str) -> Result<(), WorkflowError> {
        self.restore_execution_tree_locked(tree_id).await
    }
}

fn map_started_execution_tree_error(
    error: WorkflowError,
) -> crate::usecase::agent_session::StartedExecutionTreeRegistrationError {
    match error {
        WorkflowError::Store(kind) => {
            crate::usecase::agent_session::StartedExecutionTreeRegistrationError::Store(kind)
        }
        error @ (WorkflowError::Technical(_)
        | WorkflowError::External(_)
        | WorkflowError::Editor(_)) => {
            crate::usecase::agent_session::StartedExecutionTreeRegistrationError::Store(
                error.into(),
            )
        }
        WorkflowError::Validation(_)
        | WorkflowError::InvalidState(_)
        | WorkflowError::NotFound(_)
        | WorkflowError::UnauthorizedApprovalTarget(_)
        | WorkflowError::Conflict(_)
        | WorkflowError::CorruptStoredState(_)
        | WorkflowError::IncompatibleStoredEvent(_) => {
            crate::usecase::agent_session::StartedExecutionTreeRegistrationError::Corrupt
        }
    }
}

#[cfg(test)]
#[path = "runtime_command_test.rs"]
pub(crate) mod runtime_command_tests;

#[cfg(feature = "test-support")]
impl WorkflowRuntimeUsecase {
    pub fn test_replace_abort_execution(&mut self, abort_execution: WorkflowAbortExecutionUsecase) {
        self.abort_execution = abort_execution;
    }

    pub fn test_replace_worktree_operations(
        &mut self,
        operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
    ) {
        self.worktree_operations = operations;
    }

    pub fn test_worktree_operations(
        &self,
    ) -> &crate::usecase::worktree_operation::WorktreeOperations {
        &self.worktree_operations
    }
}
