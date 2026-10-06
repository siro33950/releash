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
    fn send(&self, reason: InvalidateReason) -> Result<(), RepositoryStateError>;
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

#[cfg(any(test, feature = "test-support"))]
#[path = "test_helpers_runtime.rs"]
pub(crate) mod test_helpers_runtime;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_helpers_runtime::tests_support;
