//! Domain error for workflow.
//!
//! Concrete external errors are converted to strings by adaptor/gateway
//! implementations. The domain layer must not depend on git2, tauri, tokio, or
//! filesystem-specific error types.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowError {
    #[error("{0}")]
    External(String),
    #[error("{0}")]
    Rule(String),
    #[error("validation_error: {0}")]
    Validation(String),
    #[error("invalid_state: {0}")]
    InvalidState(String),
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("already_active: {0}")]
    AlreadyActive(String),
    #[error("unauthorized_worktree: {0}")]
    UnauthorizedWorktree(String),
    #[error("unauthorized_approval_target: {0}")]
    UnauthorizedApprovalTarget(String),
}

impl WorkflowError {
    pub fn external(message: impl Into<String>) -> Self {
        Self::External(message.into())
    }

    pub fn rule(message: impl Into<String>) -> Self {
        Self::Rule(message.into())
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
