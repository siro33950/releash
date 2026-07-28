use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::domain::workspace_tree::{WorkspaceIdentity, WorkspaceSessionListKind};
use crate::usecase::agent_session::session::SessionSummary;
use crate::usecase::workflow::{
    command::ApprovalCommand, WorkspaceNodeDetailDto, WorkspaceTreeSnapshotDto,
    WorkspaceWorkflowHistoryItemDto,
};

/// The one backend-owned read contract shared by every client surface.
pub(crate) trait WorkspaceQueryService: Send + Sync {
    fn workspace_tree(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError>;

    fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError>;

    fn session_node_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<String>, WorkflowError>;

    fn node_approval_command(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalRoute, WorkflowError>;

    fn node_close_session_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<WorkspaceNodeCloseRoute, WorkflowError>;

    fn session_summaries(
        &self,
        workspace_identity: &WorkspaceIdentity,
        list: WorkspaceSessionListKind,
    ) -> Result<Vec<SessionSummary>, WorkflowError>;

    fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError>;

    fn execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError>;

    fn workflow_history(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError>;
}

pub(crate) enum WorkspaceNodeApprovalRoute {
    Missing,
    NotWaiting,
    Command(ApprovalCommand),
}

pub(crate) enum WorkspaceNodeCloseRoute {
    Missing,
    NotSupported,
    Session(String),
}
