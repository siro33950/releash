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

#[tokio::test]
async fn test_worktree削除排他_registryとactive待ちを期限と取消で終了する() {
    use crate::domain::operation_context::{Deadline, OperationContext, OperationStopped};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    for registry_wait in [false, true] {
        for expire in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let locks = FileWorktreeOperationLocks::new(directory.path());
            let registry = registry_wait.then(|| locks.registry_lock().unwrap());
            let mutation = (!registry_wait).then(|| locks.mutation("/repo/worktree").unwrap());
            let token = tokio_util::sync::CancellationToken::new();
            let context = OperationContext::new(
                expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(50))),
                Arc::new(token.clone()),
            );
            let mut deletion = Box::pin(crate::other::operation_context::scope(
                context,
                locks.deletion("/repo/worktree"),
            ));
            assert!(futures_util::poll!(&mut deletion).is_pending());
            if !expire {
                token.cancel();
            }
            let result = tokio::time::timeout(Duration::from_secs(2), deletion)
                .await
                .unwrap();
            assert!(
                matches!(result, Err(RepositoryError::Stopped(error)) if error == if expire { OperationStopped::Expired } else { OperationStopped::Cancelled })
            );
            drop(registry);
            drop(mutation);
            assert!(locks.mutation("/repo/worktree").is_ok());
            let lease = locks.deletion("/repo/worktree").await.unwrap();
            drop(lease);
        }
    }
}

#[test]
fn test_worktree削除排他_複製されたfdが残ってもleaseの破棄でlockを解放する() {
    let directory = tempfile::tempdir().unwrap();
    let locks = FileWorktreeOperationLocks::new(directory.path());
    let lease = locks.lease("/repo/worktree", true).unwrap();
    fs2::FileExt::try_lock_exclusive(&lease.files[1]).unwrap();
    let _duplicates: Vec<_> = lease
        .files
        .iter()
        .map(|file| file.try_clone().unwrap())
        .collect();
    drop(lease);
    assert!(locks.mutation("/repo/worktree").is_ok());
}

#[test]
fn test_worktree変更排他_registry待ちを期限と取消で終了し再取得できる() {
    use crate::domain::operation_context::{
        Cancellation, Deadline, OperationContext, OperationStopped,
    };
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};
    struct ObservedCancellation {
        token: tokio_util::sync::CancellationToken,
        checked: mpsc::Sender<()>,
    }
    impl Cancellation for ObservedCancellation {
        fn is_cancelled(&self) -> bool {
            let _ = self.checked.send(());
            self.token.is_cancelled()
        }
    }
    for expire in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let locks = FileWorktreeOperationLocks::new(directory.path());
        let registry = locks.registry_lock().unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let (checked, observed) = mpsc::channel();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(200))),
            Arc::new(ObservedCancellation {
                token: token.clone(),
                checked,
            }),
        );
        let worker_locks = locks.clone();
        let (reply, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = crate::other::operation_context::sync_scope(context, || {
                worker_locks.mutation("/repo/worktree").map(drop)
            });
            reply.send(result).unwrap();
        });
        // When
        observed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result.try_recv().is_err());
        if !expire {
            token.cancel();
        }
        let stopped = result.recv_timeout(Duration::from_secs(2));
        drop(registry);
        worker.join().unwrap();
        // Then
        assert!(
            matches!(stopped.unwrap(), Err(RepositoryError::Stopped(error))
            if error == if expire { OperationStopped::Expired } else { OperationStopped::Cancelled })
        );
        drop(locks.mutation("/repo/worktree").unwrap());
        assert_eq!(std::fs::read_dir(&locks.directory).unwrap().count(), 1);
        drop(locks.registry_lock().unwrap());
    }
}

#[test]
fn test_worktree排他_停止済みscopeの破棄でも最後の所有者がファイルを掃除する() {
    use crate::domain::operation_context::{Deadline, OperationContext};
    use std::{sync::Arc, time::Instant};
    for expire in [false, true] {
        for deleting in [false, true] {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let locks = FileWorktreeOperationLocks::new(directory.path());
            let lease = locks.lease("/repo/worktree", deleting).unwrap();
            let another = (!deleting).then(|| locks.mutation("/repo/worktree").unwrap());
            let token = tokio_util::sync::CancellationToken::new();
            if !expire {
                token.cancel();
            }
            let context = OperationContext::new(
                expire.then(|| Deadline::new(Instant::now())),
                Arc::new(token),
            );
            // When / Then
            crate::other::operation_context::sync_scope(context, || {
                drop(lease);
                if another.is_some() {
                    assert_eq!(std::fs::read_dir(&locks.directory).unwrap().count(), 3);
                }
                drop(another);
                assert_eq!(std::fs::read_dir(&locks.directory).unwrap().count(), 1);
            });
            assert!(locks.mutation("/repo/worktree").is_ok());
        }
    }
}

#[test]
fn test_worktree排他_破棄時のregistry競合では待たずlockを解放する() {
    use crate::domain::operation_context::{Deadline, OperationContext};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    for expire in [false, true] {
        for deleting in [false, true] {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let locks = FileWorktreeOperationLocks::new(directory.path());
            let lease = locks.lease("/repo/worktree", deleting).unwrap();
            let key = lease.key.clone();
            let registry = locks.registry_lock().unwrap();
            let token = tokio_util::sync::CancellationToken::new();
            if !expire {
                token.cancel();
            }
            let context = OperationContext::new(
                expire.then(|| Deadline::new(Instant::now())),
                Arc::new(token),
            );
            let (done, finished) = std::sync::mpsc::channel();
            // When
            let worker = std::thread::spawn(move || {
                crate::other::operation_context::sync_scope(context, || drop(lease));
                done.send(()).unwrap();
            });
            let result = finished.recv_timeout(Duration::from_secs(2));
            // Then
            for suffix in ["admission", "active"] {
                let file = locks.open(&format!("{key}.{suffix}")).unwrap();
                fs2::FileExt::try_lock_exclusive(&file).unwrap();
            }
            drop(registry);
            worker.join().unwrap();
            result.unwrap();
            let start = Instant::now();
            while std::fs::read_dir(&locks.directory).unwrap().count() != 1 {
                assert!(start.elapsed() < Duration::from_secs(2));
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
