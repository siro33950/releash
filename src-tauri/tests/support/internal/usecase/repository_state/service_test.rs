use super::*;
use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
use crate::usecase::repository_state::{
    runtime::tests_support::{IdentityWorktreePathNormalizer, TestRepositoryStateWorkerRuntime},
    snapshot::RepositorySnapshotParts,
    worktree::NoopRepositoryStateWatcher,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        move || state.invalidate(super::super::worker::InvalidateReason::change())
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
pub(crate) mod tests {
    use super::super::*;
    use crate::usecase::repository_dto::{FileDiffStatDto, FileStatusDto};
    use crate::usecase::repository_state::runtime::tests_support::{
        CanonicalWorktreePathNormalizer, IdentityWorktreePathNormalizer,
        TestRepositoryStateWorkerRuntime,
    };
    use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
    use crate::usecase::repository_state::worker::InvalidateReason;
    use crate::usecase::repository_state::worktree::NoopRepositoryStateWatcher;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    struct TestRepositoryStateRepository;

    impl RepositoryStateRepository for TestRepositoryStateRepository {
        fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
            Ok(path.to_string())
        }
    }

    /// `/repo-worktrees/` 以下を `/repo` の linked worktree として解決し、解決の回数を数える。
    #[derive(Default)]
    struct LinkedRepositoryStateRepository {
        resolutions: AtomicUsize,
    }

    impl RepositoryStateRepository for LinkedRepositoryStateRepository {
        fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
            self.resolutions.fetch_add(1, Ordering::SeqCst);
            Ok(if path.starts_with("/repo-worktrees/") {
                "/repo".to_string()
            } else {
                path.to_string()
            })
        }
    }

    fn worktree(path: &str, branch: &str, is_main: bool) -> Worktree {
        Worktree {
            name: branch.to_string(),
            path: path.to_string(),
            branch: branch.to_string(),
            is_main,
            is_locked: false,
            is_merged: false,
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

        fn scan_worktrees(&self, _repo_path: &str) -> Result<Vec<Worktree>, RepositoryStateError> {
            Ok(Vec::new())
        }

        fn prune_stale_branch_bases(&self, _repo_path: &str) -> Result<(), RepositoryStateError> {
            Ok(())
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

    impl CountingScanner {
        fn with_status(status: Vec<FileStatusDto>) -> Self {
            Self {
                status: parking_lot::Mutex::new(status),
                ..Self::default()
            }
        }

        fn scan_count(&self) -> usize {
            self.scans.load(Ordering::SeqCst)
        }

        fn prune_calls(&self) -> Vec<String> {
            self.prunes.lock().clone()
        }

        fn set_worktrees(&self, worktrees: Vec<Worktree>) {
            *self.worktrees.lock() = worktrees;
        }
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

    #[derive(Default)]
    struct CountingRepositoryStateWatcher {
        next_id: AtomicU64,
        started_paths: parking_lot::Mutex<Vec<String>>,
    }

    impl CountingRepositoryStateWatcher {
        fn start_count(&self) -> usize {
            self.started_paths.lock().len()
        }

        fn started_paths(&self) -> Vec<String> {
            self.started_paths.lock().clone()
        }
    }

    impl RepositoryStateWatcher for CountingRepositoryStateWatcher {
        fn next_watcher_id(&self) -> u64 {
            self.next_id.fetch_add(1, Ordering::SeqCst) + 1
        }

        fn start_watchers(
            &self,
            state: Arc<WorktreeState>,
        ) -> Result<
            Box<dyn crate::usecase::repository_state::worktree::RepositoryStateWatchSession>,
            RepositoryStateError,
        > {
            self.started_paths
                .lock()
                .push(state.worktree_path().to_string());
            Ok(Box::new(()))
        }
    }

    pub(crate) fn watching_service() -> RepositoryStateService {
        test_service(Arc::new(EmptyScanner))
    }

    fn test_service(scanner: Arc<EmptyScanner>) -> RepositoryStateService {
        RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            crate::test_support::state_subscription::test_subscriptions(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            crate::test_support::state_subscription::repository_driver(),
        )
    }

    fn test_service_with_notifier(
        scanner: Arc<EmptyScanner>,
        subscriptions: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> RepositoryStateService {
        RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            subscriptions,
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(CanonicalWorktreePathNormalizer),
            crate::test_support::state_subscription::repository_driver(),
        )
    }

    fn counting_service(
        scanner: Arc<CountingScanner>,
        watcher: Arc<dyn RepositoryStateWatcher>,
    ) -> RepositoryStateService {
        RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            crate::test_support::state_subscription::test_subscriptions(),
            watcher,
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(IdentityWorktreePathNormalizer),
            crate::test_support::state_subscription::repository_driver(),
        )
    }

    use crate::test_support::state_subscription::CapturingNotifier;

    #[tokio::test]
    pub async fn unmanaged_read_returns_ephemeral_snapshot_without_creating_worktree_or_watcher() {
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "changed.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher.clone());
        let dir = tempfile::TempDir::new().unwrap();

        let snapshot = service.get_snapshot(dir.path().to_str().unwrap()).unwrap();

        assert_eq!(snapshot.version, 0);
        assert!(!snapshot.flags.loading);
        assert_eq!(snapshot.status.len(), 1);
        assert_eq!(scanner.scan_count(), 1);
        assert_eq!(service.worktree_count(), 0);
        assert_eq!(watcher.start_count(), 0);
    }

    #[tokio::test]
    pub async fn unmanaged_reads_for_distinct_paths_do_not_accumulate_workers_or_watchers() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher.clone());
        let dirs = [
            tempfile::TempDir::new().unwrap(),
            tempfile::TempDir::new().unwrap(),
            tempfile::TempDir::new().unwrap(),
        ];

        for dir in &dirs {
            service.get_snapshot(dir.path().to_str().unwrap()).unwrap();
        }

        assert_eq!(scanner.scan_count(), 3);
        assert_eq!(service.worktree_count(), 0);
        assert_eq!(watcher.start_count(), 0);
    }

    #[tokio::test]
    pub async fn managed_worktree_read_uses_cached_snapshot_without_rescanning() {
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "worker.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner.clone(), watcher);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service.subscribe(path).unwrap();
        for _ in 0..100 {
            if service.get_snapshot(path).unwrap().version >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let scan_count = scanner.scan_count();

        let snapshot = service.get_snapshot(path).unwrap();

        assert!(snapshot.version >= 1);
        assert_eq!(snapshot.status[0].path, "worker.txt");
        assert_eq!(scanner.scan_count(), scan_count);
        assert_eq!(service.worktree_count(), 1);
    }

    #[tokio::test]
    pub async fn snapshot_dtos_are_derived_from_same_cached_version() {
        // Given
        let scanner = Arc::new(CountingScanner::with_status(vec![FileStatusDto {
            path: "changed.txt".to_string(),
            index_status: "none".to_string(),
            worktree_status: "modified".to_string(),
        }]));
        let service =
            counting_service(scanner, Arc::new(CountingRepositoryStateWatcher::default()));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service.subscribe(path).unwrap();
        for _ in 0..100 {
            if service.get_snapshot(path).unwrap().version >= 1 {
                break;
            }
            // When
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let status = service.get_snapshot(path).unwrap();
        let diff_stats = service.get_snapshot(path).unwrap();
        let head_tree = service.get_snapshot(path).unwrap();

        // Then
        assert!(status.version >= 1);
        assert_eq!(diff_stats.version, status.version);
        assert_eq!(head_tree.version, status.version);
        assert_eq!(service.dirty_count(path).value, Some(status.status.len()));
    }

    #[tokio::test]
    pub async fn watcher_creation_happens_only_through_subscribe_paths() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let (dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        let path = dir.path().to_str().unwrap();

        service.get_snapshot(path).unwrap();
        service.get_snapshot(path).unwrap();
        service.get_snapshot(path).unwrap();

        assert_eq!(watcher.start_count(), 0);
        assert_eq!(service.worktree_count(), 0);

        service.start_git_dir_watching(path).unwrap();
        assert_eq!(watcher.start_count(), 1);
        assert_eq!(service.worktree_count(), 1);

        let git_dir = tempfile::TempDir::new().unwrap();
        let git_path = git_dir.path().to_str().unwrap();
        service.start_git_dir_watching(git_path).unwrap();
        assert_eq!(watcher.start_count(), 2);
        assert_eq!(service.worktree_count(), 2);
    }

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
    pub async fn same_canonical_worktree_multiple_subscriptions_start_watchers_once() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        service.subscribe(path).unwrap();
        service.subscribe(path).unwrap();
        service.subscribe(path).unwrap();

        assert_eq!(watcher.start_count(), 1);
        assert_eq!(service.worktree_count(), 1);
    }

    #[tokio::test]
    pub async fn different_worktrees_start_one_watcher_each_without_duplicate_paths() {
        let scanner = Arc::new(CountingScanner::default());
        let watcher = Arc::new(CountingRepositoryStateWatcher::default());
        let service = counting_service(scanner, watcher.clone());
        let first = tempfile::TempDir::new().unwrap();
        let second = tempfile::TempDir::new().unwrap();
        let first_path = first.path().to_str().unwrap();
        let second_path = second.path().to_str().unwrap();

        service.subscribe(first_path).unwrap();
        service.subscribe(second_path).unwrap();

        let started_paths = watcher.started_paths();
        assert_eq!(started_paths.len(), 2);
        assert!(started_paths.contains(&first_path.to_string()));
        assert!(started_paths.contains(&second_path.to_string()));
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

    #[test]
    pub fn real_repo_dirty_count_matches_legacy_count_for_untracked_rename_and_typechange() {
        let (dir, repo) = crate::test_support::git::create_test_repo();
        crate::test_support::git::create_initial_commit(&repo);
        crate::test_support::git::add_and_commit(&repo, "rename-old.txt", "old", "add old");
        crate::test_support::git::add_and_commit(&repo, "typechange", "file", "add typechange");

        std::fs::create_dir_all(dir.path().join("untracked-dir").join("nested")).unwrap();
        std::fs::write(
            dir.path()
                .join("untracked-dir")
                .join("nested")
                .join("file.txt"),
            "new",
        )
        .unwrap();
        std::fs::rename(
            dir.path().join("rename-old.txt"),
            dir.path().join("rename-new.txt"),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("typechange")).unwrap();
        std::fs::create_dir(dir.path().join("typechange")).unwrap();
        std::fs::write(dir.path().join("typechange").join("child.txt"), "child").unwrap();

        let path = dir.path().to_str().unwrap();
        let status = crate::adaptor::gateway::repository::status::get_git_status(path).unwrap();
        let scanner = Arc::new(CountingScanner::with_status(
            status.into_iter().map(Into::into).collect(),
        ));
        let service = RepositoryStateService::new(
            Arc::new(TestRepositoryStateRepository),
            scanner,
            crate::test_support::state_subscription::test_subscriptions(),
            Arc::new(NoopRepositoryStateWatcher),
            Arc::new(TestRepositoryStateWorkerRuntime),
            Arc::new(IdentityWorktreePathNormalizer),
            crate::test_support::state_subscription::repository_driver(),
        );

        let legacy =
            crate::adaptor::gateway::repository::worktree::get_worktree_dirty_count(path).unwrap();
        let snapshot_count = service.get_snapshot(path).unwrap().status.len() as u32;

        assert_eq!(snapshot_count, legacy);
    }

    #[tokio::test]
    pub async fn subscriptions_share_state_but_get_distinct_release_ids() {
        let service = test_service(Arc::new(EmptyScanner));
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap();

        let first = service.subscribe(path).unwrap();
        let second = service.subscribe(path).unwrap();

        assert_ne!(first, second);
        assert_eq!(service.worktree_count(), 1);
        assert!(service.stop_watching(first).unwrap());
        assert_eq!(service.worktree_count(), 1);
        assert!(service.stop_watching(second).unwrap());
        assert_eq!(service.worktree_count(), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    pub async fn canonical_state_notifies_each_subscriber_path_alias() {
        let scanner = Arc::new(EmptyScanner);
        let subscriptions = crate::test_support::state_subscription::test_subscriptions();
        let notifier = Arc::new(CapturingNotifier::repositories(&subscriptions));
        let service = test_service_with_notifier(scanner, subscriptions);
        let dir = tempfile::TempDir::new().unwrap();
        let alias_parent = tempfile::TempDir::new().unwrap();
        let alias = alias_parent.path().join("alias");
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();

        let original_path = dir.path().to_str().unwrap();
        let alias_path = alias.to_str().unwrap();
        service.subscribe(original_path).unwrap();
        service.subscribe(alias_path).unwrap();
        let state = service.ensure_watching(original_path).unwrap();
        notifier.take();

        state.invalidate(InvalidateReason::change());

        for _ in 0..100 {
            let notifications = notifier.take();
            if let Some(committed) = notifications.first() {
                assert!(committed.iter().any(|path| path == original_path));
                assert!(committed.iter().any(|path| path == alias_path));
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        panic!("timed out waiting for alias notification");
    }
}
