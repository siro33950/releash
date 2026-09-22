use super::*;

#[tokio::test]
async fn test_worktree削除_先行変更を待ち新規変更を拒否し中断後は解放する() {
    // Given
    let operations = WorktreeOperations::default();
    let mutation = operations.mutate("/worktree").unwrap();
    // When
    let deletion = operations.delete("/worktree");
    tokio::pin!(deletion);
    assert!(futures_util::poll!(&mut deletion).is_pending());
    // Then
    assert!(operations.mutate("/worktree").is_err());
    assert!(operations.mutate("/other").is_ok());
    assert!(operations.delete("/worktree").await.is_err());
    drop(mutation);
    let guard = deletion.await.unwrap();
    assert!(operations.mutate("/worktree").is_err());
    drop(guard);
    assert!(operations.mutate("/worktree").is_ok());
}

#[tokio::test]
async fn test_worktree削除_待機中のキャンセルで変更受付を復旧する() {
    let operations = WorktreeOperations::default();
    let mutation = operations.mutate("/worktree").unwrap();
    {
        let deletion = operations.delete("/worktree");
        tokio::pin!(deletion);
        assert!(futures_util::poll!(&mut deletion).is_pending());
    }
    assert!(operations.mutate("/worktree").is_ok());
    drop(mutation);
}

#[tokio::test]
async fn test_worktree削除_lease解放中も一覧を読め排他終了で対象を消す() {
    use crate::usecase::repository_dto::BranchCardDto;
    use crate::usecase::repository_query_service::{BranchCardQuery, RepositoryQueryService};
    struct Cards;
    impl BranchCardQuery for Cards {
        fn list_branch_cards(&self, _: &str) -> Result<Vec<BranchCardDto>, RepositoryError> {
            Ok(Vec::new())
        }
    }
    struct Locks {
        releasing: Arc<tokio::sync::Notify>,
        released: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    }
    struct Lease;
    impl WorktreeOperationLease for Lease {}
    impl WorktreeOperationLease for Locks {}
    impl Drop for Locks {
        fn drop(&mut self) {
            self.releasing.notify_one();
            self.released
                .lock()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
    }
    struct Backend(
        Arc<tokio::sync::Notify>,
        Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    );
    #[async_trait::async_trait]
    impl WorktreeOperationLocks for Backend {
        fn mutation(&self, _: &str) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError> {
            Ok(Box::new(Lease))
        }
        async fn deletion(
            &self,
            _: &str,
        ) -> Result<Box<dyn WorktreeOperationLease>, RepositoryError> {
            Ok(Box::new(Locks {
                releasing: self.0.clone(),
                released: self.1.clone(),
            }))
        }
    }
    // Given
    let (release, released) = std::sync::mpsc::channel();
    let releasing = Arc::new(tokio::sync::Notify::new());
    let operations = Arc::new(WorktreeOperations::new(Arc::new(Backend(
        releasing.clone(),
        Arc::new(Mutex::new(released)),
    ))));
    let query = RepositoryQueryService::new(Arc::new(Cards), operations.clone());
    let mut guard = operations
        .delete_many(&["/worktree".into(), "workspace-state:worktree".into()])
        .await
        .unwrap();
    guard
        .accept(WorktreeDeletionTarget {
            repository_root: "/repo".into(),
            path: "/worktree".into(),
            branch: None,
        })
        .unwrap();
    // When
    let task = tokio::task::spawn_blocking(move || drop(guard));
    for _ in 0..2 {
        tokio::time::timeout(std::time::Duration::from_secs(5), releasing.notified())
            .await
            .unwrap();
        // Then
        let mut cards = Vec::new();
        query.include_deleting_worktrees("/repo", &mut cards);
        assert_eq!(cards.len(), 1);
        assert!(cards[0].is_deleting);
        assert!(operations.mutate("/worktree").is_err());
        assert!(operations.mutate("workspace-state:worktree").is_err());
        release.send(()).unwrap();
    }
    task.await.unwrap();
    let mut cards = Vec::new();
    query.include_deleting_worktrees("/repo", &mut cards);
    assert!(cards.is_empty());
    assert!(operations.mutate("/worktree").is_ok());
    assert!(operations.mutate("workspace-state:worktree").is_ok());
}
