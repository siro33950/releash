use super::*;
use crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worktree::WorktreeState;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Scanner {
    scans: AtomicUsize,
    worktree_scans: AtomicUsize,
    transient_failures: AtomicUsize,
}

#[async_trait::async_trait]
impl RepositoryScanner for Scanner {
    async fn scan_async(
        &self,
        repo_path: &str,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scan(repo_path)
    }

    fn scan(&self, _repo_path: &str) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        if self
            .transient_failures
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                (left > 0).then(|| left - 1)
            })
            .is_ok()
        {
            return Err(RepositoryStateError::Background {
                kind: crate::usecase::failure::Failure::Technical(
                    crate::domain::failure::TechnicalFailureNature::Transient,
                ),
                message: "busy".into(),
            });
        }
        Ok(RepositorySnapshotParts {
            status: Vec::new(),
            diff_stats: Vec::new(),
            dirty_count: 0,
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn scan_worktrees(
        &self,
        _: &str,
    ) -> Result<Vec<crate::domain::repository::Worktree>, RepositoryStateError> {
        self.worktree_scans.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    fn prune_stale_branch_bases(&self, _: &str) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn test_repository走査_一時的な失敗をやり直して走査を終える() {
    // Given
    let scanner = Arc::new(Scanner {
        scans: AtomicUsize::new(0),
        worktree_scans: AtomicUsize::new(0),
        transient_failures: AtomicUsize::new(2),
    });
    let state = WorktreeState::new(
        "/repo".to_string(),
        true,
        scanner.clone(),
        crate::test_support::state_subscription::test_subscriptions(),
        Arc::new(TestRepositoryStateWorkerRuntime),
        crate::test_support::state_subscription::scan_driver(Duration::ZERO),
    );
    // When
    state.invalidate(InvalidateReason::change());
    tokio::time::sleep(Duration::from_secs(5)).await;
    // Then
    assert_eq!(scanner.scans.load(Ordering::SeqCst), 3);
    assert_eq!(state.snapshot_for_read().version, 1);
    let records = crate::test_support::retry::shared_store().records("/repo");
    assert!(records.iter().all(|record| !record.record.active));
    state.shutdown();
}

#[tokio::test]
async fn test_repository走査_偽の遅延で理由を併合しshutdown後は走査しない() {
    // Given
    let scanner = Arc::new(Scanner {
        scans: AtomicUsize::new(0),
        worktree_scans: AtomicUsize::new(0),
        transient_failures: AtomicUsize::new(0),
    });
    let runtime = Arc::new(TestRepositoryStateWorkerRuntime);
    let (workers, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let state = WorktreeState::new(
        "/repo-fake".into(),
        true,
        scanner.clone(),
        crate::test_support::state_subscription::test_subscriptions(),
        runtime.clone(),
        workers,
    );
    let elapsed = Arc::new(tokio::sync::Notify::new());
    let armed = Arc::new(tokio::sync::Notify::new());
    let task = tokio::spawn(run_worker(
        crate::test_support::retry::test_retrying(),
        requests.recv().await.unwrap(),
        runtime,
        Arc::new({
            let elapsed = elapsed.clone();
            let armed = armed.clone();
            move || {
                armed.notify_one();
                let elapsed = elapsed.clone();
                Box::pin(async move {
                    elapsed.notified().await;
                })
            }
        }),
    ));
    state.invalidate(InvalidateReason::files());
    armed.notified().await;
    assert_eq!(scanner.scans.load(Ordering::SeqCst), 0);
    // When
    state.invalidate(InvalidateReason::refs());
    state.invalidate(InvalidateReason::files());
    elapsed.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.snapshot_for_read().version == 0
            || scanner.worktree_scans.load(Ordering::SeqCst) == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Then
    assert_eq!(scanner.scans.load(Ordering::SeqCst), 1);
    assert_eq!(scanner.worktree_scans.load(Ordering::SeqCst), 1);
    // When
    state.invalidate(InvalidateReason::files());
    armed.notified().await;
    state.shutdown();
    elapsed.notify_one();
    task.await.unwrap();
    // Then
    assert_eq!(scanner.scans.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn test_repository走査_単回操作は一時エラーを再試行せず結果を確定する() {
    // Given
    let scanner = Arc::new(Scanner {
        scans: AtomicUsize::new(0),
        worktree_scans: AtomicUsize::new(0),
        transient_failures: AtomicUsize::new(2),
    });
    let runtime = RepositoryScanWorkerRuntime::new();
    let (workers, _requests) = tokio::sync::mpsc::unbounded_channel();
    let state = WorktreeState::new(
        "/repo-single".into(),
        true,
        scanner.clone(),
        crate::test_support::state_subscription::test_subscriptions(),
        Arc::new(RepositoryScanWorkerRuntime::new()),
        workers,
    );
    let reason = InvalidateReason::change();
    let generation = state.requested_generation();
    // When
    let status = state.scan_once(scanner.clone(), &runtime).await;
    assert!(status.is_err());
    let next = state
        .finish_worker_scan(generation, reason, Some(status), scanner.clone(), &runtime)
        .await;
    // Then
    assert_eq!(scanner.scans.load(Ordering::SeqCst), 1);
    assert!(matches!(
        next,
        crate::usecase::repository_state::worktree::ScanContinuation::Finished
    ));
    assert!(state.read_snapshot().is_err());
    state.shutdown();
}

#[test]
fn test_失効通知_送信した走査理由を受信側へ渡す() {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let sender = TokioInvalidationSender(sender);
    let reason = InvalidateReason::refs();

    sender.send(reason).unwrap();

    assert_eq!(receiver.try_recv().unwrap(), reason);
}

#[test]
fn test_失効通知_受信側終了をrepository状態エラーとして返す() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let sender = TokioInvalidationSender(sender);
    drop(receiver);

    let error = sender.send(InvalidateReason::change()).unwrap_err();

    assert!(matches!(
        error,
        RepositoryStateError::Watcher(message) if message == "repository snapshot worker is stopped"
    ));
}
