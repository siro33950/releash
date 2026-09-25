use super::reader::storage_unavailable;
use crate::domain::agent_session::repository::AgentSessionRepositoryError;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
use crate::domain::local_event::LocalEventQueryError;
use crate::domain::provider_lifecycle::ProviderLifecycleRepositoryError;

#[test]
fn test_sqliteの混雑と破損と状態不備を発生元で区別する() {
    // Given / When / Then
    for (code, expected) in [
        (rusqlite::ffi::SQLITE_BUSY, FailureKind::Temporary),
        (rusqlite::ffi::SQLITE_LOCKED, FailureKind::Temporary),
        (rusqlite::ffi::SQLITE_CORRUPT, FailureKind::Corrupt),
        (rusqlite::ffi::SQLITE_NOTADB, FailureKind::Corrupt),
        (rusqlite::ffi::SQLITE_READONLY, FailureKind::StateRequired),
        (rusqlite::ffi::SQLITE_FULL, FailureKind::StateRequired),
        (rusqlite::ffi::SQLITE_ERROR, FailureKind::Internal),
    ] {
        let sqlite = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None);
        let error = storage_unavailable(&sqlite);
        assert_eq!(error.failure_kind(), expected);
        assert_eq!(
            AgentSessionRepositoryError::from(error.clone()).failure_kind(),
            expected
        );
        assert_eq!(
            ProviderLifecycleRepositoryError::from(error).failure_kind(),
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
                kind: crate::domain::failure::FailureKind::Expired,
                message: "deadline exceeded".into()
            }
        ))
        .failure_kind(),
        FailureKind::Expired
    );
    assert_eq!(
        ProviderLifecycleRepositoryError::from(LocalEventQueryError::Technical(
            crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Expired,
                message: "deadline exceeded".into()
            }
        ))
        .failure_kind(),
        FailureKind::Expired
    );
}
