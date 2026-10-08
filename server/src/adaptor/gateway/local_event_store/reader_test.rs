use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use connectrpc::ErrorCode;

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
