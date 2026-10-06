use super::*;

#[test]
fn test_node事実追記_sqliteの理由をconnectまで保持する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode;
    // Given
    for (code, kind, wire_code) in [
        (
            rusqlite::ffi::SQLITE_BUSY,
            ErrorCode::Unavailable,
            ErrorCode::Unavailable,
        ),
        (
            rusqlite::ffi::SQLITE_LOCKED,
            ErrorCode::Unavailable,
            ErrorCode::Unavailable,
        ),
        (
            rusqlite::ffi::SQLITE_CORRUPT,
            ErrorCode::DataLoss,
            ErrorCode::DataLoss,
        ),
        (
            rusqlite::ffi::SQLITE_NOTADB,
            ErrorCode::DataLoss,
            ErrorCode::DataLoss,
        ),
        (
            rusqlite::ffi::SQLITE_READONLY,
            ErrorCode::FailedPrecondition,
            ErrorCode::FailedPrecondition,
        ),
        (
            rusqlite::ffi::SQLITE_FULL,
            ErrorCode::FailedPrecondition,
            ErrorCode::FailedPrecondition,
        ),
        (
            rusqlite::ffi::SQLITE_ERROR,
            ErrorCode::Internal,
            ErrorCode::Internal,
        ),
    ] {
        let source = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        // When
        let error =
            crate::adaptor::gateway::local_event_store::commit::storage_unavailable(&source);
        let workflow = crate::domain::workflow::WorkflowError::from(error.clone());
        // Then
        assert_eq!(error.connect_code(), kind);
        assert_eq!(workflow.connect_code(), kind);
        assert_eq!(
            crate::adaptor::presenter::connect::classified_error(workflow).code,
            wire_code
        );
    }
}

#[test]
fn test_node事実追記_結果不明と競合と混雑を分類する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use crate::domain::local_event::CommitBatchError;
    use connectrpc::ErrorCode;
    // Given / When / Then
    for (error, expected) in [
        (CommitBatchError::TreeHeadConflict, ErrorCode::Aborted),
        (CommitBatchError::AppendOutcomeUnknown, ErrorCode::Aborted),
        (CommitBatchError::QueueBusy, ErrorCode::Unavailable),
        (
            crate::domain::local_event::CommitBatchError::StorageUnavailable {
                failure: crate::domain::local_event::SafeOperationFailure::new(
                    crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                    crate::domain::failure::TechnicalFailureNature::TimedOut,
                    "expired",
                    "test",
                ),
            },
            ErrorCode::DeadlineExceeded,
        ),
    ] {
        assert_eq!(error.connect_code(), expected);
    }
}

#[test]
fn test_store起動失敗_sqliteの性質が書込と読取で一致する() {
    use crate::domain::failure::{
        StorageFailure,
        TechnicalFailureNature::{Other, Transient},
    };
    use crate::domain::local_event::CommitBatchError;
    // Given
    for (code, expected) in [
        (rusqlite::ffi::SQLITE_BUSY, Transient),
        (rusqlite::ffi::SQLITE_PERM, Other),
        (rusqlite::ffi::SQLITE_IOERR, Other),
    ] {
        let error = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        // When
        let LocalEventStoreOpenError::StorageUnavailable(startup) =
            super::sqlite_open_failure(&error)
        else {
            panic!("expected storage unavailable")
        };
        let write = crate::adaptor::gateway::local_event_store::commit::storage_unavailable(&error);
        let read = StorageFailure::from(
            crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error),
        );
        // Then
        let (CommitBatchError::StorageUnavailable { failure }
        | CommitBatchError::StorageAccessRequired { failure }) = write
        else {
            panic!("expected storage write failure")
        };
        assert_eq!(startup.nature, expected);
        assert_eq!(failure.nature, expected);
        assert_eq!(read.nature, expected);
    }
}

#[test]
fn only_lock_contention_is_store_in_use() {
    assert_eq!(
        classify_writer_lock_error(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
        LocalEventStoreOpenError::WriterLockHeld
    );
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::StorageFull,
        std::io::ErrorKind::Other,
    ] {
        assert_eq!(
            classify_writer_lock_error(std::io::Error::from(kind)),
            io_open_failure(std::io::Error::from(kind))
        );
    }
}
