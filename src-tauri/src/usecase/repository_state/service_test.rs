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
