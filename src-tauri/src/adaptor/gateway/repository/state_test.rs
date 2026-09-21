use super::*;
use crate::adaptor::protocol::client as wire;
use crate::infrastructure::push::PushSink;
use crate::usecase::repository_dto::{BranchCardDto, FileStatusDto};
use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
};
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worktree::RepositoryStateNotifier;
use notify_debouncer_mini::DebouncedEventKind;
use prost::Message;
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
    snapshot_committed: AtomicUsize,
}

impl RepositoryStateNotifier for CountingNotifier {
    fn snapshot_changed(&self, _notification: SnapshotNotification) {
        self.snapshot_committed.fetch_add(1, Ordering::SeqCst);
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
    assert_eq!(notifier.snapshot_committed.load(Ordering::SeqCst), 0);

    state.commit_snapshot(
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
        },
        state.requested_generation(),
    );
    state.notify_snapshot_changed(InvalidateReason::git(true));

    assert_eq!(notifier.snapshot_committed.load(Ordering::SeqCst), 1);
}

#[test]
fn test_スキャン完了通知_gitとbranchとfileのみclientへ送る() {
    // Given
    let sink = Arc::new(PushSink::new());
    let mut receiver = sink.subscribe();
    let notifier = ClientRepositoryStateNotifier::new(sink);

    // When
    notifier.snapshot_changed(SnapshotNotification {
        worktree_paths: vec!["/repo".into()],
        file_watcher_ids: vec![7],
        reason: InvalidateReason::file(Some("/repo/file.txt".into())),
    });

    // Then
    let mut events = Vec::new();
    while let Ok(bytes) = receiver.try_recv() {
        events.push(wire::Push::decode(bytes.as_ref()).unwrap().event.unwrap());
    }
    assert_eq!(
        events,
        vec![
            wire::push::Event::GitStatusChanged(wire::GitStatusChangedEvent {
                repo_path: Some("/repo".into()),
            }),
            wire::push::Event::BranchListSync(wire::Unit {}),
            wire::push::Event::FileChange(wire::FileChangeEvent {
                watcher_id: Some(7),
                path: Some("/repo/file.txt".into()),
                kind: Some("change".into()),
            }),
        ]
    );
}
