use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

use super::error::RepositoryStateError;
use super::runtime::{RepositoryStateInvalidationSender, RepositoryStateWorkerRuntime, ScanWorker};
use super::scanner::RepositoryScanner;
use super::snapshot::{RepositorySnapshot, RepositorySnapshotParts};
use super::worker::InvalidateReason;
use crate::domain::repository::Worktree;
use crate::usecase::fetched::Fetched;

pub trait RepositoryStateWatchSession: Send + Sync {}

impl<T> RepositoryStateWatchSession for T where T: Send + Sync {}

pub trait RepositoryStateWatcher: Send + Sync {
    fn next_watcher_id(&self) -> u64;

    fn start_watchers(
        &self,
        state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError>;
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
    /// この path が Repository の root（main worktree）か。root だけが worktree の並びを持つ。
    is_repository_root: bool,
    pub(crate) scan_lock: tokio::sync::Mutex<()>,
    snapshot: RwLock<Arc<RepositorySnapshot>>,
    worktrees: RwLock<Fetched<Vec<Worktree>>>,
    scan_failure: RwLock<Option<crate::usecase::failure::WorkFailure>>,
    version: AtomicU64,
    requested_generation: AtomicU64,
    applied_generation: AtomicU64,
    refreshing: AtomicBool,
    shutdown: AtomicBool,
    invalidate_tx: Box<dyn RepositoryStateInvalidationSender>,
    watchers: Mutex<Option<Box<dyn RepositoryStateWatchSession>>>,
    subscriptions: Mutex<HashMap<u64, String>>,
    state_subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
}

impl WorktreeState {
    pub fn new(
        worktree_path: String,
        is_repository_root: bool,
        scanner: Arc<dyn RepositoryScanner>,
        state_subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
        runtime: Arc<dyn RepositoryStateWorkerRuntime>,
        debounce: Duration,
    ) -> Arc<Self> {
        let (invalidate_tx, invalidate_rx) = runtime.invalidation_channel();
        let state = Arc::new(Self {
            worktree_path,
            is_repository_root,
            scan_lock: tokio::sync::Mutex::new(()),
            snapshot: RwLock::new(Arc::new(RepositorySnapshot::loading())),
            worktrees: RwLock::new(Fetched::default()),
            scan_failure: RwLock::new(None),
            version: AtomicU64::new(0),
            requested_generation: AtomicU64::new(0),
            applied_generation: AtomicU64::new(0),
            refreshing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            invalidate_tx,
            watchers: Mutex::new(None),
            subscriptions: Mutex::new(HashMap::new()),
            state_subscriptions,
        });
        runtime.spawn_worker(ScanWorker {
            state: state.clone(),
            scanner,
            receiver: invalidate_rx,
            debounce,
        });
        state
    }

    pub async fn scan_once(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        runtime: &dyn RepositoryStateWorkerRuntime,
    ) -> Result<Option<Arc<RepositorySnapshot>>, RepositoryStateError> {
        let _scan = self.scan_lock.lock().await;
        if self.is_shutdown() {
            return Ok(None);
        }
        self.set_refreshing(true);
        let generation = self.requested_generation();
        let parts = runtime.scan(scanner, self.worktree_path.clone()).await?;
        Ok(self.commit_snapshot(parts, generation))
    }

    /// Repository の root なら worktree の並びを読み直す。失敗しても最後に読めた並びを残す。
    pub async fn scan_worktrees_once(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        runtime: &dyn RepositoryStateWorkerRuntime,
    ) {
        if !self.is_repository_root || self.is_shutdown() {
            return;
        }
        let result = runtime
            .scan_worktrees(scanner.clone(), self.worktree_path.clone())
            .await;
        if result.is_ok() {
            if let Err(err) = scanner.prune_stale_branch_bases(&self.worktree_path) {
                log::warn!(
                    "repository branch base GC failed for {}: {err}",
                    self.worktree_path
                );
            }
        }
        self.worktrees
            .write()
            .record(result.map_err(|error| error.to_string()));
    }

    /// 走査の結果を確定して知らせる。変更の状態を走査していないときは `status` は None。
    pub fn finish_scan(
        &self,
        status: Option<Result<Option<Arc<RepositorySnapshot>>, RepositoryStateError>>,
        reason: InvalidateReason,
    ) -> Option<InvalidateReason> {
        self.set_refreshing(false);
        match status {
            Some(Ok(None)) => Some(reason),
            Some(Ok(Some(_))) | None => {
                self.notify_snapshot_changed();
                None
            }
            Some(Err(err)) => {
                log::warn!(
                    "repository snapshot scan failed for {}: {err}",
                    self.worktree_path
                );
                self.mark_scan_failed(&err);
                self.notify_snapshot_changed();
                None
            }
        }
    }

    pub fn is_repository_root(&self) -> bool {
        self.is_repository_root
    }

    /// Repository の worktree の並び。root でなければ、まだ読めていないのと同じ。
    pub fn worktrees(&self) -> Fetched<Vec<Worktree>> {
        self.worktrees.read().clone()
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

    pub fn add_subscription(&self, id: u64, worktree_path: String) {
        self.subscriptions.lock().insert(id, worktree_path);
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

        self.invalidate(InvalidateReason::change());
        Ok(())
    }

    pub fn invalidate(&self, reason: InvalidateReason) {
        if self.is_shutdown() {
            return;
        }
        {
            let _snapshot = self.snapshot.write();
            self.requested_generation.fetch_add(1, Ordering::SeqCst);
        }
        if self.invalidate_tx.send(reason).is_err() {
            log::warn!(
                "repository snapshot worker is stopped for {}",
                self.worktree_path
            );
        }
    }

    pub fn read_snapshot(&self) -> Result<Arc<RepositorySnapshot>, RepositoryStateError> {
        let snapshot = self.snapshot_for_read();
        if let Some(error) = self.scan_failure.read().as_ref() {
            return Err(RepositoryStateError::Background {
                kind: error.kind,
                message: error.message.clone(),
            });
        }
        Ok(snapshot)
    }

    pub fn dirty_count(&self) -> Fetched<usize> {
        let snapshot = self.snapshot.read();
        let error = self.scan_failure.read();
        Fetched {
            value: (snapshot.version > 0 && error.is_none()).then_some(snapshot.dirty_count),
            error: error.as_ref().map(|e| e.message.clone()),
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

    pub(crate) fn commit_snapshot(
        &self,
        parts: RepositorySnapshotParts,
        generation: u64,
    ) -> Option<Arc<RepositorySnapshot>> {
        let mut current = self.snapshot.write();
        if self.requested_generation() != generation {
            return None;
        }
        let version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let snapshot = Arc::new(parts.into_snapshot(version));
        *current = snapshot.clone();
        *self.scan_failure.write() = None;
        self.applied_generation.store(generation, Ordering::SeqCst);
        Some(snapshot)
    }

    pub(crate) fn notify_snapshot_changed(&self) {
        self.state_subscriptions.notify(
            crate::usecase::state_subscription::StateChangeSource::Repository(
                self.notification_paths(),
            ),
        );
    }

    pub(crate) fn mark_scan_failed(&self, error: &RepositoryStateError) {
        *self.scan_failure.write() = Some(crate::usecase::failure::WorkFailure::from_error(error));
        self.refreshing.store(false, Ordering::SeqCst);
        let current = self.snapshot.read().clone();
        if current.version == 0 && current.flags.loading {
            *self.snapshot.write() = Arc::new(current.with_read_flags(false, false));
        }
    }

    fn notification_paths(&self) -> Vec<String> {
        let subscriptions = self.subscriptions.lock();
        let mut paths = Vec::new();
        for path in subscriptions.values() {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        if paths.is_empty() {
            paths.push(self.worktree_path.clone());
        }
        paths
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
    use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
    use crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::mpsc as std_mpsc;

    use crate::test_support::state_subscription::CapturingNotifier;

    type OnScanHook = Box<dyn Fn(usize) + Send + Sync>;

    struct FakeScanner {
        scans: AtomicUsize,
        value: parking_lot::Mutex<String>,
        fail: AtomicBool,
        prunes: parking_lot::Mutex<Vec<String>>,
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

        fn take_prune_calls(&self) -> Vec<String> {
            std::mem::take(&mut *self.prunes.lock())
        }

        fn set_on_scan(&self, f: impl Fn(usize) + Send + Sync + 'static) {
            *self.on_scan.lock() = Some(Box::new(f));
        }
    }

    #[async_trait::async_trait]

    impl RepositoryScanner for FakeScanner {
        async fn scan_async(
            &self,
            repo_path: &str,
        ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
            self.scan(repo_path)
        }

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
                dirty_count: 1,
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
            })
        }

        fn scan_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err(RepositoryStateError::Watcher("scan failed".to_string()));
            }
            Ok(vec![main_worktree(repo_path)])
        }

        fn prune_stale_branch_bases(&self, repo_path: &str) -> Result<(), RepositoryStateError> {
            self.prunes.lock().push(repo_path.to_string());
            Ok(())
        }
    }

    fn main_worktree(path: &str) -> Worktree {
        Worktree {
            name: "main".to_string(),
            path: path.to_string(),
            branch: "main".to_string(),
            is_main: true,
            is_locked: false,
            is_merged: false,
        }
    }

    fn test_state(scanner: Arc<dyn RepositoryScanner>, debounce: Duration) -> Arc<WorktreeState> {
        WorktreeState::new(
            "/repo".to_string(),
            true,
            scanner,
            crate::test_support::state_subscription::test_subscriptions(),
            Arc::new(TestRepositoryStateWorkerRuntime),
            debounce,
        )
    }

    fn test_state_with_notifier(
        scanner: Arc<dyn RepositoryScanner>,
        subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Arc<WorktreeState> {
        WorktreeState::new(
            "/repo".to_string(),
            true,
            scanner,
            subscriptions,
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
    async fn test_変更状態走査_失敗を差分と未コミット数へ返し回復で解除する() {
        // Given
        let scanner = Arc::new(FakeScanner::new("first.txt"));
        let state = test_state(scanner.clone(), Duration::ZERO);
        state.invalidate(InvalidateReason::change());
        wait_for_version(&state, 1).await;
        // When
        scanner.set_fail(true);
        let result = state
            .scan_once(scanner.clone(), &TestRepositoryStateWorkerRuntime)
            .await;
        state.finish_scan(Some(result), InvalidateReason::change());
        // Then
        assert!(state.read_snapshot().is_err());
        assert!(state.dirty_count().value.is_none());
        assert!(state.dirty_count().error.is_some());
        scanner.set_fail(false);
        let result = state
            .scan_once(scanner, &TestRepositoryStateWorkerRuntime)
            .await;
        state.finish_scan(Some(result), InvalidateReason::change());
        assert!(state.read_snapshot().is_ok());
        assert_eq!(state.dirty_count().value, Some(1));
    }

    #[tokio::test]
    async fn loading_snapshot_becomes_ready_after_scan() {
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        let state = test_state(scanner.clone(), Duration::ZERO);

        let initial = state.snapshot_for_read();
        assert_eq!(initial.version, 0);
        assert!(initial.flags.loading);

        state.invalidate(InvalidateReason::change());
        let ready = wait_for_version(&state, 1).await;

        assert_eq!(ready.version, 1);
        assert!(!ready.flags.loading);
        assert!(!ready.flags.stale);
        assert_eq!(scanner.take_prune_calls(), vec!["/repo".to_string()]);
    }

    #[tokio::test]
    async fn snapshot_is_stale_while_refresh_is_running() {
        let scanner = Arc::new(FakeScanner::new("first.txt").with_sleep(Duration::from_millis(80)));
        let state = test_state(scanner.clone(), Duration::ZERO);

        state.invalidate(InvalidateReason::change());
        let first = wait_for_version(&state, 1).await;
        assert_eq!(first.status[0].path, "first.txt");

        scanner.set_value("second.txt");
        state.invalidate(InvalidateReason::change());
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
    async fn test_スキャン通知_開始時はstaleとloadingにして通知せず正常完了時だけ通知する() {
        // Given
        let scanner = Arc::new(FakeScanner::new("first.txt"));
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let notifier = Arc::new(CapturingNotifier::repositories(&subscriptions));
        let state = test_state_with_notifier(scanner.clone(), subscriptions);
        state.invalidate(InvalidateReason::change());
        wait_for_version(&state, 1).await;
        assert_eq!(notifier.take().len(), 1);

        let (started_tx, started_rx) = std_mpsc::channel();
        let (resume_tx, resume_rx) = std_mpsc::channel();
        let resume_rx = Mutex::new(resume_rx);
        scanner.set_on_scan(move |_| {
            started_tx.send(()).unwrap();
            resume_rx
                .lock()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        });
        scanner.set_value("second.txt");

        // When
        state.invalidate(InvalidateReason::change());
        tokio::task::spawn_blocking(move || started_rx.recv_timeout(Duration::from_secs(5)))
            .await
            .unwrap()
            .unwrap();

        // Then
        let snapshot = state.snapshot_for_read();
        assert_eq!(snapshot.version, 1);
        assert!(snapshot.flags.loading);
        assert!(snapshot.flags.stale);
        assert!(notifier.take().is_empty());

        // When
        resume_tx.send(()).unwrap();
        let snapshot = wait_for_version(&state, 2).await;

        // Then
        assert_eq!(snapshot.version, 2);
        assert_eq!(snapshot.status[0].path, "second.txt");
        assert!(!snapshot.flags.loading);
        assert!(!snapshot.flags.stale);
        let notifications = notifier.take();
        assert_eq!(notifications.len(), 1);
        state.shutdown();
    }

    #[tokio::test]
    async fn debounce_groups_multiple_invalidations_into_one_scan() {
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        let state = test_state(scanner.clone(), Duration::from_millis(40));

        state.invalidate(InvalidateReason::change());
        state.invalidate(InvalidateReason::change());
        state.invalidate(InvalidateReason::change());
        wait_for_version(&state, 1).await;

        assert_eq!(scanner.scan_count(), 1);
    }

    #[tokio::test]
    async fn superseded_scan_result_is_not_committed() {
        let scanner = Arc::new(FakeScanner::new("old.txt").with_sleep(Duration::from_millis(80)));
        let state = test_state(scanner.clone(), Duration::ZERO);

        state.invalidate(InvalidateReason::change());
        wait_for_version(&state, 1).await;

        let (tx, rx) = std_mpsc::channel();
        scanner.set_on_scan(move |call| {
            if call == 2 {
                tx.send(()).unwrap();
            }
        });

        state.invalidate(InvalidateReason::change());
        tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(1)))
            .await
            .unwrap()
            .unwrap();
        scanner.set_value("new.txt");
        state.invalidate(InvalidateReason::change());

        let latest = wait_for_version(&state, 2).await;
        assert_eq!(latest.status[0].path, "new.txt");
        assert!(scanner.scan_count() >= 3);
    }

    #[tokio::test]
    async fn failed_scan_is_not_pruned_or_committed() {
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        scanner.set_fail(true);
        let state = test_state(scanner.clone(), Duration::ZERO);

        state.invalidate(InvalidateReason::change());
        tokio::time::sleep(Duration::from_millis(30)).await;

        let snapshot = state.snapshot_for_read();
        assert_eq!(snapshot.version, 0);
        assert!(!snapshot.flags.loading);
        assert!(scanner.take_prune_calls().is_empty());
    }

    #[tokio::test]
    async fn test_worktreeの並び_読み直しに失敗しても最後に読めた並びを残す() {
        // Given
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        let state = test_state(scanner.clone(), Duration::ZERO);
        state.invalidate(InvalidateReason::change());
        wait_for_version(&state, 1).await;
        assert_eq!(
            state.worktrees(),
            Fetched::ready(vec![main_worktree("/repo")])
        );

        // When
        scanner.set_fail(true);
        state.invalidate(InvalidateReason::refs());
        for _ in 0..100 {
            if state.worktrees().error.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Then
        let worktrees = state.worktrees();
        assert_eq!(worktrees.value, Some(vec![main_worktree("/repo")]));
        assert!(worktrees.error.is_some());
        assert_eq!(scanner.scan_count(), 1);
    }

    #[tokio::test]
    async fn test_worktreeの並び_repositoryのrootでなければ読まない() {
        // Given
        let scanner = Arc::new(FakeScanner::new("file.txt"));
        let state = WorktreeState::new(
            "/repo-worktrees/feature".to_string(),
            false,
            scanner.clone(),
            crate::test_support::state_subscription::test_subscriptions(),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Duration::ZERO,
        );

        // When
        state.invalidate(InvalidateReason::change());
        wait_for_version(&state, 1).await;

        // Then
        assert_eq!(state.worktrees(), Fetched::default());
        assert!(scanner.take_prune_calls().is_empty());
    }
}
