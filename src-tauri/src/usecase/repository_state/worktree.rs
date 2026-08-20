use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::error::RepositoryStateError;
use super::runtime::{RepositoryStateInvalidationSender, RepositoryStateWorkerRuntime};
use super::scanner::RepositoryScanner;
use super::snapshot::{RepositorySnapshot, RepositorySnapshotParts};
use super::worker::{run_worker, InvalidateReason};

pub trait RepositoryStateWatchSession: Send + Sync {}

impl<T> RepositoryStateWatchSession for T where T: Send + Sync {}

pub trait RepositoryStateWatcher: Send + Sync {
    fn next_watcher_id(&self) -> u64;

    fn start_watchers(
        &self,
        state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError>;
}

#[derive(Clone)]
pub struct SnapshotNotification {
    pub worktree_paths: Vec<String>,
    pub snapshot: Arc<RepositorySnapshot>,
    pub file_watcher_ids: Vec<u64>,
    pub reason: InvalidateReason,
    pub phase: SnapshotNotificationPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotNotificationPhase {
    RefreshStarted,
    SnapshotCommitted,
}

pub trait RepositoryStateNotifier: Send + Sync {
    fn snapshot_changed(&self, notification: SnapshotNotification);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchSubscriptionKind {
    File,
    Git,
}

#[derive(Debug, Clone)]
struct WatchSubscription {
    id: u64,
    kind: WatchSubscriptionKind,
    worktree_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct NotificationTargets {
    worktree_paths: Vec<String>,
    file_watcher_ids: Vec<u64>,
}

#[cfg(test)]
#[derive(Default)]
pub struct NoopRepositoryStateNotifier;

#[cfg(test)]
impl RepositoryStateNotifier for NoopRepositoryStateNotifier {
    fn snapshot_changed(&self, _notification: SnapshotNotification) {}
}

#[cfg(test)]
#[derive(Default)]
pub struct NoopRepositoryStateWatcher;

#[cfg(test)]
impl RepositoryStateWatcher for NoopRepositoryStateWatcher {
    fn next_watcher_id(&self) -> u64 {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        NEXT_ID.fetch_add(1, Ordering::SeqCst)
    }

    fn start_watchers(
        &self,
        _state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError> {
        Ok(Box::new(()))
    }
}

pub struct WorktreeState {
    worktree_path: String,
    snapshot: RwLock<Arc<RepositorySnapshot>>,
    version: AtomicU64,
    requested_generation: AtomicU64,
    applied_generation: AtomicU64,
    refreshing: AtomicBool,
    shutdown: AtomicBool,
    invalidate_tx: Box<dyn RepositoryStateInvalidationSender>,
    watchers: Mutex<Option<Box<dyn RepositoryStateWatchSession>>>,
    subscriptions: Mutex<HashMap<u64, WatchSubscription>>,
    notifier: Arc<dyn RepositoryStateNotifier>,
}

impl WorktreeState {
    pub fn new(
        worktree_path: String,
        scanner: Arc<dyn RepositoryScanner>,
        notifier: Arc<dyn RepositoryStateNotifier>,
        runtime: Arc<dyn RepositoryStateWorkerRuntime>,
        debounce: Duration,
    ) -> Arc<Self> {
        let (invalidate_tx, invalidate_rx) = runtime.invalidation_channel();
        let state = Arc::new(Self {
            worktree_path,
            snapshot: RwLock::new(Arc::new(RepositorySnapshot::loading())),
            version: AtomicU64::new(0),
            requested_generation: AtomicU64::new(0),
            applied_generation: AtomicU64::new(0),
            refreshing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            invalidate_tx,
            watchers: Mutex::new(None),
            subscriptions: Mutex::new(HashMap::new()),
            notifier,
        });
        runtime.spawn_worker(Box::pin(run_worker(
            state.clone(),
            scanner,
            runtime.clone(),
            invalidate_rx,
            debounce,
        )));
        state
    }

    pub fn worktree_path(&self) -> &str {
        &self.worktree_path
    }

    pub fn requested_generation(&self) -> u64 {
        self.requested_generation.load(Ordering::SeqCst)
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub fn set_refreshing(&self, refreshing: bool) {
        self.refreshing.store(refreshing, Ordering::SeqCst);
    }

    pub fn add_subscription(&self, id: u64, kind: WatchSubscriptionKind, worktree_path: String) {
        self.subscriptions.lock().insert(
            id,
            WatchSubscription {
                id,
                kind,
                worktree_path,
            },
        );
    }

    pub fn release_subscription(&self, id: u64) -> bool {
        self.subscriptions.lock().remove(&id).is_some()
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscriptions.lock().len()
    }

    pub fn shutdown(&self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        self.watchers.lock().take();
        let _ = self.invalidate_tx.send(InvalidateReason::shutdown());
    }

    pub fn start_watchers(
        self: &Arc<Self>,
        watcher: &dyn RepositoryStateWatcher,
    ) -> Result<(), RepositoryStateError> {
        let mut watchers = self.watchers.lock();
        if watchers.is_some() {
            return Ok(());
        }

        *watchers = Some(watcher.start_watchers(self.clone())?);
        drop(watchers);

        self.invalidate(InvalidateReason::initial());
        Ok(())
    }

    pub fn invalidate(&self, reason: InvalidateReason) {
        if self.is_shutdown() {
            return;
        }
        self.requested_generation.fetch_add(1, Ordering::SeqCst);
        if self.invalidate_tx.send(reason).is_err() {
            log::warn!(
                "repository snapshot worker is stopped for {}",
                self.worktree_path
            );
        }
    }

    pub fn snapshot_for_read(&self) -> Arc<RepositorySnapshot> {
        let snapshot = self.snapshot.read().clone();
        if self.refreshing.load(Ordering::SeqCst) {
            let stale = snapshot.version > 0;
            return Arc::new(snapshot.with_read_flags(stale, true));
        }
        snapshot
    }

    pub(crate) fn mark_refresh_started(&self, reason: &InvalidateReason) {
        self.refreshing.store(true, Ordering::SeqCst);
        let snapshot = self.snapshot_for_read();
        let targets = self.notification_targets();
        self.notifier.snapshot_changed(SnapshotNotification {
            worktree_paths: targets.worktree_paths,
            snapshot,
            file_watcher_ids: targets.file_watcher_ids,
            reason: reason.clone(),
            phase: SnapshotNotificationPhase::RefreshStarted,
        });
    }

    pub(crate) fn commit_snapshot(
        &self,
        parts: RepositorySnapshotParts,
        generation: u64,
    ) -> Arc<RepositorySnapshot> {
        let version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Arc::new(parts.into_snapshot(version));
        *self.snapshot.write() = snapshot.clone();
        self.applied_generation.store(generation, Ordering::SeqCst);
        snapshot
    }

    pub(crate) fn notify_snapshot_changed(
        &self,
        snapshot: Arc<RepositorySnapshot>,
        reason: InvalidateReason,
    ) {
        let targets = self.notification_targets();
        self.notifier.snapshot_changed(SnapshotNotification {
            worktree_paths: targets.worktree_paths,
            snapshot,
            file_watcher_ids: targets.file_watcher_ids,
            reason,
            phase: SnapshotNotificationPhase::SnapshotCommitted,
        });
    }

    pub(crate) fn mark_scan_failed(&self) {
        self.refreshing.store(false, Ordering::SeqCst);
        let current = self.snapshot.read().clone();
        if current.version == 0 && current.flags.loading {
            *self.snapshot.write() = Arc::new(current.with_read_flags(false, false));
        }
    }

    fn notification_targets(&self) -> NotificationTargets {
        let subscriptions = self.subscriptions.lock();
        let mut targets = NotificationTargets::default();
        for subscription in subscriptions.values() {
            if !targets
                .worktree_paths
                .iter()
                .any(|path| path == &subscription.worktree_path)
            {
                targets
                    .worktree_paths
                    .push(subscription.worktree_path.clone());
            }
            if subscription.kind == WatchSubscriptionKind::File {
                targets.file_watcher_ids.push(subscription.id);
            }
        }
        if targets.worktree_paths.is_empty() {
            targets.worktree_paths.push(self.worktree_path.clone());
        }
        targets
    }
}

impl Drop for WorktreeState {
    fn drop(&mut self) {
        self.watchers.lock().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecase::repository_dto::{BranchCardDto, FileDiffStatDto, FileStatusDto};
    use crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::mpsc as std_mpsc;

    #[derive(Default)]
    struct CapturingNotifier {
        notifications: parking_lot::Mutex<Vec<SnapshotNotification>>,
    }

    impl CapturingNotifier {
        fn take(&self) -> Vec<SnapshotNotification> {
            std::mem::take(&mut *self.notifications.lock())
        }
    }

    impl RepositoryStateNotifier for CapturingNotifier {
        fn snapshot_changed(&self, notification: SnapshotNotification) {
            self.notifications.lock().push(notification);
        }
    }

    type OnScanHook = Box<dyn Fn(usize) + Send + Sync>;

    struct FakeScanner {
        scans: AtomicUsize,
        value: parking_lot::Mutex<String>,
        fail: AtomicBool,
        prunes: parking_lot::Mutex<Vec<Vec<String>>>,
        sleep: Duration,
        on_scan: parking_lot::Mutex<Option<OnScanHook>>,
    }

    impl FakeScanner {
        fn new(value: &str) -> Self {
            Self {
                scans: AtomicUsize::new(0),
                value: parking_lot::Mutex::new(value.to_string()),
                fail: AtomicBool::new(false),
                prunes: parking_lot::Mutex::new(Vec::new()),
                sleep: Duration::ZERO,
                on_scan: parking_lot::Mutex::new(None),
            }
        }

        fn with_sleep(mut self, sleep: Duration) -> Self {
            self.sleep = sleep;
            self
        }

        fn set_value(&self, value: &str) {
            *self.value.lock() = value.to_string();
        }

        fn scan_count(&self) -> usize {
            self.scans.load(Ordering::SeqCst)
        }

        fn set_fail(&self, fail: bool) {
            self.fail.store(fail, Ordering::SeqCst);
        }

        fn take_prune_calls(&self) -> Vec<Vec<String>> {
            std::mem::take(&mut *self.prunes.lock())
        }

        fn set_on_scan(&self, f: impl Fn(usize) + Send + Sync + 'static) {
            *self.on_scan.lock() = Some(Box::new(f));
        }
    }

    impl RepositoryScanner for FakeScanner {
        fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            let call = self.scans.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(on_scan) = self.on_scan.lock().as_ref() {
                on_scan(call);
            }
            let value = self.value.lock().clone();
            if self.sleep > Duration::ZERO {
                std::thread::sleep(self.sleep);
            }
            if self.fail.load(Ordering::SeqCst) {
                return Err(RepositoryStateError::Watcher("scan failed".to_string()));
            }
            Ok(RepositorySnapshotParts {
                status: vec![FileStatusDto {
                    path: value,
                    index_status: "none".to_string(),
                    worktree_status: "modified".to_string(),
                }],
                diff_stats: vec![FileDiffStatDto {
                    path: "file.txt".to_string(),
                    index_additions: 0,
                    index_deletions: 0,
                    wt_additions: 1,
                    wt_deletions: 0,
                }],
                branch_cards: vec![BranchCardDto {
                    name: "main".to_string(),
                    is_main_worktree: true,
                    worktree_path: Some("/repo".to_string()),
                    dirty_count: 1,
                    is_merged: false,
                    ahead: 0,
                    behind: 0,
                    has_upstream: false,
                    base_ahead: 0,
                    management_kind: None,
                }],
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
            Ok(vec![FileStatusDto {
                path: "ignored.txt".to_string(),
                index_status: "none".to_string(),
                worktree_status: "ignored".to_string(),
            }])
        }

        fn prune_stale_branch_bases(
            &self,
            _repo_path: &str,
            existing_branches: &[String],
        ) -> Result<(), RepositoryStateError> {
            self.prunes.lock().push(existing_branches.to_vec());
            Ok(())
        }
    }

    fn test_state(scanner: Arc<dyn RepositoryScanner>, debounce: Duration) -> Arc<WorktreeState> {
        WorktreeState::new(
            "/repo".to_string(),
            scanner,
            Arc::new(NoopRepositoryStateNotifier),
            Arc::new(TestRepositoryStateWorkerRuntime),
            debounce,
        )
    }

    fn test_state_with_notifier(
        scanner: Arc<dyn RepositoryScanner>,
        notifier: Arc<dyn RepositoryStateNotifier>,
    ) -> Arc<WorktreeState> {
        WorktreeState::new(
            "/repo".to_string(),
            scanner,
            notifier,
            Arc::new(TestRepositoryStateWorkerRuntime),
            Duration::ZERO,
        )
    }

    async fn wait_for_version(state: &WorktreeState, version: u64) -> Arc<RepositorySnapshot> {
        for _ in 0..100 {
            let snapshot = state.snapshot_for_read();
            if snapshot.version >= version && !snapshot.flags.loading {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for version {version}");
    }

    #[tokio::test]
    async fn loading_snapshot_becomes_ready_after_scan() {
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        let state = test_state(scanner.clone(), Duration::ZERO);

        let initial = state.snapshot_for_read();
        assert_eq!(initial.version, 0);
        assert!(initial.flags.loading);

        state.invalidate(InvalidateReason::initial());
        let ready = wait_for_version(&state, 1).await;

        assert_eq!(ready.version, 1);
        assert!(!ready.flags.loading);
        assert!(!ready.flags.stale);
        assert_eq!(scanner.take_prune_calls(), vec![vec!["main".to_string()]]);
    }

    #[tokio::test]
    async fn snapshot_is_stale_while_refresh_is_running() {
        let scanner = Arc::new(FakeScanner::new("first.txt").with_sleep(Duration::from_millis(80)));
        let state = test_state(scanner.clone(), Duration::ZERO);

        state.invalidate(InvalidateReason::initial());
        let first = wait_for_version(&state, 1).await;
        assert_eq!(first.status[0].path, "first.txt");

        scanner.set_value("second.txt");
        state.invalidate(InvalidateReason::git(false));
        tokio::time::sleep(Duration::from_millis(10)).await;

        let stale = state.snapshot_for_read();
        assert_eq!(stale.version, 1);
        assert!(stale.flags.loading);
        assert!(stale.flags.stale);

        let second = wait_for_version(&state, 2).await;
        assert_eq!(second.status[0].path, "second.txt");
        assert!(!second.flags.stale);
    }

    #[tokio::test]
    async fn refresh_start_notification_exposes_loading_and_stale_flags() {
        let scanner = Arc::new(FakeScanner::new("first.txt").with_sleep(Duration::from_millis(80)));
        let notifier = Arc::new(CapturingNotifier::default());
        let state = test_state_with_notifier(scanner.clone(), notifier.clone());

        state.invalidate(InvalidateReason::initial());
        wait_for_version(&state, 1).await;
        notifier.take();

        scanner.set_value("second.txt");
        state.invalidate(InvalidateReason::git(false));

        for _ in 0..100 {
            let notifications = notifier.take();
            if let Some(started) = notifications
                .iter()
                .find(|n| n.phase == SnapshotNotificationPhase::RefreshStarted)
            {
                assert_eq!(started.snapshot.version, 1);
                assert!(started.snapshot.flags.loading);
                assert!(started.snapshot.flags.stale);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("timed out waiting for refresh start notification");
    }

    #[tokio::test]
    async fn debounce_groups_multiple_invalidations_into_one_scan() {
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        let state = test_state(scanner.clone(), Duration::from_millis(40));

        state.invalidate(InvalidateReason::file(None));
        state.invalidate(InvalidateReason::file(None));
        state.invalidate(InvalidateReason::git(false));
        wait_for_version(&state, 1).await;

        assert_eq!(scanner.scan_count(), 1);
    }

    #[tokio::test]
    async fn superseded_scan_result_is_not_committed() {
        let scanner = Arc::new(FakeScanner::new("old.txt").with_sleep(Duration::from_millis(80)));
        let state = test_state(scanner.clone(), Duration::ZERO);

        state.invalidate(InvalidateReason::initial());
        wait_for_version(&state, 1).await;

        let (tx, rx) = std_mpsc::channel();
        scanner.set_on_scan(move |call| {
            if call == 2 {
                tx.send(()).unwrap();
            }
        });

        state.invalidate(InvalidateReason::git(false));
        tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(1)))
            .await
            .unwrap()
            .unwrap();
        scanner.set_value("new.txt");
        state.invalidate(InvalidateReason::file(None));

        let latest = wait_for_version(&state, 2).await;
        assert_eq!(latest.status[0].path, "new.txt");
        assert!(scanner.scan_count() >= 3);
        assert_eq!(scanner.take_prune_calls().len(), 2);
    }

    #[tokio::test]
    async fn failed_scan_is_not_pruned_or_committed() {
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        scanner.set_fail(true);
        let state = test_state(scanner.clone(), Duration::ZERO);

        state.invalidate(InvalidateReason::initial());
        tokio::time::sleep(Duration::from_millis(30)).await;

        let snapshot = state.snapshot_for_read();
        assert_eq!(snapshot.version, 0);
        assert!(!snapshot.flags.loading);
        assert!(scanner.take_prune_calls().is_empty());
    }
}
