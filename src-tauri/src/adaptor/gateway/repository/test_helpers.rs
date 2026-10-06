use crate::usecase::repository_state::worker::InvalidateReason;
use crate::usecase::repository_state::worktree::WorktreeState;
use crate::usecase::repository_state::RepositoryStateError;
use notify_debouncer_mini::DebouncedEvent;
use std::sync::Arc;

use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    RepositoryStateWorkerRuntime,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::test_helpers::EmptyScanner;
use notify_debouncer_mini::DebouncedEventKind;
pub struct InertSender;
impl RepositoryStateInvalidationSender for InertSender {
    fn send(&self, _reason: InvalidateReason) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}
pub struct InertReceiver;
#[async_trait::async_trait]
impl RepositoryStateInvalidationReceiver for InertReceiver {
    async fn recv(&mut self) -> Option<InvalidateReason> {
        None
    }

    fn try_recv(&mut self) -> Option<InvalidateReason> {
        None
    }
}
pub struct InertRuntime;
#[async_trait::async_trait]
impl RepositoryStateWorkerRuntime for InertRuntime {
    fn invalidation_channel(
        &self,
    ) -> (
        Box<dyn RepositoryStateInvalidationSender>,
        Box<dyn RepositoryStateInvalidationReceiver>,
    ) {
        (Box::new(InertSender), Box::new(InertReceiver))
    }

    async fn scan(
        &self,
        _scanner: Arc<dyn RepositoryScanner>,
        _repo_path: String,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        Err(RepositoryStateError::Watcher(
            "inert runtime does not scan".to_string(),
        ))
    }

    async fn scan_worktrees(
        &self,
        _scanner: Arc<dyn RepositoryScanner>,
        _repo_path: String,
    ) -> Result<Vec<crate::domain::repository::Worktree>, RepositoryStateError> {
        Err(RepositoryStateError::Watcher(
            "inert runtime does not scan".to_string(),
        ))
    }
}

pub fn event(path: &std::path::Path) -> DebouncedEvent {
    DebouncedEvent {
        path: path.to_path_buf(),
        kind: DebouncedEventKind::Any,
    }
}
pub fn state_with_subscriptions(
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
) -> Arc<WorktreeState> {
    state_at("/repo", true, subscriptions)
}
pub fn state_at(
    path: &str,
    is_repository_root: bool,
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
) -> Arc<WorktreeState> {
    WorktreeState::new(
        path.to_string(),
        is_repository_root,
        Arc::new(EmptyScanner),
        subscriptions,
        Arc::new(InertRuntime),
        tokio::sync::mpsc::unbounded_channel().0,
    )
}
