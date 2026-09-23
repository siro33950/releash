use super::*;
use crate::usecase::repository_state::{
    runtime::tests_support::{IdentityWorktreePathNormalizer, TestRepositoryStateWorkerRuntime},
    snapshot::RepositorySnapshotParts,
    worktree::{NoopRepositoryStateWatcher, SnapshotNotification},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Default)]
struct Scanner {
    calls: AtomicUsize,
    fail: AtomicBool,
    on_scan: parking_lot::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}
impl RepositoryScanner for Scanner {
    fn scan(&self, _: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.on_scan.lock().as_ref() {
            hook();
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
            diff_file_tree: vec![],
            staged_diff_file_tree: vec![],
            changes_diff_file_tree: vec![],
            branch_cards: vec![BranchCardDto {
                name: format!("scan-{call}"),
                is_deleting: false,
                is_main_worktree: true,
                worktree_path: Some("/repo".into()),
                dirty_count: 0,
                is_merged: false,
                ahead: 0,
                behind: 0,
                has_upstream: false,
                base_ahead: 0,
            }],
        })
    }
    fn status_with_ignored(&self, _: &str) -> Result<Vec<FileStatusDto>, RepositoryStateError> {
        Ok(vec![])
    }
    fn prune_stale_branch_bases(&self, _: &str, _: &[String]) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}
struct Repository;
impl RepositoryStateRepository for Repository {
    fn include_deleting_worktrees(
        &self,
        _: &str,
        cards: &mut Vec<BranchCardDto>,
    ) -> Result<(), RepositoryStateError> {
        for card in cards {
            card.is_deleting = true;
        }
        Ok(())
    }

    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        Ok(path.into())
    }
}
#[derive(Default)]
struct Notifier {
    notifications: AtomicUsize,
    changed: tokio::sync::Notify,
}
impl RepositoryStateNotifier for Notifier {
    fn snapshot_changed(&self, _: SnapshotNotification) {
        self.notifications.fetch_add(1, Ordering::SeqCst);
        self.changed.notify_one();
    }
}
fn service(scanner: Arc<Scanner>, notifier: Arc<Notifier>) -> RepositoryStateService {
    RepositoryStateService::new_with_scanner(
        Arc::new(Repository),
        scanner,
        notifier,
        Arc::new(NoopRepositoryStateWatcher),
        Arc::new(TestRepositoryStateWorkerRuntime),
        Arc::new(IdentityWorktreePathNormalizer),
        Duration::ZERO,
    )
}

#[tokio::test]
async fn test_一覧の再走査_監視中も保存済みsnapshotを使わず毎回走査する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = service(scanner.clone(), notifier.clone());
    service.start_git_dir_watching("/repo").unwrap();
    notifier.changed.notified().await;
    let previous = service.list_branches_with_status("/repo").unwrap();
    // When
    let next = service.rescan_branches("/repo").await.unwrap();
    let next_snapshot = service.get_snapshot("/repo").unwrap();
    assert_eq!(next_snapshot.branch_cards[0].name, next[0].name);
    let latest = service.rescan_branches("/repo").await.unwrap();
    // Then
    assert!(next[0].is_deleting);
    assert!(latest[0].is_deleting);
    assert_ne!(previous[0].name, next[0].name);
    assert_ne!(next[0].name, latest[0].name);
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 3);
    assert_eq!(notifier.notifications.load(Ordering::SeqCst), 1);
    let snapshot = service.get_snapshot("/repo").unwrap();
    assert_eq!(snapshot.branch_cards[0].name, latest[0].name);
    assert_eq!(snapshot.status[0].path, latest[0].name);
    assert_eq!(service.get_status("/repo", false).unwrap(), snapshot.status);
    assert_eq!(
        service.get_diff_stats("/repo").unwrap(),
        snapshot.diff_stats
    );
    assert_eq!(
        service.list_branches_with_status("/repo").unwrap()[0].name,
        latest[0].name
    );
    assert_eq!(snapshot.version, next_snapshot.version + 1);
    // When
    scanner.fail.store(true, Ordering::SeqCst);
    assert!(service.rescan_branches("/repo").await.is_err());
    // Then
    assert!(Arc::ptr_eq(
        &snapshot,
        &service.get_snapshot("/repo").unwrap()
    ));
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
    tokio::time::timeout(Duration::from_secs(2), notifier.changed.notified())
        .await
        .unwrap();
    // Then
    assert_eq!(notifier.notifications.load(Ordering::SeqCst), 1);
    assert!(service.rescan_branches("/repo").await.is_err());
    // When
    scanner.fail.store(false, Ordering::SeqCst);
    let result = service.rescan_branches("/repo").await.unwrap();
    // Then
    assert_eq!(result.len(), 1);
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_監視走査との競合_共有snapshotへのcommitを直列化する() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = Arc::new(service(scanner.clone(), notifier.clone()));
    service.start_git_dir_watching("/repo").unwrap();
    notifier.changed.notified().await;
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
    state.invalidate(super::super::worker::InvalidateReason::git(false));
    started.notified().await;
    // When
    let refresh = {
        let service = service.clone();
        tokio::spawn(async move { service.rescan_branches("/repo").await.unwrap() })
    };
    tokio::task::yield_now().await;
    assert_eq!(scanner.calls.load(Ordering::SeqCst), 2);
    release.send(()).unwrap();
    let result = refresh.await.unwrap();
    // Then
    let snapshot = service.get_snapshot("/repo").unwrap();
    assert_eq!(snapshot.branch_cards[0].name, result[0].name);
    assert_eq!(snapshot.status[0].path, "scan-2");
    assert_eq!(snapshot.version, 3);
    assert_eq!(notifier.notifications.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_明示再走査_途中で失効した結果を公開せず再走査して返す() {
    // Given
    let scanner = Arc::new(Scanner::default());
    let notifier = Arc::new(Notifier::default());
    let service = Arc::new(service(scanner.clone(), notifier.clone()));
    service.start_git_dir_watching("/repo").unwrap();
    notifier.changed.notified().await;
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
        tokio::spawn(async move { service.rescan_branches("/repo").await.unwrap() })
    };
    started.notified().await;
    state.invalidate(super::super::worker::InvalidateReason::git(false));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), started.notified())
        .await
        .unwrap();
    // Then
    assert!(Arc::ptr_eq(
        &previous,
        &service.get_snapshot("/repo").unwrap()
    ));
    assert_eq!(notifier.notifications.load(Ordering::SeqCst), 1);
    // When
    release.send(()).unwrap();
    let result = refresh.await.unwrap();
    // Then
    assert_eq!(result[0].name, "scan-2");
    assert_ne!(
        service.get_snapshot("/repo").unwrap().branch_cards[0].name,
        "scan-1"
    );
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
    state.invalidate(super::super::worker::InvalidateReason::git(false));
    let result = state.commit_snapshot(obsolete, generation);
    // Then
    assert!(result.is_none());
    assert!(Arc::ptr_eq(&previous, &state.snapshot_for_read()));
    let current = state
        .commit_snapshot(scanner.scan("/repo").unwrap(), state.requested_generation())
        .unwrap();
    assert_eq!(current.version, previous.version + 1);
    assert_eq!(current.branch_cards[0].name, "scan-2");
}
