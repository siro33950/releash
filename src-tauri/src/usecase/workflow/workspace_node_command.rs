use std::sync::Arc;

use crate::domain::workflow::WorkflowError;

use super::command::{ApprovalCommand, RetryNodeCommand};
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApproveWorkspaceNodeCommand {
    pub worktree_path: String,
    pub node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetryWorkspaceNodeCommand {
    pub worktree_path: String,
    pub node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceNodeApprovalTarget {
    pub execution_id: String,
    pub node_name: String,
    pub node_execution_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceNodeRetryTarget {
    pub execution_id: String,
    pub node_execution_id: String,
}

pub(crate) trait WorkspaceNodeActionResolver: Send + Sync {
    fn resolve_approval_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError>;

    fn resolve_retry_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeRetryTarget, WorkflowError>;
}

#[async_trait::async_trait]
pub(crate) trait WorkspaceNodeWorkflowCommandExecutor: Send + Sync {
    async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError>;
    async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError>;
}

pub(crate) struct WorkspaceNodeCommandUsecase {
    resolver: Arc<dyn WorkspaceNodeActionResolver>,
    workflows: Arc<dyn WorkspaceNodeWorkflowCommandExecutor>,
}

impl WorkspaceNodeCommandUsecase {
    pub(crate) fn new(
        resolver: Arc<dyn WorkspaceNodeActionResolver>,
        workflows: Arc<dyn WorkspaceNodeWorkflowCommandExecutor>,
    ) -> Self {
        Self {
            resolver,
            workflows,
        }
    }

    pub(crate) async fn approve_workspace_node(
        &self,
        command: ApproveWorkspaceNodeCommand,
    ) -> Result<(), WorkflowError> {
        let target = self
            .resolver
            .resolve_approval_target(&command.worktree_path, &command.node_id)?;
        self.workflows
            .approve_node(ApprovalCommand {
                execution_id: target.execution_id,
                node_name: target.node_name,
                node_execution_id: Some(target.node_execution_id),
                comment: None,
            })
            .await
    }

    pub(crate) async fn retry_workspace_node(
        &self,
        command: RetryWorkspaceNodeCommand,
    ) -> Result<(), WorkflowError> {
        let target = self
            .resolver
            .resolve_retry_target(&command.worktree_path, &command.node_id)?;
        self.workflows
            .retry_node(RetryNodeCommand {
                execution_id: target.execution_id,
                node_execution_id: target.node_execution_id,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    enum FakeActionResult<T> {
        Value(T),
        Unavailable,
    }

    struct FakeWorkspaceNodeActionResolver {
        approval: FakeActionResult<WorkspaceNodeApprovalTarget>,
        retry: FakeActionResult<WorkspaceNodeRetryTarget>,
        requests: Mutex<Vec<(String, String, String)>>,
    }

    impl FakeWorkspaceNodeActionResolver {
        fn approval(target: WorkspaceNodeApprovalTarget) -> Self {
            Self {
                approval: FakeActionResult::Value(target),
                retry: FakeActionResult::Unavailable,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn retry(target: WorkspaceNodeRetryTarget) -> Self {
            Self {
                approval: FakeActionResult::Unavailable,
                retry: FakeActionResult::Value(target),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl WorkspaceNodeActionResolver for FakeWorkspaceNodeActionResolver {
        fn resolve_approval_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError> {
            self.requests.lock().unwrap().push((
                "approval".to_string(),
                worktree_path.to_string(),
                node_id.to_string(),
            ));
            match &self.approval {
                FakeActionResult::Value(target) => Ok(target.clone()),
                FakeActionResult::Unavailable => {
                    Err(WorkflowError::invalid_state("approval unavailable"))
                }
            }
        }

        fn resolve_retry_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceNodeRetryTarget, WorkflowError> {
            self.requests.lock().unwrap().push((
                "retry".to_string(),
                worktree_path.to_string(),
                node_id.to_string(),
            ));
            match &self.retry {
                FakeActionResult::Value(target) => Ok(target.clone()),
                FakeActionResult::Unavailable => {
                    Err(WorkflowError::invalid_state("retry unavailable"))
                }
            }
        }
    }

    #[derive(Default)]
    struct FakeWorkspaceNodeWorkflowGateway {
        approvals: Mutex<Vec<ApprovalCommand>>,
        retries: Mutex<Vec<RetryNodeCommand>>,
    }

    #[async_trait::async_trait]
    impl WorkspaceNodeWorkflowCommandExecutor for FakeWorkspaceNodeWorkflowGateway {
        async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
            self.approvals.lock().unwrap().push(command);
            Ok(())
        }

        async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
            self.retries.lock().unwrap().push(command);
            Ok(())
        }
    }

    #[tokio::test]
    async fn approve_workspace_node_resolves_target_and_executes_one_workflow_command() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::approval(
            WorkspaceNodeApprovalTarget {
                execution_id: "execution-1".to_string(),
                node_name: "review".to_string(),
                node_execution_id: "node-execution-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let usecase = WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone());

        usecase
            .approve_workspace_node(ApproveWorkspaceNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "approval".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert_eq!(workflows.approvals.lock().unwrap().len(), 1);
        assert!(workflows.retries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn retry_workspace_node_resolves_target_and_executes_one_workflow_command() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::retry(
            WorkspaceNodeRetryTarget {
                execution_id: "execution-1".to_string(),
                node_execution_id: "node-execution-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let usecase = WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone());

        usecase
            .retry_workspace_node(RetryWorkspaceNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "retry".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert!(workflows.approvals.lock().unwrap().is_empty());
        assert_eq!(workflows.retries.lock().unwrap().len(), 1);
    }
}
