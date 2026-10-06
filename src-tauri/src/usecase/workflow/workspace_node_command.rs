use std::sync::Arc;

use crate::domain::workflow::WorkflowError;
use crate::usecase::agent_session::{AgentSessionRenameError, AgentSessionRenameExecutor};

use super::command::{ApprovalCommand, ResumeSessionNodeCommand, RetryNodeCommand};
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
pub(crate) struct ResumeWorkspaceSessionNodeCommand {
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

#[async_trait::async_trait]
pub(crate) trait WorkspaceNodeActionResolver: Send + Sync {
    async fn resolve_approval_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError>;

    async fn resolve_retry_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeRetryTarget, WorkflowError>;

    async fn resolve_session_resume_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<ResumeSessionNodeCommand, WorkflowError>;

    async fn resolve_session_rename_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError>;
}

#[async_trait::async_trait]
pub(crate) trait WorkspaceNodeWorkflowCommandExecutor: Send + Sync {
    async fn approve_node(&self, command: ApprovalCommand) -> Result<(), WorkflowError>;
    async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError>;
    async fn resume_session_node(
        &self,
        command: ResumeSessionNodeCommand,
    ) -> Result<(), WorkflowError>;
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
            .resolve_approval_target(&command.worktree_path, &command.node_id)
            .await?;
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
            .resolve_retry_target(&command.worktree_path, &command.node_id)
            .await?;
        self.workflows
            .retry_node(RetryNodeCommand {
                execution_id: target.execution_id,
                node_execution_id: target.node_execution_id,
            })
            .await
    }

    pub(crate) async fn resume_workspace_session_node(
        &self,
        command: ResumeWorkspaceSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        let target = self
            .resolver
            .resolve_session_resume_target(&command.worktree_path, &command.node_id)
            .await?;
        self.workflows.resume_session_node(target).await
    }

    pub(crate) async fn rename_workspace_session_node(
        &self,
        command: RenameWorkspaceSessionNodeCommand,
    ) -> Result<(), WorkflowError> {
        let target = self
            .resolver
            .resolve_session_rename_target(&command.worktree_path, &command.node_id)
            .await?;
        self.session_renames
            .rename(&target.agent_session_id, &command.name)
            .await
            .map(|_| ())
            .map_err(map_session_rename_error)
    }
}

fn map_session_rename_error(error: AgentSessionRenameError) -> WorkflowError {
    match error {
        AgentSessionRenameError::Store(kind) => WorkflowError::Store(kind),
        AgentSessionRenameError::NotFound => {
            WorkflowError::NotFound("AgentSession for Workspace Node was not found".to_string())
        }
        AgentSessionRenameError::InvalidOperation => {
            WorkflowError::validation("Session Node name must not be empty")
        }
        AgentSessionRenameError::Conflict => {
            WorkflowError::Conflict("AgentSession rename conflicted".to_string())
        }
        AgentSessionRenameError::ProviderSessionAlreadyOwned => WorkflowError::InvalidState(
            "Provider session is already owned by another AgentSession".to_string(),
        ),
        AgentSessionRenameError::Unavailable => WorkflowError::Store(
            crate::domain::failure::TechnicalFailure {
                message: "AgentSession rename storage is unavailable".to_string(),
                nature: crate::domain::failure::TechnicalFailureNature::Transient,
            }
            .into(),
        ),
        AgentSessionRenameError::Corrupt => {
            WorkflowError::CorruptStoredState("AgentSession rename state is corrupt".to_string())
        }
    }
}

#[cfg(test)]
#[path = "workspace_node_command_test.rs"]
mod workspace_node_command_tests;
