use crate::usecase::repository_state::worktree::RepositoryStateWatcher;

use crate::domain::repository::Worktree;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::error::RepositoryStateError;
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::service::RepositoryStateRepository;
use crate::usecase::repository_state::service::*;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::test_helpers::tests_support::{
    IdentityWorktreePathNormalizer, TestRepositoryStateWorkerRuntime,
};
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

pub(crate) mod tests_support {
    use super::*;
    use crate::usecase::repository_state::runtime::*;
    use crate::usecase::repository_state::worker::InvalidateReason;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    pub struct TestRepositoryStateWorkerRuntime;

    struct TokioInvalidationSender(mpsc::UnboundedSender<InvalidateReason>);

    impl RepositoryStateInvalidationSender for TokioInvalidationSender {
        fn send(&self, reason: InvalidateReason) -> Result<(), RepositoryStateError> {
            self.0.send(reason).map_err(|_| {
                RepositoryStateError::Watcher("repository snapshot worker is stopped".into())
            })
        }
    }

    struct TokioInvalidationReceiver(mpsc::UnboundedReceiver<InvalidateReason>);

    #[async_trait::async_trait]
    impl RepositoryStateInvalidationReceiver for TokioInvalidationReceiver {
        async fn recv(&mut self) -> Option<InvalidateReason> {
            self.0.recv().await
        }

        fn try_recv(&mut self) -> Option<InvalidateReason> {
            self.0.try_recv().ok()
        }
    }

    #[async_trait::async_trait]
    impl RepositoryStateWorkerRuntime for TestRepositoryStateWorkerRuntime {
        fn invalidation_channel(
            &self,
        ) -> (
            Box<dyn RepositoryStateInvalidationSender>,
            Box<dyn RepositoryStateInvalidationReceiver>,
        ) {
            let (tx, rx) = mpsc::unbounded_channel();
            (
                Box::new(TokioInvalidationSender(tx)),
                Box::new(TokioInvalidationReceiver(rx)),
            )
        }

        async fn scan(
            &self,
            scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            crate::common::operation_context::spawn_blocking(move || scanner.scan(&repo_path))
                .await
                .map_err(|err| RepositoryStateError::Watcher(format!("test scan failed: {err}")))?
        }

        async fn scan_worktrees(
            &self,
            scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<Vec<Worktree>, RepositoryStateError> {
            crate::common::operation_context::spawn_blocking(move || {
                scanner.scan_worktrees(&repo_path)
            })
            .await
            .map_err(|err| RepositoryStateError::Watcher(format!("test scan failed: {err}")))?
        }
    }

    /// tokio ランタイム外（std スレッド）から `WorktreeState::new` を呼ぶテスト用。
    /// worker を spawn しないため snapshot は更新されない。
    #[cfg(test)]
    pub(crate) struct NoSpawnRepositoryStateWorkerRuntime;

    #[cfg(test)]
    #[async_trait::async_trait]
    impl RepositoryStateWorkerRuntime for NoSpawnRepositoryStateWorkerRuntime {
        fn invalidation_channel(
            &self,
        ) -> (
            Box<dyn RepositoryStateInvalidationSender>,
            Box<dyn RepositoryStateInvalidationReceiver>,
        ) {
            let (tx, rx) = mpsc::unbounded_channel();
            (
                Box::new(TokioInvalidationSender(tx)),
                Box::new(TokioInvalidationReceiver(rx)),
            )
        }

        async fn scan(
            &self,
            _scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            Err(RepositoryStateError::Watcher(format!(
                "no-spawn runtime does not scan {repo_path}"
            )))
        }

        async fn scan_worktrees(
            &self,
            _scanner: Arc<dyn RepositoryScanner>,
            repo_path: String,
        ) -> Result<Vec<Worktree>, RepositoryStateError> {
            Err(RepositoryStateError::Watcher(format!(
                "no-spawn runtime does not scan {repo_path}"
            )))
        }
    }

    pub struct IdentityWorktreePathNormalizer;

    impl WorktreePathNormalizer for IdentityWorktreePathNormalizer {
        fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
            Ok(PathBuf::from(worktree_path))
        }
    }
}
