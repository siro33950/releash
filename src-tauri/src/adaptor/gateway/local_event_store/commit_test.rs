use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind};

#[test]
fn test_sqlite書込みの混雑と破損と状態不備を区別する() {
    // Given
    for (code, expected) in [
        (rusqlite::ffi::SQLITE_BUSY, FailureKind::Temporary),
        (rusqlite::ffi::SQLITE_LOCKED, FailureKind::Temporary),
        (rusqlite::ffi::SQLITE_CORRUPT, FailureKind::Corrupt),
        (rusqlite::ffi::SQLITE_NOTADB, FailureKind::Corrupt),
        (rusqlite::ffi::SQLITE_READONLY, FailureKind::StateRequired),
        (rusqlite::ffi::SQLITE_FULL, FailureKind::StateRequired),
        (rusqlite::ffi::SQLITE_ERROR, FailureKind::Internal),
    ] {
        let source = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        // When
        let error = storage_unavailable(&source);
        // Then
        assert_eq!(error.failure_kind(), expected);
    }
}
