//! Domain error for workflow.
//!
//! Concrete external errors are converted to strings by adaptor/gateway
//! implementations. The domain layer must not depend on git2, tauri, tokio, or
//! filesystem-specific error types.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    Store(crate::domain::failure::FailureKind),
    External(String),
    Editor(crate::domain::external_editor::EditorError),
    StorageUnavailable {
        message: String,
        kind: crate::domain::failure::FailureKind,
    },
    CorruptStoredState(String),
    IncompatibleStoredEvent(String),
    Validation(String),
    Conflict(String),
    InvalidState(String),
    NotFound(String),
    UnauthorizedApprovalTarget(String),
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(kind) => write!(f, "Store failure: {kind:?}"),
            Self::External(msg) => f.write_str(msg),
            Self::Editor(error) => error.fmt(f),
            Self::StorageUnavailable { message, kind } => {
                write!(
                    f,
                    "storage_unavailable (retryable={}): {message}",
                    *kind == crate::domain::failure::FailureKind::Temporary
                )
            }
            Self::CorruptStoredState(message) => {
                write!(f, "corrupt_stored_state: {message}")
            }
            Self::IncompatibleStoredEvent(message) => {
                write!(f, "incompatible_stored_event: {message}")
            }
            Self::Validation(msg) => write!(f, "validation_error: {msg}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::InvalidState(msg) => write!(f, "invalid_state: {msg}"),
            Self::NotFound(msg) => write!(f, "not_found: {msg}"),
            Self::UnauthorizedApprovalTarget(msg) => {
                write!(f, "unauthorized_approval_target: {msg}")
            }
        }
    }
}

impl std::error::Error for WorkflowError {}

impl WorkflowError {
    pub fn external(message: impl Into<String>) -> Self {
        Self::External(message.into())
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState(message.into())
    }
}

#[cfg(test)]
mod workflow_error_tests {
    use super::*;

    #[test]
    fn test_workflow_error_display_keeps_legacy_prefixes() {
        assert_eq!(
            WorkflowError::validation("bad input").to_string(),
            "validation_error: bad input"
        );
        assert_eq!(
            WorkflowError::invalid_state("not waiting").to_string(),
            "invalid_state: not waiting"
        );
    }
}

impl crate::domain::failure::ClassifiedFailure for WorkflowError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::Store(kind) => *kind,
            Self::External(_) => F::Internal,
            Self::Editor(error) => error.failure_kind(),
            Self::StorageUnavailable { kind, .. } => *kind,
            Self::CorruptStoredState(_) => F::Corrupt,
            Self::IncompatibleStoredEvent(_) | Self::InvalidState(_) => F::StateRequired,
            Self::Validation(_) => F::InvalidInput,
            Self::Conflict(_) => F::RestartRequired,
            Self::NotFound(_) => F::Missing,
            Self::UnauthorizedApprovalTarget(_) => F::Permission,
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;

impl From<crate::domain::local_event::CommitBatchError> for WorkflowError {
    fn from(error: crate::domain::local_event::CommitBatchError) -> Self {
        use crate::domain::failure::ClassifiedFailure;
        Self::StorageUnavailable {
            message: format!("node fact append failed: {error}"),
            kind: error.failure_kind(),
        }
    }
}
