use crate::domain::workflow::{ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::WorkspaceNodeDetailDto;

/// The one backend-owned read contract shared by every client surface.
#[async_trait::async_trait]
pub trait WorkspaceQueryService: Send + Sync {
    async fn resolve_session_nodes(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_ids: &[String],
    ) -> SessionNodeResolutionDto;

    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError>;

    async fn session_node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        tree_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<SessionNodeDetailDto>, WorkflowError>;

    async fn node_artifact(
        &self,
        node_execution_id: &str,
    ) -> Result<crate::usecase::workflow::WorkflowGetOutputResult, WorkflowError>;

    async fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError>;
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionNodeResolutionDto {
    pub sessions: std::collections::HashMap<String, String>,
    pub failures: Vec<crate::domain::failure::WorkFailure>,
}

pub struct SessionNodeDetailDto {
    pub detail: WorkspaceNodeDetailDto,
    pub(crate) classification: Vec<crate::domain::workspace_tree::WorkspaceNodeClassification>,
}
