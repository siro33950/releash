use super::*;
use crate::usecase::repository_state::runtime::tests_support::TestRepositoryStateWorkerRuntime;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worktree::{NoopRepositoryStateNotifier, WorktreeState};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Scanner {
    scans: AtomicUsize,
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
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
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
            branch_cards: Vec::new(),
            diff_file_tree: Vec::new(),
            staged_diff_file_tree: Vec::new(),
            changes_diff_file_tree: Vec::new(),
        })
    }

    fn prune_stale_branch_bases(&self, _: &str, _: &[String]) -> Result<(), RepositoryStateError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn test_repository走査_一時的な失敗をやり直して走査を終える() {
    // Given
    let scanner = Arc::new(Scanner {
        scans: AtomicUsize::new(0),
        transient_failures: AtomicUsize::new(2),
    });
    let state = WorktreeState::new(
        "/repo".to_string(),
        scanner.clone(),
        Arc::new(NoopRepositoryStateNotifier),
        Arc::new(TestRepositoryStateWorkerRuntime),
        Duration::ZERO,
    );
    // When
    state.invalidate(InvalidateReason::initial());
    tokio::time::sleep(Duration::from_secs(5)).await;
    // Then
    assert_eq!(scanner.scans.load(Ordering::SeqCst), 3);
    assert_eq!(state.snapshot_for_read().version, 1);
    let records = crate::usecase::retry::shared().records("/repo");
    assert!(records.iter().all(|record| !record.record.active));
    state.shutdown();
}
