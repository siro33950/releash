use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use connectrpc::ErrorCode;

#[tokio::test]
async fn test_読み取り接続_期限超過後も同じ接続でトランザクションを開始できる() {
    use crate::common::operation_context::{scope, Deadline};
    use std::time::{Duration, Instant};

    // Given
    let pool = ReaderPool::new();
    let worker_pool = pool.clone();
    let worker = std::thread::spawn(move || {
        let connection =
            crate::infrastructure::local_event_store_connection::configure_busy_handler(
                Connection::open_in_memory().unwrap(),
                Arc::new(crate::common::retry::RetryLimiter::new()),
            )
            .unwrap();
        worker_pool.run_worker(connection);
    });
    let context = OperationContext::default()
        .with_deadline(Deadline::new(Instant::now() + Duration::from_millis(250)));
    let (started, ready) = oneshot::channel();
    let expired = scope(
        context,
        pool.submit(move |connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                .unwrap();
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        }),
    );
    tokio::pin!(expired);
    tokio::select! {
        result = &mut expired => panic!("read ended before its transaction started: {result:?}"),
        result = ready => result.unwrap(),
    }
    let error = expired.await.unwrap_err();

    // When
    let mut results = Vec::new();
    for _ in 0..3 {
        results.push(
            pool.submit(|connection| {
                let transaction = connection
                    .unchecked_transaction()
                    .map_err(|error| storage_unavailable(&error))?;
                transaction
                    .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                    .map_err(|error| storage_unavailable(&error))
            })
            .await,
        );
    }
    pool.close();
    worker.join().unwrap();

    // Then
    assert_eq!(error.connect_code(), ErrorCode::DeadlineExceeded);
    for result in results {
        assert_eq!(result.unwrap(), 1);
    }
}

#[tokio::test]
async fn test_読み取り接続_ロールバックの失敗を呼び出し元へ返す() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

    // Given
    let pool = ReaderPool::new();
    let worker_pool = pool.clone();
    let worker = std::thread::spawn(move || {
        let mut connection =
            crate::infrastructure::local_event_store_connection::configure_busy_handler(
                Connection::open_in_memory().unwrap(),
                Arc::new(crate::common::retry::RetryLimiter::new()),
            )
            .unwrap();
        connection
            .authorizer(Some(|context: AuthContext<'_>| match context.action {
                AuthAction::Transaction {
                    operation: TransactionOperation::Rollback,
                } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .unwrap();
        worker_pool.run_worker(connection);
    });

    // When
    let result = pool
        .submit(|connection| {
            connection.execute_batch("BEGIN").unwrap();
            Ok(())
        })
        .await;
    pool.close();
    worker.join().unwrap();

    // Then
    assert!(matches!(
        result,
        Err(LocalEventQueryError::StorageAccessRequired { .. })
    ));
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
        assert_eq!(
            crate::adaptor::gateway::shared::sqlite_failure::condition(&error),
            crate::adaptor::gateway::shared::sqlite_failure::SqliteFailureCondition::Inaccessible
        );
        assert_eq!(
            storage_unavailable(&error).connect_code(),
            ErrorCode::FailedPrecondition
        );
    }
}

mod restored_memory_tests {
    use super::super::*;

    #[tokio::test]
    pub async fn test_読み込み待ち_実行前の期限切れでqueryを実行しない() {
        use crate::common::operation_context::Deadline;
        let pool = ReaderPool::new();
        let context =
            OperationContext::default().with_deadline(Deadline::new(std::time::Instant::now()));
        let result = crate::common::operation_context::scope(
            context,
            pool.submit(|_| -> Result<(), LocalEventQueryError> { panic!("expired query ran") }),
        )
        .await;
        assert_eq!(
            result,
            Err(LocalEventQueryError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                    message: "deadline exceeded".into()
                }
            ))
        );
    }
}
