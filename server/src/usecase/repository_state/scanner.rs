use crate::domain::git::Worktree;

use super::error::RepositoryStateError;
use super::snapshot::RepositorySnapshotParts;

#[async_trait::async_trait]
pub trait RepositoryScanner: Send + Sync {
    /// worktree の変更の状態を読む。
    fn scan(&self, repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    /// Repository の worktree の並びを読む。
    fn scan_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError>;

    /// 現存しないブランチの base 設定を掃除する。
    fn prune_stale_branch_bases(&self, repo_path: &str) -> Result<(), RepositoryStateError>;
}
