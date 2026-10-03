use std::path::PathBuf;
use std::sync::Arc;

use super::error::RepositoryStateError;
use super::scanner::RepositoryScanner;
use super::snapshot::RepositorySnapshotParts;
use super::worker::InvalidateReason;
use crate::domain::repository::Worktree;

pub struct ScanWorker {
    pub state: Arc<super::worktree::WorktreeState>,
    pub scanner: Arc<dyn RepositoryScanner>,
    pub receiver: Box<dyn RepositoryStateInvalidationReceiver>,
}

pub trait RepositoryStateInvalidationSender: Send + Sync {
    fn send(&self, reason: InvalidateReason) -> Result<(), ()>;
}

#[async_trait::async_trait]
pub trait RepositoryStateInvalidationReceiver: Send {
    async fn recv(&mut self) -> Option<InvalidateReason>;
    fn try_recv(&mut self) -> Option<InvalidateReason>;
}

#[async_trait::async_trait]
pub trait RepositoryStateWorkerRuntime: Send + Sync {
    fn invalidation_channel(
        &self,
    ) -> (
        Box<dyn RepositoryStateInvalidationSender>,
        Box<dyn RepositoryStateInvalidationReceiver>,
    );

    async fn scan(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        repo_path: String,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError>;

    async fn scan_worktrees(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        repo_path: String,
    ) -> Result<Vec<Worktree>, RepositoryStateError>;
}

pub trait WorktreePathNormalizer: Send + Sync {
    fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError>;
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use tokio::sync::mpsc;

    pub(crate) struct TestRepositoryStateWorkerRuntime;

    struct TokioInvalidationSender(mpsc::UnboundedSender<InvalidateReason>);

    impl RepositoryStateInvalidationSender for TokioInvalidationSender {
        fn send(&self, reason: InvalidateReason) -> Result<(), ()> {
            self.0.send(reason).map_err(|_| ())
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
    pub(crate) struct NoSpawnRepositoryStateWorkerRuntime;

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

    pub(crate) struct IdentityWorktreePathNormalizer;

    impl WorktreePathNormalizer for IdentityWorktreePathNormalizer {
        fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
            Ok(PathBuf::from(worktree_path))
        }
    }

    pub(crate) struct CanonicalWorktreePathNormalizer;

    impl WorktreePathNormalizer for CanonicalWorktreePathNormalizer {
        fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
            let path = PathBuf::from(worktree_path);
            path.canonicalize().map_err(|err| {
                RepositoryStateError::Watcher(format!(
                    "Failed to canonicalize worktree path {}: {err}",
                    path.display()
                ))
            })
        }
    }
}
