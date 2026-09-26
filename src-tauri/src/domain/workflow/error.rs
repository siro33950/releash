//! Domain error for workflow.
//!
//! Concrete external errors are converted to strings by adaptor/gateway
//! implementations. The domain layer must not depend on git2, tauri, tokio, or
//! filesystem-specific error types.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    Technical(crate::domain::failure::TechnicalFailure),
    Store(crate::domain::failure::StorageFailure),
    External(String),
    Editor(crate::domain::external_editor::EditorError),
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
            Self::Technical(error) => error.fmt(f),
            Self::Store(failure) => failure.fmt(f),
            Self::External(msg) => f.write_str(msg),
            Self::Editor(error) => error.fmt(f),
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

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
