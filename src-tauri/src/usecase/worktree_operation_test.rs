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
