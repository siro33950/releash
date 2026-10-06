use std::sync::Arc;

use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary, WorkflowPageRequest,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::WorkspaceNodeDetailDto;

use super::WorkspaceQueryService;

/// Explicit unit-test fake. Production composition never branches to this type.
pub struct TestWorkspaceQueryService {
    executions: Vec<WorkflowExecutionSummary>,
}

impl TestWorkspaceQueryService {
    pub fn new(executions: Vec<WorkflowExecutionSummary>) -> Arc<Self> {
        Arc::new(Self { executions })
    }
}

#[async_trait::async_trait]
impl WorkspaceQueryService for TestWorkspaceQueryService {
    async fn node_detail(
        &self,
        _workspace_identity: &WorkspaceIdentity,
        _node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        Ok(None)
    }

    async fn execution_summaries(
        &self,
        _workspace_identity: Option<&WorkspaceIdentity>,
        _status: Option<ExecutionStatusFilter>,
        _page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        Ok(self.executions.clone())
    }

    async fn execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        Ok(self
            .executions
            .iter()
            .find(|execution| execution.execution_id == execution_id)
            .cloned())
    }
}

pub struct TestWorkspaceTreeRepository;

impl TestWorkspaceTreeRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

#[async_trait::async_trait]
impl crate::domain::workspace_tree::WorkspaceTreeRepository for TestWorkspaceTreeRepository {
    async fn load_trees(
        &self,
        workspaces: &[WorkspaceIdentity],
    ) -> Vec<Result<crate::domain::workspace_tree::WorkspaceTree, WorkflowError>> {
        workspaces
            .iter()
            .map(|workspace| {
                Ok(crate::domain::workspace_tree::WorkspaceTree::empty(
                    workspace.as_str(),
                ))
            })
            .collect()
    }
    async fn load_node_by_session_id(
        &self,
        _: &crate::domain::workspace_tree::WorkspaceIdentity,
        _: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        Ok(None)
    }

    async fn load_node(
        &self,
        _: &WorkspaceIdentity,
        _: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        Ok(None)
    }
    async fn load_node_by_node_execution_id(
        &self,
        _: &str,
    ) -> Result<
        Option<crate::domain::workspace_tree::WorkspaceTreeNode>,
        crate::domain::local_event::LocalEventQueryError,
    > {
        Ok(None)
    }
}
