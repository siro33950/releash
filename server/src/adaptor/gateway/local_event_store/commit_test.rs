use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use connectrpc::ErrorCode;

#[test]
fn test_sqlite書込みの混雑と破損と状態不備を区別する() {
    // Given
    for (code, expected) in [
        (rusqlite::ffi::SQLITE_BUSY, ErrorCode::Unavailable),
        (rusqlite::ffi::SQLITE_LOCKED, ErrorCode::Unavailable),
        (rusqlite::ffi::SQLITE_CORRUPT, ErrorCode::DataLoss),
        (rusqlite::ffi::SQLITE_NOTADB, ErrorCode::DataLoss),
        (
            rusqlite::ffi::SQLITE_READONLY,
            ErrorCode::FailedPrecondition,
        ),
        (rusqlite::ffi::SQLITE_FULL, ErrorCode::FailedPrecondition),
        (rusqlite::ffi::SQLITE_ERROR, ErrorCode::Internal),
    ] {
        let source = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        // When
        let error = storage_unavailable(&source);
        // Then
        assert_eq!(error.connect_code(), expected);
    }
}
