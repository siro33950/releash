use std::sync::Arc;

use crate::domain::repository::Worktree;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::scanner::{
    changes_diff_tree_entries, diff_tree_entries, staged_diff_tree_entries, RepositoryScanner,
};
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::RepositoryStateError;
use crate::usecase::repository_usecase::RepositoryUsecase;

pub struct DefaultRepositoryScanner {
    repository: Arc<RepositoryUsecase>,
    code: Arc<CodeUsecase>,
}

impl DefaultRepositoryScanner {
    pub fn new(repository: Arc<RepositoryUsecase>, code: Arc<CodeUsecase>) -> Self {
        Self { repository, code }
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
        let diff_stats: Vec<FileDiffStatDto> =
            status_scan.diff_stats.into_iter().map(Into::into).collect();
        let diff_file_tree = self
            .code
            .build_diff_file_tree(diff_tree_entries(&status, &diff_stats));
        let staged_diff_file_tree = self
            .code
            .build_diff_file_tree(staged_diff_tree_entries(&status, &diff_stats));
        let changes_diff_file_tree = self
            .code
            .build_diff_file_tree(changes_diff_tree_entries(&status, &diff_stats));

        Ok(RepositorySnapshotParts {
            status,
            diff_stats,
            dirty_count,
            diff_file_tree,
            staged_diff_file_tree,
            changes_diff_file_tree,
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
            .repository
            .prune_stale_branch_bases(repo_path, &existing_branches)?)
    }
}
