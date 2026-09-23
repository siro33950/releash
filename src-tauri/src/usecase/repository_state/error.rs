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
