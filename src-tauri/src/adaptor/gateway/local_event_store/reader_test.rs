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
