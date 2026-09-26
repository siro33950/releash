use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::{
    WorkspaceNodeDetailDto, WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto,
};

/// The one backend-owned read contract shared by every client surface.
#[async_trait::async_trait]
pub(crate) trait WorkspaceQueryService: Send + Sync {
    async fn failure_targets(&self, node_id: &str) -> Result<Vec<String>, WorkflowError>;

    async fn workspace_tree(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError>;

    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError>;

    async fn session_node_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<String>, WorkflowError>;

    async fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError>;

    async fn execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError>;

    async fn workflow_history(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError>;
}
