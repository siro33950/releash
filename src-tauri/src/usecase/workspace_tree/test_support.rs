use std::sync::Arc;

use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::{
    WorkspaceNodeDetailDto, WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto,
};

use super::WorkspaceQueryService;

/// Explicit unit-test fake. Production composition never branches to this type.
pub(crate) struct TestWorkspaceQueryService {
    executions: Vec<WorkflowExecutionSummary>,
}

impl TestWorkspaceQueryService {
    pub(crate) fn new(executions: Vec<WorkflowExecutionSummary>) -> Arc<Self> {
        Arc::new(Self { executions })
    }
}

impl WorkspaceQueryService for TestWorkspaceQueryService {
    fn workspace_tree(
        &self,
        _workspace_identity: &WorkspaceIdentity,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError> {
        Ok(WorkspaceTreeSnapshotDto {
            nodes: Vec::new(),
            sessions: Vec::new(),
            preferred_node_id: None,
        })
    }

    fn node_detail(
        &self,
        _workspace_identity: &WorkspaceIdentity,
        _node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        Ok(None)
    }

    fn session_node_id(
        &self,
        _workspace_identity: &WorkspaceIdentity,
        _session_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        Ok(None)
    }

    fn execution_summaries(
        &self,
        _workspace_identity: Option<&WorkspaceIdentity>,
        _status: Option<ExecutionStatusFilter>,
        _page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        Ok(self.executions.clone())
    }

    fn execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        Ok(self
            .executions
            .iter()
            .find(|execution| execution.execution_id == execution_id)
            .cloned())
    }

    fn workflow_history(
        &self,
        _workspace_identity: &WorkspaceIdentity,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        Ok(Vec::new())
    }
}
