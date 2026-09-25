use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
#[tokio::test(flavor = "current_thread")]
async fn test_読み込み待ち_同じruntimeの別処理が先に完了する() {
    // Given
    let pool = ReaderPool::new();
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
    let pool = ReaderPool::new();
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
    let job = pool.pop_blocking().unwrap();
    (job.task)(
        &Connection::open_in_memory().unwrap(),
        &job.context
            .with_deadline(crate::domain::operation_context::Deadline::new(
                std::time::Instant::now(),
            )),
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

#[tokio::test]
async fn test_読み込み実行中_期限と取り消しでsqliteを止め接続を再利用できる() {
    use crate::domain::operation_context::{Deadline, OperationContext};
    use std::time::{Duration, Instant};
    for expire in [false, true] {
        // Given
        let pool = ReaderPool::new();
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            worker_pool.run_worker(Connection::open_in_memory().unwrap())
        });
        let token = tokio_util::sync::CancellationToken::new();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(30))),
            Arc::new(token.clone()),
        );
        let (started, ready) = tokio::sync::oneshot::channel();
        let query = crate::other::operation_context::scope(context, pool.submit(move |connection| {
            let _ = started.send(());
            connection.query_row("WITH RECURSIVE numbers(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM numbers WHERE n<1000000000) SELECT sum(n) FROM numbers", [], |row| row.get::<_, i64>(0)).map_err(|error| storage_unavailable(&error))
        }));
        tokio::pin!(query);
        assert!(futures_util::poll!(&mut query).is_pending());
        ready.await.unwrap();
        // When
        if !expire {
            token.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(2), query)
            .await
            .unwrap()
            .unwrap_err();
        // Then
        assert_eq!(
            error.failure_kind(),
            if expire {
                FailureKind::Expired
            } else {
                FailureKind::Cancelled
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), pool.submit(|_| Ok(42)))
                .await
                .unwrap()
                .unwrap(),
            42
        );
        pool.close();
        worker.join().unwrap();
    }
}

#[tokio::test]
async fn test_読み込み待ち_実行前の期限切れでqueryを実行しない() {
    use crate::domain::operation_context::Deadline;
    let pool = ReaderPool::new();
    let context =
        OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
    let result = crate::other::operation_context::scope(
        context,
        pool.submit(|_| -> Result<(), LocalEventQueryError> { panic!("expired query ran") }),
    )
    .await;
    assert_eq!(result, Err(LocalEventQueryError::DeadlineExceeded));
}

#[tokio::test]
async fn test_読み込み取消_短い文の間で取り消しても次の文を実行しない() {
    use crate::domain::operation_context::OperationContext;
    use std::sync::atomic::{AtomicBool, Ordering};
    // Given
    let pool = ReaderPool::new();
    let worker_pool = pool.clone();
    let worker =
        std::thread::spawn(move || worker_pool.run_worker(Connection::open_in_memory().unwrap()));
    let token = tokio_util::sync::CancellationToken::new();
    let context = OperationContext::new(None, Arc::new(token.clone()));
    let second_ran = Arc::new(AtomicBool::new(false));
    let observed = second_ran.clone();
    // When
    let result = crate::other::operation_context::scope(
        context,
        pool.submit(move |connection| {
            connection
                .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                .unwrap();
            token.cancel();
            let second = connection.query_row("SELECT 2", [], |row| row.get::<_, i64>(0));
            observed.store(second.is_ok(), Ordering::SeqCst);
            second.map_err(|error| storage_unavailable(&error))
        }),
    )
    .await;
    pool.close();
    worker.join().unwrap();
    // Then
    assert_eq!(result.unwrap_err().failure_kind(), FailureKind::Cancelled);
    assert!(!second_ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_reader_busy待ち_実際のdb競合で期限と取消を引き継ぐ() {
    use crate::domain::operation_context::{Deadline, OperationContext};
    use std::time::{Duration, Instant};
    for expire in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("busy.db");
        let blocker = Connection::open(&path).unwrap();
        blocker
            .execute_batch("CREATE TABLE value(n); INSERT INTO value VALUES(1);")
            .unwrap();
        let connection = super::super::connection::open_reader(&path).unwrap();
        blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let pool = ReaderPool::new();
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || worker_pool.run_worker(connection));
        let token = tokio_util::sync::CancellationToken::new();
        let context = OperationContext::new(
            expire.then(|| Deadline::new(Instant::now() + Duration::from_millis(100))),
            Arc::new(token.clone()),
        );
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let mut query = Box::pin(crate::other::operation_context::scope(
            context,
            pool.submit(move |connection| {
                started.send(()).unwrap();
                let result =
                    connection.query_row("SELECT n FROM value", [], |row| row.get::<_, i64>(0));
                finished
                    .send(
                        result
                            .as_ref()
                            .err()
                            .and_then(|error| error.sqlite_error_code()),
                    )
                    .unwrap();
                result.map_err(|error| storage_unavailable(&error))
            }),
        ));
        tokio::select! { _ = ready => {}, result = &mut query => panic!("query ended early: {result:?}") }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(futures_util::poll!(&mut query).is_pending());
        if !expire {
            token.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(1), query)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.failure_kind(),
            if expire {
                FailureKind::Expired
            } else {
                FailureKind::Cancelled
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), completion)
                .await
                .unwrap()
                .unwrap(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        blocker.execute_batch("ROLLBACK").unwrap();
        pool.close();
        worker.join().unwrap();
    }
}

#[tokio::test]
async fn test_読み込み資源期限_親が無期限でも長い期限でも二秒で待ちを終える() {
    use crate::domain::operation_context::{Deadline, OperationContext};
    use std::time::{Duration, Instant};
    // Given
    let pool = ReaderPool::new();
    let start = Instant::now();
    let contexts = [
        OperationContext::default(),
        OperationContext::default().with_deadline(Deadline::new(start + Duration::from_secs(10))),
    ];
    let mut queries = Vec::new();
    for context in contexts {
        let mut query = Box::pin(crate::other::operation_context::scope(
            context,
            pool.submit(|_| -> Result<(), LocalEventQueryError> {
                panic!("expired queued query must not run")
            }),
        ));
        assert!(futures_util::poll!(&mut query).is_pending());
        queries.push(query);
    }
    // When
    for query in queries {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(4), query)
                .await
                .unwrap(),
            Err(LocalEventQueryError::DeadlineExceeded)
        );
    }
    // Then
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_secs(4));
    let connection = Connection::open_in_memory().unwrap();
    for _ in 0..2 {
        let job = pool.pop_blocking().unwrap();
        (job.task)(&connection, &job.context);
    }
    pool.close();
}
