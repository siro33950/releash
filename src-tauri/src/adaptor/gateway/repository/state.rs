use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};
use tauri::{Emitter, Runtime};

use crate::adaptor::protocol::{BranchCardMsg, BranchListSync, WsMessage};
use crate::usecase::agent_session::context::invalidate_instruction_resolution_cache_for_path;
use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    RepositoryStateWorkerFuture, RepositoryStateWorkerRuntime, WorktreePathNormalizer,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::service::RepositoryStateRepository;
use crate::usecase::repository_state::snapshot::RepositorySnapshotChangedEvent;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worker::InvalidateReason;
use crate::usecase::repository_state::worktree::{
    RepositoryStateNotifier, RepositoryStateWatchSession, RepositoryStateWatcher,
    SnapshotNotification, SnapshotNotificationPhase, WorktreeState,
};
use crate::usecase::repository_state::RepositoryStateError;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::ws_bridge::WsBroadcaster;

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
        Ok(self.repository.get_main_repo_path(path)?)
    }
}

struct RepositoryStateWatcherHandles {
    _file_debouncer: RecommendedDebouncer,
    _git_debouncer: RecommendedDebouncer,
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
            _file_debouncer: file_debouncer,
            _git_debouncer: git_debouncer,
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
            .watch(git_dir, RecursiveMode::NonRecursive)
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
        invalidate_instruction_resolution_cache_for_path(Path::new(&event_path));
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
        let scan_repo_path = repo_path.clone();
        tokio::task::spawn_blocking(move || scanner.scan(&scan_repo_path))
            .await
            .map_err(|err| {
                RepositoryStateError::Watcher(format!(
                    "repository snapshot worker failed for {repo_path}: {err}"
                ))
            })?
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

pub struct TauriRepositoryStateNotifier<R: Runtime> {
    app: tauri::AppHandle<R>,
    ws: Arc<WsBroadcaster>,
}

impl<R: Runtime> TauriRepositoryStateNotifier<R> {
    pub fn new(app: tauri::AppHandle<R>, ws: Arc<WsBroadcaster>) -> Self {
        Self { app, ws }
    }
}

impl<R: Runtime> RepositoryStateNotifier for TauriRepositoryStateNotifier<R> {
    fn snapshot_changed(&self, notification: SnapshotNotification) {
        for worktree_path in &notification.worktree_paths {
            let event = RepositorySnapshotChangedEvent::from_snapshot(
                worktree_path.clone(),
                &notification.snapshot,
            );
            let _ = self.app.emit("repository-snapshot-changed", event);
        }

        if notification.phase == SnapshotNotificationPhase::RefreshStarted {
            return;
        }

        for worktree_path in &notification.worktree_paths {
            let _ = self.app.emit(
                "git-status-changed",
                GitStatusChangedEvent {
                    repo_path: worktree_path.clone(),
                },
            );
        }

        let _ = self.app.emit("branch-list-sync", ());
        let branch_msgs: Vec<BranchCardMsg> = notification
            .snapshot
            .branch_cards
            .iter()
            .cloned()
            .map(BranchCardMsg::from)
            .collect();
        self.ws.try_send(WsMessage::BranchListSync(BranchListSync {
            branches: branch_msgs,
        }));

        if notification.reason.file_change {
            let path = notification.reason.path.unwrap_or_else(|| {
                notification
                    .worktree_paths
                    .first()
                    .cloned()
                    .unwrap_or_default()
            });
            for watcher_id in notification.file_watcher_ids {
                let _ = self.app.emit(
                    "file-change",
                    FileChangeEvent {
                        watcher_id,
                        path: path.clone(),
                        kind: "change".to_string(),
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::repository_dto::{BranchCardDto, FileStatusDto};
    use crate::usecase::repository_state::runtime::{
        RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    };
    use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
    use crate::usecase::repository_state::worktree::RepositoryStateNotifier;
    use notify_debouncer_mini::DebouncedEventKind;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct InertSender;

    impl RepositoryStateInvalidationSender for InertSender {
        fn send(&self, _reason: InvalidateReason) -> Result<(), ()> {
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

        fn spawn_worker(&self, _future: RepositoryStateWorkerFuture) {}

        async fn sleep(&self, _duration: Duration) {}

        async fn scan(
            &self,
            _scanner: Arc<dyn RepositoryScanner>,
            _repo_path: String,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            Err(RepositoryStateError::Watcher(
                "inert runtime does not scan".to_string(),
            ))
        }
    }

    struct EmptyScanner;

    impl RepositoryScanner for EmptyScanner {
        fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            Ok(RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                branch_cards: Vec::new(),
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
                limited: false,
            })
        }

        fn status_with_ignored(
            &self,
            _repo_path: &str,
        ) -> Result<Vec<FileStatusDto>, RepositoryStateError> {
            Ok(Vec::new())
        }

        fn prune_stale_branch_bases(
            &self,
            _repo_path: &str,
            _existing_branches: &[String],
        ) -> Result<(), RepositoryStateError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingNotifier {
        refresh_started: AtomicUsize,
        snapshot_committed: AtomicUsize,
    }

    impl RepositoryStateNotifier for CountingNotifier {
        fn snapshot_changed(&self, notification: SnapshotNotification) {
            match notification.phase {
                SnapshotNotificationPhase::RefreshStarted => {
                    self.refresh_started.fetch_add(1, Ordering::SeqCst);
                }
                SnapshotNotificationPhase::SnapshotCommitted => {
                    self.snapshot_committed.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    fn event(path: &std::path::Path) -> DebouncedEvent {
        DebouncedEvent {
            path: path.to_path_buf(),
            kind: DebouncedEventKind::Any,
        }
    }

    fn state_with_notifier(notifier: Arc<CountingNotifier>) -> Arc<WorktreeState> {
        WorktreeState::new(
            "/repo".to_string(),
            Arc::new(EmptyScanner),
            notifier,
            Arc::new(InertRuntime),
            Duration::ZERO,
        )
    }

    #[test]
    fn watcher_callbacks_only_invalidate_until_worker_commit() {
        let notifier = Arc::new(CountingNotifier::default());
        let state = state_with_notifier(notifier.clone());
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "content").unwrap();

        handle_file_events(state.as_ref(), vec![event(&file_path)]);
        handle_git_events(state.as_ref(), &[event(&PathBuf::from("/repo/.git/HEAD"))]);
        handle_git_events(state.as_ref(), &[event(&PathBuf::from("/repo/.git/index"))]);

        assert_eq!(state.requested_generation(), 3);
        assert_eq!(notifier.refresh_started.load(Ordering::SeqCst), 0);
        assert_eq!(notifier.snapshot_committed.load(Ordering::SeqCst), 0);

        let snapshot = state.commit_snapshot(
            RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                branch_cards: vec![BranchCardDto {
                    name: "main".to_string(),
                    is_main_worktree: true,
                    worktree_path: Some("/repo".to_string()),
                    dirty_count: 0,
                    is_merged: false,
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                    base_ahead: 0,
                }],
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
                limited: false,
            },
            state.requested_generation(),
        );
        state.notify_snapshot_changed(snapshot, InvalidateReason::git(true));

        assert_eq!(notifier.snapshot_committed.load(Ordering::SeqCst), 1);
    }
}
