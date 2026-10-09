use crate::domain::workflow::{ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::WorkspaceNodeDetailDto;

/// The one backend-owned read contract shared by every client surface.
#[async_trait::async_trait]
pub trait WorkspaceQueryService: Send + Sync {
    async fn worktree_executions(
        &self,
        workspace: &WorkspaceIdentity,
        failures: &dyn crate::domain::failure::FailureRecordRepository,
    ) -> Result<Vec<WorktreeExecutionSummary>, WorkflowError>;
    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError>;

    async fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError>;
}

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceNodeStatusClassification;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeExecutionSummary {
    pub id: String,
    pub title: String,
    pub is_workflow: bool,
    pub provider: Option<ProviderKind>,
    pub status: WorkspaceNodeStatusClassification,
    pub node_count: usize,
    pub session_states: Vec<WorkspaceNodeStatusClassification>,
}
