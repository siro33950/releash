use super::fixture_helpers::EmptyScanner;
use super::*;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime;
use crate::usecase::repository_state::runtime::tests_support::{
    IdentityWorktreePathNormalizer, NoSpawnRepositoryStateWorkerRuntime,
};
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;
struct TestRepositoryStateRepository;

impl RepositoryStateRepository for TestRepositoryStateRepository {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        Ok(path.to_string())
    }
}

#[derive(Default)]
struct CountingScanner {
    scans: AtomicUsize,
    prunes: parking_lot::Mutex<Vec<String>>,
    status: parking_lot::Mutex<Vec<FileStatusDto>>,
    diff_stats: parking_lot::Mutex<Vec<FileDiffStatDto>>,
    worktrees: parking_lot::Mutex<Vec<Worktree>>,
}

#[async_trait::async_trait]

impl RepositoryScanner for CountingScanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scan(repo_path)
    }

    fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        let status = self.status.lock().clone();
        Ok(RepositorySnapshotParts {
            dirty_count: status.len(),
            status,
            diff_stats: self.diff_stats.lock().clone(),
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn scan_worktrees(&self, _repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
        Ok(self.worktrees.lock().clone())
    }

    fn prune_stale_branch_bases(&self, repo_path: &str) -> Result<(), RepositoryStateError> {
        self.prunes.lock().push(repo_path.to_string());
        Ok(())
    }
}

struct GateWatchSession {
    drop_entered: Arc<std::sync::atomic::AtomicBool>,
    release: Arc<std::sync::Barrier>,
}

impl Drop for GateWatchSession {
    fn drop(&mut self) {
        self.drop_entered.store(true, Ordering::SeqCst);
        self.release.wait();
    }
}

struct GateWatcher {
    next_id: AtomicU64,
    drop_entered: Arc<std::sync::atomic::AtomicBool>,
    release: Arc<std::sync::Barrier>,
}

impl RepositoryStateWatcher for GateWatcher {
    fn next_watcher_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn start_watchers(
        &self,
        _state: Arc<WorktreeState>,
    ) -> Result<
        Box<dyn crate::usecase::repository_state::worktree::RepositoryStateWatchSession>,
        RepositoryStateError,
    > {
        Ok(Box::new(GateWatchSession {
            drop_entered: self.drop_entered.clone(),
            release: self.release.clone(),
        }))
    }
}

struct BlockingStartWatcher {
    next_id: AtomicU64,
    start_entered: Arc<AtomicUsize>,
    release: Arc<std::sync::Barrier>,
}

impl RepositoryStateWatcher for BlockingStartWatcher {
    fn next_watcher_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn start_watchers(
        &self,
        _state: Arc<WorktreeState>,
    ) -> Result<
        Box<dyn crate::usecase::repository_state::worktree::RepositoryStateWatchSession>,
        RepositoryStateError,
    > {
        self.start_entered.fetch_add(1, Ordering::SeqCst);
        self.release.wait();
        Ok(Box::new(()))
    }
}

fn no_spawn_service(watcher: Arc<dyn RepositoryStateWatcher>) -> Arc<RepositoryStateService> {
    Arc::new(RepositoryStateService::new(
        Arc::new(TestRepositoryStateRepository),
        Arc::new(CountingScanner::default()),
        crate::test_support::state_subscription::test_subscriptions(),
        watcher,
        Arc::new(NoSpawnRepositoryStateWorkerRuntime),
        Arc::new(IdentityWorktreePathNormalizer),
        crate::test_support::state_subscription::repository_driver(),
    ))
}

#[test]
fn stop_watching_releases_worktrees_lock_before_watch_session_drop() {
    let drop_entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let release = Arc::new(std::sync::Barrier::new(2));
    let service = no_spawn_service(Arc::new(GateWatcher {
        next_id: AtomicU64::new(0),
        drop_entered: drop_entered.clone(),
        release: release.clone(),
    }));
    let id = service.subscribe("/repo").unwrap();

    let stopper = {
        let service = service.clone();
        std::thread::spawn(move || service.stop_watching(id).unwrap())
    };
    while !drop_entered.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }

    let (probe_tx, probe_rx) = std::sync::mpsc::channel();
    {
        let service = service.clone();
        std::thread::spawn(move || {
            let _ = probe_tx.send(service.get_snapshot("/repo").is_ok());
        });
    }
    let probe = probe_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("watch session の drop 中も worktrees ロックが解放されていること");

    release.wait();
    assert!(probe);
    assert!(stopper.join().unwrap());
    assert_eq!(service.worktree_count(), 0);
}

#[test]
fn subscribe_starts_watchers_outside_worktrees_lock() {
    let start_entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(std::sync::Barrier::new(2));
    let service = no_spawn_service(Arc::new(BlockingStartWatcher {
        next_id: AtomicU64::new(0),
        start_entered: start_entered.clone(),
        release: release.clone(),
    }));

    let subscriber = {
        let service = service.clone();
        std::thread::spawn(move || service.subscribe("/blocked").unwrap())
    };
    while start_entered.load(Ordering::SeqCst) == 0 {
        std::thread::yield_now();
    }

    let (probe_tx, probe_rx) = std::sync::mpsc::channel();
    {
        let service = service.clone();
        std::thread::spawn(move || {
            let _ = probe_tx.send(service.get_snapshot("/other").is_ok());
        });
    }
    let probe = probe_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("start_watchers 中も worktrees ロックが解放されていること");

    release.wait();
    assert!(probe);
    subscriber.join().unwrap();
    assert_eq!(service.worktree_count(), 1);
}

#[test]
fn concurrent_subscribe_for_same_worktree_converges_to_single_state() {
    let start_entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(std::sync::Barrier::new(3));
    let service = no_spawn_service(Arc::new(BlockingStartWatcher {
        next_id: AtomicU64::new(0),
        start_entered: start_entered.clone(),
        release: release.clone(),
    }));

    let first = {
        let service = service.clone();
        std::thread::spawn(move || service.subscribe("/repo").unwrap())
    };
    let second = {
        let service = service.clone();
        std::thread::spawn(move || service.subscribe("/repo").unwrap())
    };
    while start_entered.load(Ordering::SeqCst) < 2 {
        std::thread::yield_now();
    }
    release.wait();

    let first_id = first.join().unwrap();
    let second_id = second.join().unwrap();

    assert_eq!(service.worktree_count(), 1);
    assert!(service.stop_watching(first_id).unwrap());
    assert_eq!(service.worktree_count(), 1);
    assert!(service.stop_watching(second_id).unwrap());
    assert_eq!(service.worktree_count(), 0);
}

#[test]
fn subscribe_failure_does_not_register_worktree() {
    struct FailingWatcher {
        next_id: AtomicU64,
    }

    impl RepositoryStateWatcher for FailingWatcher {
        fn next_watcher_id(&self) -> u64 {
            self.next_id.fetch_add(1, Ordering::SeqCst) + 1
        }

        fn start_watchers(
            &self,
            _state: Arc<WorktreeState>,
        ) -> Result<
            Box<dyn crate::usecase::repository_state::worktree::RepositoryStateWatchSession>,
            RepositoryStateError,
        > {
            Err(RepositoryStateError::Watcher("start failed".to_string()))
        }
    }

    let service = no_spawn_service(Arc::new(FailingWatcher {
        next_id: AtomicU64::new(0),
    }));

    assert!(service.subscribe("/repo").is_err());
    assert_eq!(service.worktree_count(), 0);
}

#[tokio::test]
async fn same_worktree_reuses_one_state() {
    let service = test_service(Arc::new(EmptyScanner));

    let first = service.ensure_for_tests("/repo");
    let second = service.ensure_for_tests("/repo");

    assert_eq!(service.worktree_count(), 1);
    assert!(Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn multiple_worktrees_are_independent_entries() {
    let service = test_service(Arc::new(EmptyScanner));

    let first = service.ensure_for_tests("/repo-one");
    let second = service.ensure_for_tests("/repo-two");

    assert_eq!(service.worktree_count(), 2);
    assert!(!Arc::ptr_eq(&first, &second));
}

mod rescan_tests {
    use crate::domain::repository::entities::worktree::Worktree;
    use crate::usecase::repository_dto::FileDiffStatDto;
    use crate::usecase::repository_dto::FileStatusDto;
    use crate::usecase::repository_state::error::RepositoryStateError;
    use crate::usecase::repository_state::runtime::test_helpers_runtime::tests_support::IdentityWorktreePathNormalizer;
    use crate::usecase::repository_state::runtime::test_helpers_runtime::tests_support::TestRepositoryStateWorkerRuntime;
    use crate::usecase::repository_state::scanner::RepositoryScanner;
    use crate::usecase::repository_state::service::RepositoryStateRepository;
    use crate::usecase::repository_state::service::RepositoryStateService;
    use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
    use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

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
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
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
            tokio::sync::broadcast::Receiver<
                crate::usecase::state_subscription::target::StateChangeSource,
            >,
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
            crate::test_support::state_subscription::repository_driver(),
        )
    }

    fn branch(service: &RepositoryStateService) -> String {
        service.worktrees("/repo").value.unwrap()[0].branch.clone()
    }

    #[tokio::test]
    pub async fn test_一覧の再走査_監視中も保存済みの結果を使わず毎回走査する() {
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
    }

    #[tokio::test]
    pub async fn test_一覧の再走査_取得後の失敗で前の一覧と失敗を返す() {
        // Given
        let scanner = Arc::new(Scanner::default());
        let notifier = Arc::new(Notifier::default());
        let service = service(scanner.clone(), notifier.clone());
        service.start_git_dir_watching("/repo").unwrap();
        notifier.wait().await;
        service.rescan("/repo").await.unwrap();
        service.rescan("/repo").await.unwrap();
        let latest = branch(&service);
        // When
        scanner.fail.store(true, Ordering::SeqCst);
        service.rescan("/repo").await.unwrap();
        // Then
        let worktrees = service.worktrees("/repo");
        assert!(worktrees.error.is_some());
        assert_eq!(worktrees.value.unwrap()[0].branch, latest);
        assert!(service.get_snapshot("/repo").is_err());
        assert!(service.dirty_count("/repo").error.is_some());
    }

    #[tokio::test]
    pub async fn test_走査失敗_自動更新へ通知し未取得と失敗を返す() {
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
        let initial_count = notifier.count();
        service.rescan("/repo").await.unwrap();
        // Then
        assert_eq!(initial_count, 1);
        assert_eq!(notifier.count(), 2);
        let failed = service.worktrees("/repo");
        assert!(!failed.loaded());
        assert!(failed.error.is_some());
    }

    #[tokio::test]
    pub async fn test_走査失敗_再起動せず明示的な再走査で復旧する() {
        // Given
        let scanner = Arc::new(Scanner::default());
        scanner.fail.store(true, Ordering::SeqCst);
        let notifier = Arc::new(Notifier::default());
        let service = service(scanner.clone(), notifier.clone());
        service.start_git_dir_watching("/repo").unwrap();
        tokio::time::timeout(Duration::from_secs(2), notifier.wait())
            .await
            .unwrap();
        service.rescan("/repo").await.unwrap();
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
    pub async fn test_監視走査との競合_共有snapshotへのcommitを直列化する() {
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
        state.invalidate(crate::usecase::repository_state::worker::InvalidateReason::change());
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
    pub async fn test_明示再走査_途中で失効した結果を公開せず再走査する() {
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
        state.invalidate(crate::usecase::repository_state::worker::InvalidateReason::change());
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
    pub async fn test_snapshot公開_失効世代はversionと前回情報を変更しない() {
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
        let _scan = state.test_lock_scan().await;
        state.invalidate(crate::usecase::repository_state::worker::InvalidateReason::change());
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
    pub async fn test_明示再走査_監視していないrepositoryは走査せず登録しない() {
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
    pub async fn test_明示再走査_失効が続くと終了してscanロックを解放する() {
        // Given
        let scanner = Arc::new(Scanner::default());
        let service = service(scanner.clone(), Arc::new(Notifier::default()));
        let state = service.ensure_for_tests("/repo");
        let previous = state
            .commit_snapshot(scanner.scan("/repo").unwrap(), 0)
            .unwrap();
        *scanner.on_scan.lock() = Some(Box::new({
            let state = state.clone();
            move || {
                state.invalidate(
                    crate::usecase::repository_state::worker::InvalidateReason::change(),
                )
            }
        }));
        // When
        let result = tokio::time::timeout(Duration::from_secs(2), service.rescan_status(&state))
            .await
            .unwrap();
        // Then
        assert!(matches!(result, Err(RepositoryStateError::ScanInvalidated)));
        let scan = tokio::time::timeout(Duration::from_secs(2), state.test_lock_scan())
            .await
            .unwrap();
        assert_eq!(state.snapshot_for_read().version, previous.version);
        *scanner.on_scan.lock() = None;
        drop(scan);
        assert!(service.rescan_status(&state).await.is_ok());
    }

    #[tokio::test]
    pub async fn test_背景走査_開始失敗を再試行中も明示再走査が走査できる() {
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
        let result =
            tokio::time::timeout(Duration::from_millis(100), service.rescan_status(&state))
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
}

mod fixture_memory_tests {
    use super::super::*;
    use crate::usecase::repository_state::runtime::test_helpers_runtime::tests_support::{
        IdentityWorktreePathNormalizer, TestRepositoryStateWorkerRuntime,
    };
    use crate::usecase::repository_state::service::fixture_helpers::*;
    use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
    use crate::usecase::repository_state::worker::InvalidateReason;
    use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    pub async fn file_change_in_one_worktree_does_not_update_other_worktree_snapshot_version() {
        let scanner = Arc::new(CountingScanner::default());
        let service = counting_service(scanner, Arc::new(NoopRepositoryStateWatcher));

        let first = service.ensure_for_tests("/repo-one");
        let second = service.ensure_for_tests("/repo-two");
        first.commit_snapshot(
            RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                dirty_count: 0,
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
            },
            0,
        );
        second.commit_snapshot(
            RepositorySnapshotParts {
                status: Vec::new(),
                diff_stats: Vec::new(),
                dirty_count: 0,
                diff_file_tree: Vec::new(),
                staged_diff_file_tree: Vec::new(),
                changes_diff_file_tree: Vec::new(),
            },
            0,
        );

        first.invalidate(InvalidateReason::change());

        for _ in 0..100 {
            if first.snapshot_for_read().version >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(first.snapshot_for_read().version >= 2);
        assert_eq!(second.snapshot_for_read().version, 1);
    }

    #[tokio::test]
    pub async fn test_worktreeの並び_監視していないrepositoryは読まず掃除もしない() {
        // Given
        let scanner = Arc::new(CountingScanner::default());
        scanner.set_worktrees(vec![worktree("/repo", "main", true)]);
        let service = counting_service(scanner.clone(), Arc::new(NoopRepositoryStateWatcher));

        // When
        let worktrees = service.worktrees("/repo");
        let dirty_count = service.dirty_count("/repo");

        // Then
        assert_eq!(worktrees, Fetched::default());
        assert_eq!(dirty_count, Fetched::default());
        assert_eq!(scanner.scan_count(), 0);
        assert!(scanner.prune_calls().is_empty());
    }

    #[tokio::test]
    pub async fn test_worktreeの並び_linked_worktreeのpathからもrootの並びを一度の解決で読む() {
        // Given
        let scanner = Arc::new(CountingScanner::default());
        scanner.set_worktrees(vec![
            worktree("/repo", "main", true),
            worktree("/repo-worktrees/feature", "feature", false),
        ]);
        let repository = Arc::new(LinkedRepositoryStateRepository::default());
        let service = RepositoryStateService::new(
            repository.clone(),
            scanner.clone(),
            crate::test_support::state_subscription::test_subscriptions(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(IdentityWorktreePathNormalizer),
            crate::test_support::state_subscription::repository_driver(),
        );
        service.subscribe("/repo-worktrees/feature").unwrap();
        service.subscribe("/repo").unwrap();
        for _ in 0..100 {
            if service.worktrees("/repo").loaded() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let resolutions = repository.resolutions.load(Ordering::SeqCst);

        // When
        let from_linked = service.worktrees("/repo-worktrees/feature");
        let from_root = service.worktrees("/repo");

        // Then
        assert_eq!(from_linked.value.as_ref().map(Vec::len), Some(2));
        assert_eq!(from_linked, from_root);
        assert_eq!(
            service.repository_root("/repo-worktrees/feature").unwrap(),
            "/repo"
        );
        assert_eq!(repository.resolutions.load(Ordering::SeqCst), resolutions);
        assert_eq!(scanner.prune_calls(), vec!["/repo".to_string()]);
    }
}

fn test_service(scanner: Arc<EmptyScanner>) -> RepositoryStateService {
    RepositoryStateService::new(
        Arc::new(TestRepositoryStateRepository),
        scanner,
        crate::test_support::state_subscription::test_subscriptions(),
        Arc::new(NoopRepositoryStateWatcher),
        Arc::new(TestRepositoryStateWorkerRuntime),
        Arc::new(IdentityWorktreePathNormalizer),
        crate::test_support::state_subscription::repository_driver(),
    )
}
