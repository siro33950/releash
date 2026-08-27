use std::sync::Arc;

use crate::domain::workflow::{WorkflowError, WorkflowExecutionId};
use crate::usecase::workflow::control_plane::{
    WorkflowControlPlaneGateway, WorkflowControlPlaneUsecase,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryNodeCommand {
    pub execution_id: String,
    pub node_execution_id: String,
}

#[derive(Clone)]
pub(crate) struct WorkflowRetryNodeUsecase {
    control_plane: WorkflowControlPlaneUsecase,
}

impl WorkflowRetryNodeUsecase {
    pub(crate) fn new(runtime: Arc<dyn WorkflowControlPlaneGateway>) -> Self {
        Self {
            control_plane: WorkflowControlPlaneUsecase::new(runtime),
        }
    }

    pub(crate) async fn execute(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone())?;
        if command.node_execution_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "node_execution_id must not be empty",
            ));
        }
        self.control_plane.retry_node(command).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::WorkflowExecution;
    use crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit;
    use crate::usecase::workflow::runtime_driver::NodeOutcome;
    use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

    struct RecoveryFencedRuntime;

    #[async_trait::async_trait]
    impl WorkflowControlPlaneGateway for RecoveryFencedRuntime {
        fn current_timestamp(&self) -> f64 {
            unreachable!()
        }

        fn new_node_execution_id(&self) -> String {
            unreachable!()
        }

        fn ensure_node_recovery_available(
            &self,
            _execution_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowError> {
            Err(WorkflowError::invalid_state(
                "isolated worktree is missing: /repo-worktrees/.releash-isolated/node-1-a1",
            ))
        }

        async fn resolve_workflow_execution_id(
            &self,
            _node_execution_id: &str,
        ) -> Result<Option<String>, WorkflowError> {
            unreachable!()
        }

        async fn load_active_execution(
            &self,
            _execution_id: &str,
        ) -> Result<Option<WorkflowExecution>, WorkflowError> {
            unreachable!()
        }

        async fn recover_active_executions(&self) -> Result<(), WorkflowError> {
            unreachable!()
        }

        async fn register_started_execution_tree(
            &self,
            _tree_id: &str,
        ) -> Result<(), WorkflowError> {
            unreachable!()
        }

        async fn approval_persisted(
            &self,
            _execution_id: &str,
            _node_name: &str,
            _node_execution_id: Option<&str>,
        ) -> Result<bool, WorkflowError> {
            unreachable!()
        }

        fn configured_secret_values(&self) -> Vec<String> {
            Vec::new()
        }

        async fn commit_control_plane(
            &self,
            _commit: WorkflowControlPlaneCommit,
        ) -> Result<RuntimeCommitSnapshot, WorkflowError> {
            unreachable!()
        }

        async fn finish_control_plane_commit(
            &self,
            _worktree_path: &str,
            _snapshot: &RuntimeCommitSnapshot,
            _outcome: Option<NodeOutcome>,
        ) -> Result<(), WorkflowError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn isolated_worktree_loss_rejects_retry_before_starting_a_new_attempt() {
        let usecase = WorkflowRetryNodeUsecase::new(Arc::new(RecoveryFencedRuntime));

        let error = usecase
            .execute(RetryNodeCommand {
                execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
                node_execution_id: "node-1".to_string(),
            })
            .await
            .unwrap_err();

        assert_eq!(
            error,
            WorkflowError::InvalidState(
                "isolated worktree is missing: /repo-worktrees/.releash-isolated/node-1-a1"
                    .to_string()
            )
        );
    }
}
