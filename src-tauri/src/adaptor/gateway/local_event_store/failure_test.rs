use super::reader::storage_unavailable;
use crate::adaptor::presenter::connect::ConnectFailure;
use crate::domain::agent_session::repository::AgentSessionRepositoryError;
use crate::domain::local_event::LocalEventQueryError;
use crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError;
use connectrpc::ErrorCode;

#[test]
fn test_sqliteの混雑と破損と状態不備を発生元で区別する() {
    // Given / When / Then
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
        let sqlite = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        let error = storage_unavailable(&sqlite);
        assert_eq!(error.connect_code(), expected);
        assert_eq!(
            AgentSessionRepositoryError::from(error.clone()).connect_code(),
            expected
        );
        assert_eq!(
            ProviderLifecycleRepositoryError::from(error).connect_code(),
            expected
        );
    }
}

#[test]
fn test_storeの期限切れを各repositoryが混雑へ潰さない() {
    // Given / When / Then
    assert_eq!(
        AgentSessionRepositoryError::from(LocalEventQueryError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into()
            }
        ))
        .connect_code(),
        ErrorCode::DeadlineExceeded
    );
    assert_eq!(
        ProviderLifecycleRepositoryError::from(LocalEventQueryError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into()
            }
        ))
        .connect_code(),
        ErrorCode::DeadlineExceeded
    );
}
