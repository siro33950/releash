#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqliteFailureCondition {
    Busy,
    Corrupt,
    Inaccessible,
    Other,
}

pub(crate) fn condition(error: &rusqlite::Error) -> SqliteFailureCondition {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            SqliteFailureCondition::Busy
        }
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            SqliteFailureCondition::Corrupt
        }
        Some(
            rusqlite::ErrorCode::PermissionDenied
            | rusqlite::ErrorCode::ReadOnly
            | rusqlite::ErrorCode::CannotOpen
            | rusqlite::ErrorCode::DiskFull
            | rusqlite::ErrorCode::OutOfMemory
            | rusqlite::ErrorCode::OperationInterrupted
            | rusqlite::ErrorCode::SystemIoFailure
            | rusqlite::ErrorCode::FileLockingProtocolFailed
            | rusqlite::ErrorCode::TooBig
            | rusqlite::ErrorCode::NoLargeFileSupport
            | rusqlite::ErrorCode::AuthorizationForStatementDenied,
        ) => SqliteFailureCondition::Inaccessible,
        _ => SqliteFailureCondition::Other,
    }
}
