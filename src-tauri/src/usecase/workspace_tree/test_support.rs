use std::sync::{Arc, RwLock};

use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::domain::workspace_tree::{WorkspaceIdentity, WorkspaceSessionListKind};
use crate::usecase::agent_session::session::SessionSummary;
use crate::usecase::workflow::{
    WorkspaceNodeDetailDto, WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto,
};

use super::WorkspaceQueryService;

/// Explicit unit-test fake. Production composition never branches to this type.
pub(crate) struct TestWorkspaceQueryService {
    active_sessions: RwLock<Vec<SessionSummary>>,
    closed_sessions: RwLock<Vec<SessionSummary>>,
    executions: Vec<WorkflowExecutionSummary>,
}

impl TestWorkspaceQueryService {
    pub(crate) fn new(
        active_sessions: Vec<SessionSummary>,
        closed_sessions: Vec<SessionSummary>,
        executions: Vec<WorkflowExecutionSummary>,
    ) -> Arc<Self> {
        Arc::new(Self {
            active_sessions: RwLock::new(active_sessions),
            closed_sessions: RwLock::new(closed_sessions),
            executions,
        })
    }

    pub(crate) fn replace_session_summaries(
        &self,
        active_sessions: Vec<SessionSummary>,
        closed_sessions: Vec<SessionSummary>,
    ) {
        *self.active_sessions.write().unwrap() = active_sessions;
        *self.closed_sessions.write().unwrap() = closed_sessions;
    }
}

impl WorkspaceQueryService for TestWorkspaceQueryService {
    fn workspace_tree(
        &self,
        _workspace_identity: &WorkspaceIdentity,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError> {
        Ok(WorkspaceTreeSnapshotDto {
            nodes: Vec::new(),
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

    fn session_summaries(
        &self,
        _workspace_identity: &WorkspaceIdentity,
        list: WorkspaceSessionListKind,
    ) -> Result<Vec<SessionSummary>, WorkflowError> {
        Ok(match list {
            WorkspaceSessionListKind::Active => self.active_sessions.read().unwrap().clone(),
            WorkspaceSessionListKind::Closed => self.closed_sessions.read().unwrap().clone(),
            WorkspaceSessionListKind::Archived => Vec::new(),
        })
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
