use std::sync::Arc;

use crate::domain::workflow::{ExecutionStatusFilter, WorkflowError, WorkflowExecutionSummary};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::workflow::WorkspaceNodeDetailDto;

use super::WorkspaceQueryService;

/// Explicit unit-test fake. Production composition never branches to this type.
pub struct TestWorkspaceQueryService {
    executions: Vec<WorkflowExecutionSummary>,
    pub session_nodes: parking_lot::Mutex<super::query_service::SessionNodeResolutionDto>,
    pub resolved_nodes: parking_lot::Mutex<Vec<Vec<String>>>,
}

impl TestWorkspaceQueryService {
    pub fn new(executions: Vec<WorkflowExecutionSummary>) -> Arc<Self> {
        Arc::new(Self {
            executions,
            session_nodes: Default::default(),
            resolved_nodes: Default::default(),
        })
    }
}

#[async_trait::async_trait]
impl WorkspaceQueryService for TestWorkspaceQueryService {
    async fn resolve_session_nodes(
        &self,
        _: &WorkspaceIdentity,
        ids: &[String],
    ) -> super::query_service::SessionNodeResolutionDto {
        self.resolved_nodes.lock().push(ids.to_vec());
        self.session_nodes.lock().clone()
    }

    async fn node_detail(
        &self,
        _workspace_identity: &WorkspaceIdentity,
        _node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        Ok(None)
    }

    async fn session_node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        _tree_id: &str,
        node_execution_id: &str,
    ) -> Result<Option<super::query_service::SessionNodeDetailDto>, WorkflowError> {
        self.node_detail(workspace_identity, node_execution_id)
            .await
            .map(|detail| {
                detail.map(|detail| super::query_service::SessionNodeDetailDto {
                    detail,
                    classification: Vec::new(),
                })
            })
    }

    async fn node_artifact(
        &self,
        _: &str,
    ) -> Result<crate::usecase::workflow::WorkflowGetOutputResult, WorkflowError> {
        Ok(crate::usecase::workflow::WorkflowGetOutputResult::NotSubmitted)
    }

    async fn execution_summaries(
        &self,
        _workspace_identity: Option<&WorkspaceIdentity>,
        _status: Option<ExecutionStatusFilter>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        Ok(self.executions.clone())
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
}
