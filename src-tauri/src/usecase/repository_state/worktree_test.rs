use super::*;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

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
        crate::test_support::state_subscription::scan_driver(debounce),
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
        crate::test_support::state_subscription::scan_driver(Duration::ZERO),
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
async fn test_変更状態走査_失敗を差分と未コミット数へ返す() {
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
    assert_eq!(state.dirty_count().value, Some(1));
    assert!(state.dirty_count().error.is_some());
}

#[tokio::test]
async fn test_変更状態走査_回復で失敗を解除する() {
    // Given
    let scanner = Arc::new(FakeScanner::new("first.txt"));
    let state = test_state(scanner.clone(), Duration::ZERO);
    state.invalidate(InvalidateReason::change());
    wait_for_version(&state, 1).await;
    scanner.set_fail(true);
    let result = state
        .scan_once(scanner.clone(), &TestRepositoryStateWorkerRuntime)
        .await;
    state.finish_scan(Some(result), InvalidateReason::change());
    // When
    scanner.set_fail(false);
    let result = state
        .scan_once(scanner, &TestRepositoryStateWorkerRuntime)
        .await;
    state.finish_scan(Some(result), InvalidateReason::change());
    // Then
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
        crate::test_support::state_subscription::scan_driver(Duration::ZERO),
    );

    // When
    state.invalidate(InvalidateReason::change());
    wait_for_version(&state, 1).await;

    // Then
    assert_eq!(state.worktrees(), Fetched::default());
    assert!(scanner.take_prune_calls().is_empty());
}
