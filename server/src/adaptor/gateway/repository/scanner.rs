use std::sync::Arc;

use crate::domain::git::Worktree;
use crate::usecase::git_usecase::GitUsecase;
use crate::usecase::repository_dto::FileStatusDto;
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::RepositoryStateError;

pub struct DefaultRepositoryScanner {
    repository: Arc<GitUsecase>,
    repository_config: Arc<crate::usecase::repository_config_usecase::RepositoryConfigUsecase>,
}

impl DefaultRepositoryScanner {
    pub fn new(
        repository: Arc<GitUsecase>,
        repository_config: Arc<crate::usecase::repository_config_usecase::RepositoryConfigUsecase>,
    ) -> Self {
        Self {
            repository,
            repository_config,
        }
    }
}

#[async_trait::async_trait]

impl RepositoryScanner for DefaultRepositoryScanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        super::super::shared::background_worker::execute(
            &super::super::shared::background_worker::Request::RepositoryScan(repo_path.to_owned()),
        )
        .await
        .map_err(|error| RepositoryStateError::Background {
            kind: error.kind,
            message: error.message,
        })
    }

    fn scan(&self, repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        let status_scan = self.repository.get_repository_status_scan(repo_path)?;
        let dirty_count = status_scan.dirty_count;
        let status: Vec<FileStatusDto> = status_scan.status.into_iter().map(Into::into).collect();
        Ok(RepositorySnapshotParts {
            status,
            dirty_count,
        })
    }

    fn scan_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
        Ok(self.repository.list_working_worktrees(repo_path)?)
    }

    fn prune_stale_branch_bases(&self, repo_path: &str) -> Result<(), RepositoryStateError> {
        let existing_branches = self
            .repository
            .list_branches(repo_path)?
            .into_iter()
            .filter(|branch| !branch.is_remote)
            .map(|branch| branch.name)
            .collect::<Vec<_>>();
        Ok(self
            .repository_config
            .prune_stale_branch_bases(repo_path, &existing_branches)?)
    }
}
