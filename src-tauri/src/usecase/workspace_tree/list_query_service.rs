use std::sync::Arc;

use crate::domain::git_host::PrStatus;
use crate::usecase::{
    git_host::GitHostUsecase,
    repo_paths_usecase::RepoPathsUsecase,
    repository_dto::BranchCardDto,
    repository_state::RepositoryStateService,
    workflow::{WorkflowUsecase, WorkspaceTreeSnapshotDto, WorkspaceWorkflowHistoryItemDto},
};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct WorkspaceListUsecaseError(pub String);

#[async_trait::async_trait]
pub(crate) trait WorkspaceListQueryService: Send + Sync {
    fn repositories(&self) -> Result<Vec<String>, WorkspaceListUsecaseError>;
    async fn branches(&self, path: &str) -> Result<Vec<BranchCardDto>, WorkspaceListUsecaseError>;
    async fn current_branches(
        &self,
        path: &str,
    ) -> Result<Vec<BranchCardDto>, WorkspaceListUsecaseError> {
        self.branches(path).await
    }
    fn pr_status(&self, path: &str, force: bool) -> Result<PrStatus, WorkspaceListUsecaseError>;
    async fn nodes(
        &self,
        path: &str,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkspaceListUsecaseError>;
    async fn history(
        &self,
        path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkspaceListUsecaseError>;
}

pub(crate) struct WorkspaceListServices {
    pub repositories: Arc<RepoPathsUsecase>,
    pub repository_state: Arc<RepositoryStateService>,
    pub workflow: Arc<WorkflowUsecase>,
    pub git_host: Arc<GitHostUsecase>,
}

#[async_trait::async_trait]
impl WorkspaceListQueryService for WorkspaceListServices {
    fn repositories(&self) -> Result<Vec<String>, WorkspaceListUsecaseError> {
        Ok(self.repositories.get())
    }

    async fn branches(&self, path: &str) -> Result<Vec<BranchCardDto>, WorkspaceListUsecaseError> {
        self.repository_state
            .rescan_branches(path)
            .await
            .map_err(|error| WorkspaceListUsecaseError(error.to_string()))
    }

    async fn current_branches(
        &self,
        path: &str,
    ) -> Result<Vec<BranchCardDto>, WorkspaceListUsecaseError> {
        self.repository_state
            .list_branches_with_status_snapshot(path)
            .map(|snapshot| snapshot.branches)
            .map_err(|error| WorkspaceListUsecaseError(error.to_string()))
    }

    fn pr_status(&self, path: &str, force: bool) -> Result<PrStatus, WorkspaceListUsecaseError> {
        let result = if force {
            self.git_host.fetch_pr_status(path)
        } else {
            self.git_host.get_cached_pr_status(path)
        };
        result.map_err(|error| WorkspaceListUsecaseError(error.to_string()))
    }

    async fn nodes(
        &self,
        path: &str,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkspaceListUsecaseError> {
        self.workflow
            .list_workspace_tree_nodes(path)
            .await
            .map_err(|error| WorkspaceListUsecaseError(error.to_string()))
    }

    async fn history(
        &self,
        path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkspaceListUsecaseError> {
        self.workflow
            .list_workspace_workflow_history(path)
            .await
            .map_err(|error| WorkspaceListUsecaseError(error.to_string()))
    }
}
