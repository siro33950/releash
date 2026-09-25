use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::adaptor::gateway::push::BackendPush;
use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};

use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    RepositoryStateWorkerFuture, RepositoryStateWorkerRuntime, WorktreePathNormalizer,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::service::RepositoryStateRepository;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worker::InvalidateReason;
use crate::usecase::repository_state::worktree::{
    RepositoryStateNotifier, RepositoryStateWatchSession, RepositoryStateWatcher,
    SnapshotNotification, WorktreeState,
};
use crate::usecase::repository_state::RepositoryStateError;
use crate::usecase::repository_usecase::RepositoryUsecase;

use super::watch::{
    canonicalize_event_path, classify_git_dir_events, generate_watcher_id,
    resolve_file_watch_paths, resolve_git_watch_paths, FileChangeEvent, GitStatusChangedEvent,
};

type RecommendedDebouncer =
    notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>;

pub struct RepositoryStateRepositoryGateway {
    repository: Arc<RepositoryUsecase>,
}

impl RepositoryStateRepositoryGateway {
    pub fn new(repository: Arc<RepositoryUsecase>) -> Self {
        Self { repository }
    }
}

impl RepositoryStateRepository for RepositoryStateRepositoryGateway {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        let root = self.repository.get_main_repo_path(path)?;
        Ok(super::worktree_operation::worktree_identity(&root)
            .map_err(crate::usecase::repository_error::UsecaseError::from)?
            .to_string_lossy()
            .into_owned())
    }

    fn include_deleting_worktrees(
        &self,
        repository_root: &str,
        cards: &mut Vec<crate::usecase::repository_dto::BranchCardDto>,
    ) -> Result<(), RepositoryStateError> {
        for card in cards.iter_mut() {
            if let Some(path) = &mut card.worktree_path {
                *path = super::worktree_operation::worktree_identity(path)
                    .map_err(crate::usecase::repository_error::UsecaseError::from)?
                    .to_string_lossy()
                    .into_owned();
            }
        }
        self.repository
            .include_deleting_worktrees(repository_root, cards);
        Ok(())
    }
}

struct RepositoryStateWatcherHandles {
    file_debouncer: Option<RecommendedDebouncer>,
    git_debouncer: Option<RecommendedDebouncer>,
}

impl Drop for RepositoryStateWatcherHandles {
    fn drop(&mut self) {
        // FsEventWatcher の drop は run loop の停止待ちでブロックし得るため、
        // 呼び出し元スレッドでは実行しない（#1641）
        crate::infrastructure::dispose::dispose_in_background(
            "repository-watcher-dispose",
            (self.file_debouncer.take(), self.git_debouncer.take()),
        );
    }
}

pub struct NotifyRepositoryStateWatcher {
    repository: Arc<RepositoryUsecase>,
}

impl NotifyRepositoryStateWatcher {
    pub fn new(repository: Arc<RepositoryUsecase>) -> Self {
        Self { repository }
    }
}

impl RepositoryStateWatcher for NotifyRepositoryStateWatcher {
    fn next_watcher_id(&self) -> u64 {
        generate_watcher_id()
    }

    fn start_watchers(
        &self,
        state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError> {
        let file_debouncer = start_file_watcher(state.clone(), &self.repository)?;
        let git_debouncer = start_git_watcher(state, &self.repository)?;
        Ok(Box::new(RepositoryStateWatcherHandles {
            file_debouncer: Some(file_debouncer),
            git_debouncer: Some(git_debouncer),
        }))
    }
}

fn start_file_watcher(
    state: Arc<WorktreeState>,
    repository: &RepositoryUsecase,
) -> Result<RecommendedDebouncer, RepositoryStateError> {
    let watch_paths = resolve_file_watch_paths(repository, state.worktree_path());
    let debouncer = new_debouncer(
        Duration::from_millis(100),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| match res {
            Ok(events) => handle_file_events(state.as_ref(), events),
            Err(err) => {
                log::warn!("file watcher error for {}: {err:?}", state.worktree_path());
            }
        },
    )
    .map_err(|err| RepositoryStateError::Watcher(format!("Failed to create debouncer: {err}")))?;

    let mut debouncer = debouncer;
    for watch_path in watch_paths {
        debouncer
            .watcher()
            .watch(&watch_path, RecursiveMode::Recursive)
            .map_err(|err| {
                RepositoryStateError::Watcher(format!(
                    "Failed to watch path {}: {err}",
                    watch_path.display()
                ))
            })?;
    }
    Ok(debouncer)
}

fn start_git_watcher(
    state: Arc<WorktreeState>,
    repository: &RepositoryUsecase,
) -> Result<RecommendedDebouncer, RepositoryStateError> {
    let paths = resolve_git_watch_paths(repository, state.worktree_path())
        .map_err(RepositoryStateError::Watcher)?;
    log::debug!("starting git watcher for main repo {}", paths.main_repo);
    let debouncer = new_debouncer(
        Duration::from_millis(100),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| {
            let events = match res {
                Ok(events) => events,
                Err(err) => {
                    log::warn!(
                        "git dir watcher error for {}: {err:?}",
                        state.worktree_path()
                    );
                    return;
                }
            };
            handle_git_events(state.as_ref(), &events);
        },
    )
    .map_err(|err| RepositoryStateError::Watcher(format!("Failed to create debouncer: {err}")))?;

    let mut debouncer = debouncer;
    if paths.refs_heads.exists() {
        debouncer
            .watcher()
            .watch(&paths.refs_heads, RecursiveMode::Recursive)
            .map_err(|err| {
                RepositoryStateError::Watcher(format!("Failed to watch refs/heads: {err}"))
            })?;
    }
    if paths.head_file.exists() {
        debouncer
            .watcher()
            .watch(&paths.head_file, RecursiveMode::NonRecursive)
            .map_err(|err| RepositoryStateError::Watcher(format!("Failed to watch HEAD: {err}")))?;
    }
    if let Some(git_dir) = paths.index_file.parent() {
        debouncer
            .watcher()
            .watch(git_dir, RecursiveMode::Recursive)
            .map_err(|err| {
                RepositoryStateError::Watcher(format!("Failed to watch .git dir: {err}"))
            })?;
    }
    if paths.worktrees_dir.exists() {
        debouncer
            .watcher()
            .watch(&paths.worktrees_dir, RecursiveMode::Recursive)
            .map_err(|err| {
                RepositoryStateError::Watcher(format!("Failed to watch worktrees: {err}"))
            })?;
    }
    Ok(debouncer)
}

fn handle_file_events(state: &WorktreeState, events: Vec<DebouncedEvent>) {
    for event in events {
        let event_path = canonicalize_event_path(&event.path);
        state.invalidate(InvalidateReason::file(Some(event_path)));
    }
}

fn handle_git_events(state: &WorktreeState, events: &[DebouncedEvent]) {
    let (branch_change, index_change) = classify_git_dir_events(events);
    if branch_change || index_change {
        state.invalidate(InvalidateReason::git(branch_change));
    }
}

pub struct TokioRepositoryStateWorkerRuntime;

struct TokioInvalidationSender(tokio::sync::mpsc::UnboundedSender<InvalidateReason>);

impl RepositoryStateInvalidationSender for TokioInvalidationSender {
    fn send(&self, reason: InvalidateReason) -> Result<(), ()> {
        self.0.send(reason).map_err(|_| ())
    }
}

struct TokioInvalidationReceiver(tokio::sync::mpsc::UnboundedReceiver<InvalidateReason>);

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
impl RepositoryStateWorkerRuntime for TokioRepositoryStateWorkerRuntime {
    fn invalidation_channel(
        &self,
    ) -> (
        Box<dyn RepositoryStateInvalidationSender>,
        Box<dyn RepositoryStateInvalidationReceiver>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Box::new(TokioInvalidationSender(tx)),
            Box::new(TokioInvalidationReceiver(rx)),
        )
    }

    fn spawn_worker(&self, future: RepositoryStateWorkerFuture) {
        tokio::spawn(future);
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    async fn scan(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        repo_path: String,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        scanner.scan_async(&repo_path).await
    }
}

pub struct FsWorktreePathNormalizer;

impl WorktreePathNormalizer for FsWorktreePathNormalizer {
    fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
        let path = Path::new(worktree_path);
        path.canonicalize().map_err(|err| {
            RepositoryStateError::Watcher(format!(
                "Failed to canonicalize worktree path {}: {err}",
                path.display()
            ))
        })
    }
}

pub struct ClientRepositoryStateNotifier {
    publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    sink: std::sync::Arc<crate::infrastructure::push::PushSink>,
}

impl ClientRepositoryStateNotifier {
    pub fn new(
        sink: std::sync::Arc<crate::infrastructure::push::PushSink>,
        publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    ) -> Self {
        Self { sink, publisher }
    }
}

impl RepositoryStateNotifier for ClientRepositoryStateNotifier {
    fn snapshot_changed(&self, notification: SnapshotNotification) {
        for worktree_path in &notification.worktree_paths {
            BackendPush::GitStatusChanged(GitStatusChangedEvent {
                repo_path: worktree_path.clone(),
            })
            .emit(&self.sink);
        }

        self.publisher.invalidate(
            crate::domain::state_subscription::StateChangeSource::Repository(
                notification.worktree_paths.clone(),
            ),
        );

        if notification.reason.file_change {
            let path = notification.reason.path.unwrap_or_else(|| {
                notification
                    .worktree_paths
                    .first()
                    .cloned()
                    .unwrap_or_default()
            });
            for watcher_id in notification.file_watcher_ids {
                BackendPush::FileChange(FileChangeEvent {
                    watcher_id,
                    path: path.clone(),
                    kind: "change".to_string(),
                })
                .emit(&self.sink);
            }
        }
    }
}

#[cfg(test)]
#[path = "state_test.rs"]
mod state_tests;
