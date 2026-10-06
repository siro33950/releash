use super::*;
use crate::domain::repository::Worktree;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::error::RepositoryStateError;
use crate::usecase::repository_state::runtime::test_helpers_runtime::tests_support::{
    IdentityWorktreePathNormalizer, TestRepositoryStateWorkerRuntime,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::service::RepositoryStateRepository;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct TestRepositoryStateRepository;

impl RepositoryStateRepository for TestRepositoryStateRepository {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        Ok(path.to_string())
    }
}

/// `/repo-worktrees/` 以下を `/repo` の linked worktree として解決し、解決の回数を数える。
#[derive(Default)]
#[cfg(test)]
pub struct LinkedRepositoryStateRepository {
    pub resolutions: AtomicUsize,
}

#[cfg(test)]
impl RepositoryStateRepository for LinkedRepositoryStateRepository {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        self.resolutions.fetch_add(1, Ordering::SeqCst);
        Ok(if path.starts_with("/repo-worktrees/") {
            "/repo".to_string()
        } else {
            path.to_string()
        })
    }
}

#[cfg(test)]
pub fn worktree(path: &str, branch: &str, is_main: bool) -> Worktree {
    Worktree {
        name: branch.to_string(),
        path: path.to_string(),
        branch: branch.to_string(),
        is_main,
        is_locked: false,
        is_merged: false,
    }
}

#[derive(Default)]
pub struct CountingScanner {
    pub scans: AtomicUsize,
    pub prunes: parking_lot::Mutex<Vec<String>>,
    pub status: parking_lot::Mutex<Vec<FileStatusDto>>,
    pub diff_stats: parking_lot::Mutex<Vec<FileDiffStatDto>>,
    pub worktrees: parking_lot::Mutex<Vec<Worktree>>,
}

impl CountingScanner {
    pub fn with_status(status: Vec<FileStatusDto>) -> Self {
        Self {
            status: parking_lot::Mutex::new(status),
            ..Self::default()
        }
    }

    pub fn scan_count(&self) -> usize {
        self.scans.load(Ordering::SeqCst)
    }

    pub fn prune_calls(&self) -> Vec<String> {
        self.prunes.lock().clone()
    }

    pub fn set_worktrees(&self, worktrees: Vec<Worktree>) {
        *self.worktrees.lock() = worktrees;
    }
}

#[async_trait::async_trait]
impl RepositoryScanner for CountingScanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scan(repo_path)
    }

    fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        let status = self.status.lock().clone();
        Ok(RepositorySnapshotParts {
            dirty_count: status.len(),
            status,
            diff_stats: self.diff_stats.lock().clone(),
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn scan_worktrees(&self, _repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
        Ok(self.worktrees.lock().clone())
    }

    fn prune_stale_branch_bases(&self, repo_path: &str) -> Result<(), RepositoryStateError> {
        self.prunes.lock().push(repo_path.to_string());
        Ok(())
    }
}

pub fn counting_service(
    scanner: Arc<CountingScanner>,
    watcher: Arc<dyn RepositoryStateWatcher>,
) -> RepositoryStateService {
    RepositoryStateService::new(
        Arc::new(TestRepositoryStateRepository),
        scanner,
        crate::test_support::state_subscription::test_subscriptions(),
        watcher,
        Arc::new(TestRepositoryStateWorkerRuntime),
        Arc::new(IdentityWorktreePathNormalizer),
        crate::test_support::state_subscription::repository_driver(),
    )
}

pub struct EmptyScanner;

#[async_trait::async_trait]

impl RepositoryScanner for EmptyScanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scan(repo_path)
    }

    fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        Ok(RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn scan_worktrees(&self, _repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
        Ok(Vec::new())
    }

    fn prune_stale_branch_bases(&self, _repo_path: &str) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}
