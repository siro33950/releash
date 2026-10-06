use super::*;
use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    RepositoryStateWorkerRuntime,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use notify_debouncer_mini::DebouncedEventKind;

struct InertSender;

impl RepositoryStateInvalidationSender for InertSender {
    fn send(&self, _reason: InvalidateReason) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}

struct InertReceiver;

#[async_trait::async_trait]
impl RepositoryStateInvalidationReceiver for InertReceiver {
    async fn recv(&mut self) -> Option<InvalidateReason> {
        None
    }

    fn try_recv(&mut self) -> Option<InvalidateReason> {
        None
    }
}

struct InertRuntime;

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

struct EmptyScanner;

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

    fn scan_worktrees(
        &self,
        _repo_path: &str,
    ) -> Result<Vec<crate::domain::repository::Worktree>, RepositoryStateError> {
        Ok(Vec::new())
    }

    fn prune_stale_branch_bases(&self, _repo_path: &str) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}

fn event(path: &std::path::Path) -> DebouncedEvent {
    DebouncedEvent {
        path: path.to_path_buf(),
        kind: DebouncedEventKind::Any,
    }
}

fn state_with_subscriptions(
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
) -> Arc<WorktreeState> {
    state_at("/repo", true, subscriptions)
}

fn state_at(
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

#[test]
fn test_監視失敗_対象の読取を失敗にして購読へ通知する() {
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let state = state_with_subscriptions(subscriptions);
    handle_watch_failure(&state, "watch unavailable".into());
    assert_eq!(
        state.dirty_count().error.unwrap().message,
        "Watcher(\"watch unavailable\")"
    );
    assert!(matches!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Repository(_)
    ));
}

#[test]
fn test_ファイル監視_gitディレクトリの中の変化では変更の状態を読み直さない() {
    // Given
    let state =
        state_with_subscriptions(crate::test_support::state_subscription::test_subscriptions());
    // When
    handle_file_events(
        state.as_ref(),
        vec![
            event(&PathBuf::from("/repo/.git/objects/ab/cdef")),
            event(&PathBuf::from("/repo/.git/worktrees/feature/index")),
        ],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
    // When
    handle_file_events(
        state.as_ref(),
        vec![
            event(&PathBuf::from("/repo/.git/objects/ab/cdef")),
            event(&PathBuf::from("/repo/src/main.rs")),
        ],
    );
    // Then
    assert_eq!(state.requested_generation(), 1);
}

#[test]
fn test_git監視_他のworktreeのindexの変化ではrootの変更の状態を読み直さない() {
    // Given
    let state =
        state_with_subscriptions(crate::test_support::state_subscription::test_subscriptions());
    // When
    handle_git_events(
        state.as_ref(),
        Path::new("/repo/.git"),
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/index"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
}

#[test]
fn test_git監視_linked_worktreeは自分のindexだけを読み直しrefの変化では読み直さない() {
    // Given
    let state = state_at(
        "/repo-worktrees/feature",
        false,
        crate::test_support::state_subscription::test_subscriptions(),
    );
    let git_dir = Path::new("/repo/.git/worktrees/feature");
    // When
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/HEAD"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 0);
    // When
    handle_git_events(
        state.as_ref(),
        git_dir,
        &[event(&PathBuf::from("/repo/.git/worktrees/feature/index"))],
    );
    // Then
    assert_eq!(state.requested_generation(), 1);
}
