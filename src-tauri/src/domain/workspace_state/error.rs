#[derive(Debug)]
pub enum WorkspaceStateError {
    Message(String),
}

impl std::fmt::Display for WorkspaceStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for WorkspaceStateError {}

impl crate::domain::failure::ClassifiedFailure for WorkspaceStateError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::Message(_) => F::Internal,
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
