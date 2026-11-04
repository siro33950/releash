use crate::domain::workflow::{ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::WorkspaceNodeDetailDto;

/// The one backend-owned read contract shared by every client surface.
#[async_trait::async_trait]
pub trait WorkspaceQueryService: Send + Sync {
    /// 全 worktree の集約状態と、archive されていない実行木の一覧を返す。
    async fn execution_overview(
        &self,
    ) -> Result<super::execution_overview::ExecutionOverviewDto, WorkflowError>;

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
    pub failures: Vec<SessionNodeResolutionFailureDto>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionNodeResolutionFailureDto {
    pub reference_id: String,
    pub failure: crate::domain::failure::WorkFailure,
}
