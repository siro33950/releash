//! Domain error for workflow.
//!
//! Concrete external errors are converted to strings by adaptor/gateway
//! implementations. The domain layer must not depend on git2, tauri, tokio, or
//! filesystem-specific error types.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    External(String),
    Rule(String),
    Validation(String),
    InvalidState(String),
    NotFound(String),
    AlreadyActive(String),
    UnauthorizedWorktree(String),
    UnauthorizedApprovalTarget(String),
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::External(msg) | Self::Rule(msg) => f.write_str(msg),
            Self::Validation(msg) => write!(f, "validation_error: {msg}"),
            Self::InvalidState(msg) => write!(f, "invalid_state: {msg}"),
            Self::NotFound(msg) => write!(f, "not_found: {msg}"),
            Self::AlreadyActive(msg) => write!(f, "already_active: {msg}"),
            Self::UnauthorizedWorktree(msg) => write!(f, "unauthorized_worktree: {msg}"),
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
