use super::*;
use crate::common::operation_context::Deadline;

#[test]
fn test_ファイルlock_期限と取り消しで待ちを終え取得済みlockは解放できる() {
    for expire in [false, true] {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let holder = File::create(&path).unwrap();
        let waiter = File::open(&path).unwrap();
        fs2::FileExt::try_lock_exclusive(&holder).unwrap();
        let token = tokio_util::sync::CancellationToken::new();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(10))),
            std::sync::Arc::new(token.clone()),
        );
        if !expire {
            token.cancel();
        }
        // When / Then
        assert!(
            matches!(crate::common::operation_context::sync_scope(context, || exclusive(&waiter)), Err(LockError::Stopped(reason)) if reason == if expire { OperationStopped::Expired } else { OperationStopped::Cancelled })
        );
        drop(holder);
        assert!(exclusive(&waiter).is_ok());
        drop(waiter);
        assert!(exclusive(&File::open(path).unwrap()).is_ok());
    }
}
