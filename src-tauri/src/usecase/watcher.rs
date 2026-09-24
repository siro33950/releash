use std::sync::Arc;

use crate::domain::repository::file_watcher::FileWatchGateway;
use crate::domain::repository::watch_subscriptions::{WatchSubscriptionError, WatchSubscriptions};
use crate::usecase::repository_state::RepositoryStateService;

#[derive(Debug, thiserror::Error)]
pub(crate) enum UsecaseError {
    #[error(transparent)]
    Subscription(#[from] WatchSubscriptionError),
    #[error("{0}")]
    Repository(crate::usecase::repository_state::RepositoryStateError),
    #[error("{0}")]
    File(String),
    #[error("Repository state unavailable")]
    RepositoryUnavailable,
}

pub(crate) struct WatcherUsecase {
    repository: Option<Arc<RepositoryStateService>>,
    files: Arc<dyn FileWatchGateway>,
    subscriptions: parking_lot::Mutex<WatchSubscriptions>,
}

impl WatcherUsecase {
    pub(crate) fn new(
        repository: Option<Arc<RepositoryStateService>>,
        files: Arc<dyn FileWatchGateway>,
    ) -> Self {
        Self {
            repository,
            files,
            subscriptions: Default::default(),
        }
    }

    pub(crate) fn start(&self, path: &str) -> Result<u64, UsecaseError> {
        if let Some(repository) = &self.repository {
            if let Some(id) = repository
                .start_file_watching_if_repository(path)
                .map_err(UsecaseError::Repository)?
            {
                return Ok(id);
            }
        }
        self.files.start(path).map_err(UsecaseError::File)
    }

    pub(crate) fn start_files(&self, path: &str) -> Result<u64, UsecaseError> {
        self.files.start_tree(path).map_err(UsecaseError::File)
    }

    pub(crate) fn start_git_dir(&self, path: &str) -> Result<u64, UsecaseError> {
        self.repository
            .as_ref()
            .ok_or(UsecaseError::RepositoryUnavailable)?
            .start_git_dir_watching(path)
            .map_err(UsecaseError::Repository)
    }

    pub(crate) fn stop(&self, watcher_id: u64) -> Result<(), UsecaseError> {
        self.stop_backend(watcher_id)?;
        self.subscriptions.lock().stopped(watcher_id);
        Ok(())
    }

    fn stop_backend(&self, watcher_id: u64) -> Result<(), UsecaseError> {
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
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        id: String,
    ) -> Result<WatcherSubscription, UsecaseError> {
        self.subscriptions.lock().subscribe(id.clone())?;
        Ok(WatcherSubscription {
            id,
            usecase: self.clone(),
        })
    }

    pub(crate) fn watch(
        &self,
        subscription: &str,
        path: &str,
        git: bool,
    ) -> Result<u64, UsecaseError> {
        let reservation = self.subscriptions.lock().reserve(subscription)?;
        let result = if git {
            self.start_git_dir(path)
        } else {
            self.start(path)
        };
        let registered = self
            .subscriptions
            .lock()
            .complete(reservation, result.as_ref().ok().copied());
        let id = result?;
        if let Err(error) = registered {
            self.stop_backend(id)?;
            return Err(error.into());
        }
        Ok(id)
    }

    fn stop_watchers(&self, watchers: std::collections::HashSet<u64>) {
        for id in watchers {
            if let Err(error) = self.stop_backend(id) {
                log::error!("Watcher cleanup failed: {error}");
            }
        }
    }
}

#[cfg(test)]
#[path = "watcher_test.rs"]
pub(crate) mod watcher_tests;

pub(crate) struct WatcherSubscription {
    id: String,
    usecase: Arc<WatcherUsecase>,
}
impl Drop for WatcherSubscription {
    fn drop(&mut self) {
        let usecase = self.usecase.clone();
        let watchers = usecase.subscriptions.lock().unsubscribe(&self.id);
        tokio::task::spawn_blocking(move || usecase.stop_watchers(watchers));
    }
}

impl crate::domain::failure::ClassifiedFailure for UsecaseError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        use crate::domain::failure::FailureKind;
        match self {
            Self::Subscription(error) => error.failure_kind(),
            Self::Repository(error) => error.failure_kind(),
            Self::File(_) => FailureKind::Internal,
            Self::RepositoryUnavailable => FailureKind::StateRequired,
        }
    }
}
