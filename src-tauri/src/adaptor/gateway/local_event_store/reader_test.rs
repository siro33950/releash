use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
use std::sync::atomic::{AtomicI64, Ordering};

struct Clock(AtomicI64);
impl StoreClock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn test_読み込み待ち_同じruntimeの別処理が先に完了する() {
    // Given
    let pool = ReaderPool::new(Arc::new(Clock(AtomicI64::new(0))));
    let mut read = Box::pin(pool.submit(|_| Ok(42)));
    assert!(futures_util::poll!(&mut read).is_pending());

    // When
    let other = tokio::spawn(async { 7 }).await.unwrap();

    // Then
    assert_eq!(other, 7);
    assert!(futures_util::poll!(&mut read).is_pending());
    let worker_pool = pool.clone();
    let worker = std::thread::spawn(move || {
        worker_pool.run_worker(Connection::open_in_memory().unwrap());
    });
    assert_eq!(read.await.unwrap(), 42);
    pool.close();
    worker.join().unwrap();
}

#[tokio::test]
async fn test_読み込みキュー_混雑と期限切れとreply喪失を分類する() {
    // Given
    let clock = Arc::new(Clock(AtomicI64::new(0)));
    let pool = ReaderPool::new(clock.clone());
    let mut pending = Vec::new();
    for _ in 0..READ_QUEUE_MAX_DEPTH {
        let mut read = Box::pin(pool.submit(|_| Ok(())));
        assert!(futures_util::poll!(&mut read).is_pending());
        pending.push(read);
    }

    // When / Then
    assert_eq!(
        pool.submit(|_| Ok(())).await,
        Err(LocalEventQueryError::QueryBusy)
    );
    clock.0.store(QUERY_DEADLINE_MS + 1, Ordering::SeqCst);
    let job = pool.pop_blocking().unwrap();
    (job.task)(
        &Connection::open_in_memory().unwrap(),
        clock.now_ms() > job.deadline_ms,
    );
    assert_eq!(
        pending.remove(0).await,
        Err(LocalEventQueryError::DeadlineExceeded)
    );
    pool.close();
    assert_eq!(
        pending.remove(0).await.unwrap_err().failure_kind(),
        FailureKind::Temporary
    );
    assert_eq!(
        pool.submit(|_| Ok(())).await.unwrap_err().failure_kind(),
        FailureKind::StateRequired
    );
}

#[test]
fn test_sqlite分類_環境起因の全コードを同じ分類にする() {
    // Given
    for code in [
        rusqlite::ffi::SQLITE_PERM,
        rusqlite::ffi::SQLITE_READONLY,
        rusqlite::ffi::SQLITE_CANTOPEN,
        rusqlite::ffi::SQLITE_FULL,
        rusqlite::ffi::SQLITE_IOERR,
        rusqlite::ffi::SQLITE_NOMEM,
        rusqlite::ffi::SQLITE_PROTOCOL,
        rusqlite::ffi::SQLITE_TOOBIG,
        rusqlite::ffi::SQLITE_NOLFS,
        rusqlite::ffi::SQLITE_INTERRUPT,
        rusqlite::ffi::SQLITE_AUTH,
    ] {
        let error = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        // When / Then
        assert_eq!(sqlite_failure_kind(&error), FailureKind::StateRequired);
        assert_eq!(
            storage_unavailable(&error).failure_kind(),
            FailureKind::StateRequired
        );
    }
}
