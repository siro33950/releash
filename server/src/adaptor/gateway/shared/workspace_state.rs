use crate::domain::failure::{StorageFailure, TechnicalFailure, TechnicalFailureNature};
use crate::domain::local_event::{CommitBatchError, LocalEventQueryError};
use crate::domain::workspace_state::WorkspaceStateError;

pub(crate) fn unexpected(error: impl std::fmt::Display) -> WorkspaceStateError {
    WorkspaceStateError::Technical(TechnicalFailure {
        nature: TechnicalFailureNature::Other,
        message: error.to_string(),
    })
}

impl From<StorageFailure> for WorkspaceStateError {
    fn from(error: StorageFailure) -> Self {
        Self::Technical(TechnicalFailure {
            nature: error.nature,
            message: error.to_string(),
        })
    }
}

impl From<LocalEventQueryError> for WorkspaceStateError {
    fn from(error: LocalEventQueryError) -> Self {
        match error {
            LocalEventQueryError::Technical(failure) => Self::Technical(failure),
            error => StorageFailure::from(error).into(),
        }
    }
}

impl From<CommitBatchError> for WorkspaceStateError {
    fn from(error: CommitBatchError) -> Self {
        match error {
            CommitBatchError::StreamHeadConflict { .. } => Self::VersionConflict,
            CommitBatchError::Technical(failure) => Self::Technical(failure),
            error => StorageFailure::from(error).into(),
        }
    }
}

impl From<std::io::Error> for WorkspaceStateError {
    fn from(error: std::io::Error) -> Self {
        Self::Technical(error.into())
    }
}

impl From<serde_json::Error> for WorkspaceStateError {
    fn from(error: serde_json::Error) -> Self {
        match error.io_error_kind() {
            Some(kind) => Self::Technical(TechnicalFailure {
                nature: super::background_io::nature(&std::io::Error::from(kind)),
                message: error.to_string(),
            }),
            None => unexpected(error),
        }
    }
}

impl From<tokio::task::JoinError> for WorkspaceStateError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::Technical(error.into())
    }
}

#[cfg(test)]
#[path = "workspace_state_test.rs"]
mod workspace_state_tests;
