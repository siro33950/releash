use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::{AgentSessionRenameError, AgentSessionRenameExecutor};

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
pub(crate) struct RenameWorkspaceSessionNodeCommand {
    pub worktree_path: String,
    pub node_id: String,
    pub name: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceSessionNodeRenameTarget {
    pub agent_session_id: String,
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

    fn resolve_session_rename_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError>;
}

#[async_trait::async_trait]
pub(crate) trait WorkspaceNodeWorkflowCommandExecutor: Send + Sync {
    async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError>;
    async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError>;
}

pub(crate) struct WorkspaceNodeCommandUsecase {
    resolver: Arc<dyn WorkspaceNodeActionResolver>,
    workflows: Arc<dyn WorkspaceNodeWorkflowCommandExecutor>,
    session_renames: Arc<dyn AgentSessionRenameExecutor>,
}

impl WorkspaceNodeCommandUsecase {
    pub(crate) fn new(
        resolver: Arc<dyn WorkspaceNodeActionResolver>,
        workflows: Arc<dyn WorkspaceNodeWorkflowCommandExecutor>,
        session_renames: Arc<dyn AgentSessionRenameExecutor>,
    ) -> Self {
        Self {
            resolver,
            workflows,
            session_renames,
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

    pub(crate) async fn rename_workspace_session_node(
        &self,
        command: RenameWorkspaceSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        let target = self
            .resolver
            .resolve_session_rename_target(&command.worktree_path, &command.node_id)?;
        self.session_renames
            .rename(&target.agent_session_id, &command.name)
            .await
            .map(|_| ())
            .map_err(map_session_rename_error)
    }
}

fn map_session_rename_error(error: AgentSessionRenameError) -> WorkflowError {
    match error {
        AgentSessionRenameError::NotFound => {
            WorkflowError::NotFound("AgentSession for Workspace Node was not found".to_string())
        }
        AgentSessionRenameError::InvalidOperation => {
            WorkflowError::validation("Session Node name must not be empty")
        }
        AgentSessionRenameError::Conflict => {
            WorkflowError::Conflict("AgentSession rename conflicted".to_string())
        }
        AgentSessionRenameError::Unavailable => WorkflowError::StorageUnavailable {
            message: "AgentSession rename storage is unavailable".to_string(),
            retryable: true,
        },
        AgentSessionRenameError::Corrupt => {
            WorkflowError::CorruptStoredState("AgentSession rename state is corrupt".to_string())
        }
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
        rename: FakeActionResult<WorkspaceSessionNodeRenameTarget>,
        requests: Mutex<Vec<(String, String, String)>>,
    }

    impl FakeWorkspaceNodeActionResolver {
        fn approval(target: WorkspaceNodeApprovalTarget) -> Self {
            Self {
                approval: FakeActionResult::Value(target),
                retry: FakeActionResult::Unavailable,
                rename: FakeActionResult::Unavailable,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn retry(target: WorkspaceNodeRetryTarget) -> Self {
            Self {
                approval: FakeActionResult::Unavailable,
                retry: FakeActionResult::Value(target),
                rename: FakeActionResult::Unavailable,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn rename(target: WorkspaceSessionNodeRenameTarget) -> Self {
            Self {
                approval: FakeActionResult::Unavailable,
                retry: FakeActionResult::Unavailable,
                rename: FakeActionResult::Value(target),
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

        fn resolve_session_rename_target(
            &self,
            worktree_path: &str,
            node_id: &str,
        ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError> {
            self.requests.lock().unwrap().push((
                "rename".to_string(),
                worktree_path.to_string(),
                node_id.to_string(),
            ));
            match &self.rename {
                FakeActionResult::Value(target) => Ok(target.clone()),
                FakeActionResult::Unavailable => {
                    Err(WorkflowError::invalid_state("rename unavailable"))
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

    #[derive(Default)]
    struct FakeAgentSessionRenameExecutor {
        requests: Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl AgentSessionRenameExecutor for FakeAgentSessionRenameExecutor {
        async fn rename(
            &self,
            agent_session_id: &str,
            name: &str,
        ) -> Result<
            crate::domain::agent_session::aggregates::AgentSessionMutationOutcome,
            AgentSessionRenameError,
        > {
            self.requests
                .lock()
                .unwrap()
                .push((agent_session_id.to_string(), name.to_string()));
            Ok(crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied)
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
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames);

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
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames);

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

    #[tokio::test]
    async fn rename_workspace_session_node_resolves_target_and_delegates_name() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::rename(
            WorkspaceSessionNodeRenameTarget {
                agent_session_id: "agent-session-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase =
            WorkspaceNodeCommandUsecase::new(resolver.clone(), workflows.clone(), renames.clone());

        usecase
            .rename_workspace_session_node(RenameWorkspaceSessionNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "opaque-node-id".to_string(),
                name: "release review".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            *resolver.requests.lock().unwrap(),
            vec![(
                "rename".to_string(),
                "/repo".to_string(),
                "opaque-node-id".to_string()
            )]
        );
        assert_eq!(
            *renames.requests.lock().unwrap(),
            vec![("agent-session-1".to_string(), "release review".to_string())]
        );
        assert!(workflows.approvals.lock().unwrap().is_empty());
        assert!(workflows.retries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rename_workspace_session_node_rejects_unresolvable_node_without_delegation() {
        let resolver = Arc::new(FakeWorkspaceNodeActionResolver::approval(
            WorkspaceNodeApprovalTarget {
                execution_id: "execution-1".to_string(),
                node_name: "sequence".to_string(),
                node_execution_id: "sequence-1".to_string(),
            },
        ));
        let workflows = Arc::new(FakeWorkspaceNodeWorkflowGateway::default());
        let renames = Arc::new(FakeAgentSessionRenameExecutor::default());
        let usecase = WorkspaceNodeCommandUsecase::new(resolver, workflows, renames.clone());

        let result = usecase
            .rename_workspace_session_node(RenameWorkspaceSessionNodeCommand {
                worktree_path: "/repo".to_string(),
                node_id: "non-renameable-node".to_string(),
                name: "release review".to_string(),
            })
            .await;

        assert!(matches!(result, Err(WorkflowError::InvalidState(_))));
        assert!(renames.requests.lock().unwrap().is_empty());
    }
}
