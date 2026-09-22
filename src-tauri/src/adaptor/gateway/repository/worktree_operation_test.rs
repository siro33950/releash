use super::*;

#[test]
fn test_worktree変更排他_未認可pathの反復でも解放後のファイル数は増えない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let locks = FileWorktreeOperationLocks::new(directory.path());
    // When
    for index in 0..128 {
        drop(locks.mutation(&format!("/unauthorized/{index}")).unwrap());
        assert!(locks.mutation(&format!("/invalid\0/{index}")).is_err());
    }
    // Then
    assert_eq!(std::fs::read_dir(&locks.directory).unwrap().count(), 1);
}

#[tokio::test]
async fn test_worktree変更排他_最後の所有者だけがファイルを削除し再取得後も排他する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let first = FileWorktreeOperationLocks::new(directory.path());
    let second = FileWorktreeOperationLocks::new(directory.path());
    let mutation = first.mutation("/repo/worktree").unwrap();
    let another = second.mutation("/repo/worktree").unwrap();
    // When
    drop(mutation);
    // Then
    assert_eq!(std::fs::read_dir(&first.directory).unwrap().count(), 3);
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(30),
        first.deletion("/repo/worktree")
    )
    .await
    .is_err());
    drop(another);
    assert_eq!(std::fs::read_dir(&first.directory).unwrap().count(), 1);
    let deletion = second.deletion("/repo/worktree").await.unwrap();
    assert!(first.mutation("/repo/worktree").is_err());
    drop(deletion);
    assert_eq!(std::fs::read_dir(&first.directory).unwrap().count(), 1);
    assert!(first.mutation("/repo/worktree").is_ok());
}

#[tokio::test]
async fn test_worktree削除排他_別所有者の変更を待ち削除待機中から新しい変更を拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let first = FileWorktreeOperationLocks::new(directory.path());
    let second = FileWorktreeOperationLocks::new(directory.path());
    let mutation = first.mutation("/repo/worktree").unwrap();
    // When
    let deletion = second.deletion("/repo/worktree/");
    tokio::pin!(deletion);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut deletion)
            .await
            .is_err()
    );
    // Then
    assert!(first.mutation("/repo/worktree").is_err());
    assert!(first.mutation("/other").is_ok());
    drop(mutation);
    let guard = deletion.await.unwrap();
    assert!(first.mutation("/repo/worktree").is_err());
    drop(guard);
    assert!(first.mutation("/repo/worktree").is_ok());
}

#[tokio::test]
async fn test_worktree削除排他_取消時は変更受付を戻し別表記も同じ実体として拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("worktree");
    std::fs::create_dir(&path).unwrap();
    let locks = FileWorktreeOperationLocks::new(directory.path());
    let mutation = locks.mutation(path.to_str().unwrap()).unwrap();
    // When
    let alias = path.join(".");
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(30),
        locks.deletion(alias.to_str().unwrap())
    )
    .await
    .is_err());
    // Then
    assert!(locks.mutation(path.to_str().unwrap()).is_ok());
    drop(mutation);
    let guard = locks.deletion(path.to_str().unwrap()).await.unwrap();
    assert!(locks.mutation(alias.to_str().unwrap()).is_err());
    drop(guard);
}

#[cfg(unix)]
#[tokio::test]
async fn test_worktree削除排他_フォルダ削除後も親pathの別表記からの変更を拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(&parent, &alias).unwrap();
    let path = parent.join("worktree");
    std::fs::create_dir(&path).unwrap();
    let locks = FileWorktreeOperationLocks::new(directory.path());
    let deletion = locks.deletion(path.to_str().unwrap()).await.unwrap();
    // When
    std::fs::remove_dir(&path).unwrap();
    // Then
    assert!(locks
        .mutation(alias.join("worktree").to_str().unwrap())
        .is_err());
    drop(deletion);
    assert!(locks
        .mutation(alias.join("worktree").to_str().unwrap())
        .is_ok());
}

#[tokio::test]
async fn test_worktree削除排他_保存先がファイルならioエラーを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("worktree-operations"), "unavailable").unwrap();
    let locks = FileWorktreeOperationLocks::new(directory.path());
    // When / Then
    assert!(matches!(
        locks.mutation("/repo"),
        Err(RepositoryError::External(_))
    ));
    assert!(matches!(
        locks.deletion("/repo").await,
        Err(RepositoryError::External(_))
    ));
}
