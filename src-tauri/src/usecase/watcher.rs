use std::sync::Arc;

use crate::domain::repository::file_watcher::FileWatchGateway;
use crate::usecase::repository_state::RepositoryStateService;

#[derive(Debug, thiserror::Error)]
pub enum UsecaseError {
    #[error("{0}")]
    Repository(crate::usecase::repository_state::RepositoryStateError),
    #[error("{0}")]
    File(String),
    #[error("Repository state unavailable")]
    RepositoryUnavailable,
}

pub struct WatcherUsecase {
    repository: Option<Arc<RepositoryStateService>>,
    files: Arc<dyn FileWatchGateway>,
}

impl WatcherUsecase {
    pub fn new(
        repository: Option<Arc<RepositoryStateService>>,
        files: Arc<dyn FileWatchGateway>,
    ) -> Self {
        Self { repository, files }
    }

    pub(crate) fn start_files(
        &self,
        path: &str,
        on_change: crate::domain::repository::file_watcher::WatchChangeHandler,
    ) -> Result<u64, UsecaseError> {
        self.files
            .start_tree(path, on_change)
            .map_err(UsecaseError::File)
    }

    pub fn start_git_dir(&self, path: &str) -> Result<u64, UsecaseError> {
        self.repository
            .as_ref()
            .ok_or(UsecaseError::RepositoryUnavailable)?
            .start_git_dir_watching(path)
            .map_err(UsecaseError::Repository)
    }

    pub fn stop(&self, watcher_id: u64) -> Result<(), UsecaseError> {
        if let Some(repository) = &self.repository {
            if repository
                .stop_watching(watcher_id)
                .map_err(UsecaseError::Repository)?
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

#[cfg(any(test, feature = "test-support"))]
#[path = "watcher_test_helpers.rs"]
pub(crate) mod watcher_test_helpers;
