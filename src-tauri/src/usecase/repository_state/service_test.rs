use super::*;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::{
    runtime::tests_support::{IdentityWorktreePathNormalizer, TestRepositoryStateWorkerRuntime},
    snapshot::RepositorySnapshotParts,
    worktree::NoopRepositoryStateWatcher,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Scanner {
    calls: AtomicUsize,
    worktree_calls: AtomicUsize,
    fail: AtomicBool,
    retry_failures: AtomicUsize,
    on_scan: parking_lot::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
#[async_trait::async_trait]
impl RepositoryScanner for Scanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scan(repo_path)
    }

    fn scan(&self, _: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_scan.lock().as_ref() {
            hook();
        }
        if self
            .retry_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            return Err(RepositoryStateError::ScanInvalidated);
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err(RepositoryStateError::Watcher("scan failed".into()));
        }
        Ok(RepositorySnapshotParts {
            status: vec![FileStatusDto {
                path: format!("scan-{call}"),
                index_status: "none".into(),
                worktree_status: "modified".into(),
            }],
            diff_stats: vec![FileDiffStatDto {
                path: format!("scan-{call}"),
                index_additions: 0,
                index_deletions: 0,
                wt_additions: call as u32,
                wt_deletions: 0,
            }],
            dirty_count: call,
            diff_file_tree: vec![],
            staged_diff_file_tree: vec![],
            changes_diff_file_tree: vec![],
        })
    }

    fn scan_worktrees(&self, repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
        let call = self.worktree_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(RepositoryStateError::Watcher("scan failed".into()));
        }
        Ok(vec![Worktree {
            name: format!("scan-{call}"),
            path: repo_path.into(),
            branch: format!("scan-{call}"),
            is_main: true,
            is_locked: false,
            is_merged: false,
        }])
    }

    fn prune_stale_branch_bases(&self, _: &str) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}
struct Repository;
impl RepositoryStateRepository for Repository {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        Ok(path.into())
    }
}
struct Notifier {
    subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    changes: tokio::sync::Mutex<
        tokio::sync::broadcast::Receiver<crate::usecase::state_subscription::StateChangeSource>,
    >,
    notifications: AtomicUsize,
}
impl Default for Notifier {
    fn default() -> Self {
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let changes = tokio::sync::Mutex::new(subscriptions.changes());
        Self {
            subscriptions,
            changes,
            notifications: AtomicUsize::new(0),
        }
    }
}
impl Notifier {
    async fn wait(&self) {
        self.changes.lock().await.recv().await.unwrap();
        self.notifications.fetch_add(1, Ordering::SeqCst);
    }
    fn count(&self) -> usize {
        if let Ok(mut changes) = self.changes.try_lock() {
            self.notifications.fetch_add(
                crate::test_support::state_subscription::take_changes(&mut changes).len(),
                Ordering::SeqCst,
            );
        }
        self.notifications.load(Ordering::SeqCst)
    }
}
fn service(scanner: Arc<Scanner>, notifier: Arc<Notifier>) -> RepositoryStateService {
    RepositoryStateService::new(
        Arc::new(Repository),
        scanner,
        notifier.subscriptions.clone(),
        Arc::new(NoopRepositoryStateWatcher),
        Arc::new(TestRepositoryStateWorkerRuntime),
        Arc::new(IdentityWorktreePathNormalizer),
    )
    .with_debounce(Duration::ZERO)
}

fn branch(service: &RepositoryStateService) -> String {
    service.worktrees("/repo").value.unwrap()[0].branch.clone()
}

#[tokio::test]
async fn test_一覧の再走査_監視中も保存済みの結果を使わず毎回走査する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = service(scanner.clone(), notifier.clone());
    service.start_git_dir_watching("/repo").unwrap();
    notifier.wait().await;
    let previous = branch(&service);
    service.rescan("/repo").await.unwrap();
    let next = branch(&service);
    let next_snapshot = service.get_snapshot("/repo").unwrap();
    // When
    service.rescan("/repo").await.unwrap();
    let latest = branch(&service);
    // Then
    assert_ne!(previous, next);
    assert_ne!(next, latest);
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 3);
    assert_eq!(scanner.worktree_calls.load(Ordering::SeqCst), 3);
    assert_eq!(notifier.count(), 3);
    let snapshot = service.get_snapshot("/repo").unwrap();
    assert_eq!(snapshot.status[0].path, "scan-2");
    assert_eq!(service.dirty_count("/repo").value, Some(2));
    assert_eq!(snapshot.version, next_snapshot.version + 1);
    scanner.fail.store(true, Ordering::SeqCst);
    service.rescan("/repo").await.unwrap();
    let worktrees = service.worktrees("/repo");
    assert!(worktrees.error.is_some());
    assert_eq!(worktrees.value.unwrap()[0].branch, latest);
    assert!(service.get_snapshot("/repo").is_err());
    assert!(service.dirty_count("/repo").error.is_some());
}

#[tokio::test]
async fn test_走査失敗_自動更新へ通知し再起動せず明示的な再走査で復旧する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    scanner.fail.store(true, Ordering::SeqCst);
    let notifier = Arc::new(Notifier::default());
    let service = service(scanner.clone(), notifier.clone());
    service.start_git_dir_watching("/repo").unwrap();
    // When
    tokio::time::timeout(Duration::from_secs(2), notifier.wait())
        .await
        .unwrap();
    // Then
    assert_eq!(notifier.count(), 1);
    service.rescan("/repo").await.unwrap();
    let failed = service.worktrees("/repo");
    assert!(!failed.loaded());
    assert!(failed.error.is_some());
    // When
    scanner.fail.store(false, Ordering::SeqCst);
    service.rescan("/repo").await.unwrap();
    // Then
    let recovered = service.worktrees("/repo");
    assert_eq!(recovered.value.map(|worktrees| worktrees.len()), Some(1));
    assert!(recovered.error.is_none());
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_監視走査との競合_共有snapshotへのcommitを直列化する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = Arc::new(service(scanner.clone(), notifier.clone()));
    service.start_git_dir_watching("/repo").unwrap();
    notifier.wait().await;
    let started = Arc::new(tokio::sync::Notify::new());
    let (release, receive) = std::sync::mpsc::channel();
    let receive = parking_lot::Mutex::new(receive);
    let once = AtomicBool::new(false);
    *scanner.on_scan.lock() = Some(Box::new({
        let started = started.clone();
        move || {
            if !once.swap(true, Ordering::SeqCst) {
                started.notify_one();
                receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
    }));
    let state = service.ensure_watching("/repo").unwrap();
    state.invalidate(super::super::worker::InvalidateReason::change());
    started.notified().await;
    // When
    let refresh = {
        let service = service.clone();
        tokio::spawn(async move { service.rescan("/repo").await.unwrap() })
    };
    tokio::task::yield_now().await;
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 2);
    release.send(()).unwrap();
    refresh.await.unwrap();
    // Then
    let snapshot = service.get_snapshot("/repo").unwrap();
    assert_eq!(snapshot.status[0].path, "scan-2");
    assert_eq!(snapshot.version, 3);
    for _ in 0..2 {
        tokio::time::timeout(Duration::from_secs(2), notifier.wait())
            .await
            .unwrap();
    }
    assert_eq!(notifier.count(), 3);
}

#[tokio::test]
async fn test_明示再走査_途中で失効した結果を公開せず再走査する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = Arc::new(service(scanner.clone(), notifier.clone()));
    service.start_git_dir_watching("/repo").unwrap();
    notifier.wait().await;
    let previous = service.get_snapshot("/repo").unwrap();
    let state = service.ensure_watching("/repo").unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let (release, receive) = std::sync::mpsc::channel();
    let receive = parking_lot::Mutex::new(receive);
    let calls = AtomicUsize::new(0);
    *scanner.on_scan.lock() = Some(Box::new({
        let started = started.clone();
        move || {
            if calls.fetch_add(1, Ordering::SeqCst) < 2 {
                started.notify_one();
                receive.lock().recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
    }));
    // When
    let refresh = {
        let service = service.clone();
        tokio::spawn(async move { service.rescan("/repo").await.unwrap() })
    };
    started.notified().await;
    state.invalidate(super::super::worker::InvalidateReason::change());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    // Then
    assert!(Arc::ptr_eq(
        &previous,
        &service.get_snapshot("/repo").unwrap()
    ));
    assert_eq!(notifier.count(), 1);
    // When
    release.send(()).unwrap();
    refresh.await.unwrap();
    // Then
    let snapshot = service.get_snapshot("/repo").unwrap();
    assert!(snapshot.version > previous.version);
    assert_ne!(snapshot.status[0].path, "scan-1");
}

#[tokio::test]
async fn test_snapshot公開_失効世代はversionと前回情報を変更しない() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = service(scanner.clone(), notifier);
    let state = service.ensure_for_tests("/repo");
    let generation = state.requested_generation();
    let previous = state
        .commit_snapshot(scanner.scan("/repo").unwrap(), generation)
        .unwrap();
    let obsolete = scanner.scan("/repo").unwrap();
    // When
    let _scan = state.scan_lock.lock().await;
    state.invalidate(super::super::worker::InvalidateReason::change());
    let result = state.commit_snapshot(obsolete, generation);
    // Then
    assert!(result.is_none());
    assert!(Arc::ptr_eq(&previous, &state.snapshot_for_read()));
    let current = state
        .commit_snapshot(scanner.scan("/repo").unwrap(), state.requested_generation())
        .unwrap();
    assert_eq!(current.version, previous.version + 1);
    assert_eq!(current.status[0].path, "scan-2");
}

#[tokio::test]
async fn test_明示再走査_監視していないrepositoryは走査せず登録しない() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let service = service(scanner.clone(), Arc::new(Notifier::default()));
    // When
    for path in ["/a", "/b", "/c"] {
        service.rescan(path).await.unwrap();
    }
    // Then
    assert_eq!(service.worktree_count(), 0);
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(scanner.worktree_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn test_明示再走査_失効が続くと終了してscanロックを解放する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let service = service(scanner.clone(), Arc::new(Notifier::default()));
    let state = service.ensure_for_tests("/repo");
    let previous = state
        .commit_snapshot(scanner.scan("/repo").unwrap(), 0)
        .unwrap();
    *scanner.on_scan.lock() = Some(Box::new({
        let state = state.clone();
        move || state.invalidate(super::super::worker::InvalidateReason::change())
    }));
    // When
    let result = tokio::time::timeout(Duration::from_secs(2), service.rescan_status(&state))
        .await
        .unwrap();
    // Then
    assert!(matches!(result, Err(RepositoryStateError::ScanInvalidated)));
    let scan = tokio::time::timeout(Duration::from_secs(2), state.scan_lock.lock())
        .await
        .unwrap();
    assert_eq!(state.snapshot_for_read().version, previous.version);
    *scanner.on_scan.lock() = None;
    drop(scan);
    assert!(service.rescan_status(&state).await.is_ok());
}

#[tokio::test]
async fn test_背景走査_開始失敗を再試行中も明示再走査が走査できる() {
    let scanner = Arc::new(Scanner::default());
    scanner.retry_failures.store(usize::MAX, Ordering::SeqCst);
    let notifier = Arc::new(Notifier::default());
    let service = service(scanner.clone(), notifier.clone());
    service.start_git_dir_watching("/repo").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while scanner.calls.load(Ordering::SeqCst) < 3 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let calls = scanner.calls.load(Ordering::SeqCst);
    let state = service.ensure_watching("/repo").unwrap();
    let result = tokio::time::timeout(Duration::from_millis(100), service.rescan_status(&state))
        .await
        .expect("backoff must release scan_lock");
    assert!(matches!(result, Err(RepositoryStateError::ScanInvalidated)));
    assert!(scanner.calls.load(Ordering::SeqCst) > calls);
    scanner.retry_failures.store(0, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(2), notifier.wait())
        .await
        .unwrap();
    assert_eq!(service.get_snapshot("/repo").unwrap().status.len(), 1);
}
