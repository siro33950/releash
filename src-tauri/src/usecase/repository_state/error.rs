use crate::usecase::code_error::CodeUsecaseError;
use crate::usecase::repository_error::UsecaseError;

#[derive(Debug, thiserror::Error)]
pub enum RepositoryStateError {
    #[error("Repository changed during rescan; retry the refresh")]
    ScanInvalidated,
    #[error(transparent)]
    Repository(#[from] UsecaseError),
    #[error(transparent)]
    Code(#[from] CodeUsecaseError),
    #[error("{0}")]
    Watcher(String),
}

impl crate::domain::failure::ClassifiedFailure for RepositoryStateError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind as F;
        match self {
            Self::ScanInvalidated => F::RestartRequired,
            Self::Repository(error) => error.failure_kind(),
            Self::Code(error) => error.failure_kind(),
            Self::Watcher(_) => F::Internal,
        }
    }
}

#[cfg(test)]
#[path = "error_test.rs"]
mod error_tests;
