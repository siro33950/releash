use std::sync::Arc;

use crate::domain::repository::file_watcher::FileWatchGateway;
use crate::usecase::repository_state::RepositoryStateService;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum UsecaseError {
    #[error("{0}")]
    Repository(String),
    #[error("{0}")]
    File(String),
    #[error("Repository state unavailable")]
    RepositoryUnavailable,
}

pub(crate) struct WatcherUsecase {
    repository: Option<Arc<RepositoryStateService>>,
    files: Arc<dyn FileWatchGateway>,
}

impl WatcherUsecase {
    pub(crate) fn new(
        repository: Option<Arc<RepositoryStateService>>,
        files: Arc<dyn FileWatchGateway>,
    ) -> Self {
        Self { repository, files }
    }

    pub(crate) fn start(&self, path: &str) -> Result<u64, UsecaseError> {
        if let Some(repository) = &self.repository {
            if let Some(id) = repository
                .start_file_watching_if_repository(path)
                .map_err(|error| UsecaseError::Repository(error.to_string()))?
            {
                return Ok(id);
            }
        }
        self.files.start(path).map_err(UsecaseError::File)
    }

    pub(crate) fn start_git_dir(&self, path: &str) -> Result<u64, UsecaseError> {
        self.repository
            .as_ref()
            .ok_or(UsecaseError::RepositoryUnavailable)?
            .start_git_dir_watching(path)
            .map_err(|error| UsecaseError::Repository(error.to_string()))
    }

    pub(crate) fn stop(&self, watcher_id: u64) -> Result<(), UsecaseError> {
        if let Some(repository) = &self.repository {
            if repository
                .stop_watching(watcher_id)
                .map_err(|error| UsecaseError::Repository(error.to_string()))?
            {
                return Ok(());
            }
        }
        self.files.stop(watcher_id).map_err(UsecaseError::File)
    }
}

#[cfg(test)]
#[path = "watcher_test.rs"]
mod watcher_tests;
