use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::WorkspaceNodeDetailDto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionNodeSelectionDto {
    pub agent_session_id: String,
    pub node_id: String,
}

/// The one backend-owned read contract shared by every client surface.
#[async_trait::async_trait]
pub(crate) trait WorkspaceQueryService: Send + Sync {
    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError>;

    async fn session_selection(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<SessionNodeSelectionDto>, WorkflowError>;

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
}
