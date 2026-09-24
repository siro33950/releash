use std::path::Path;
use std::sync::Arc;

use crate::adaptor::gateway::push::BackendPush;
use crate::adaptor::gateway::repository::watch::{
    canonicalize_event_path, generate_watcher_id, FileChangeEvent,
};
use crate::domain::repository::file_watcher::FileWatchGateway;
use crate::infrastructure::file_watcher::FileWatcherManager;

pub(crate) struct FileWatcherGateway {
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionPublisher>,
    manager: Arc<FileWatcherManager>,
    sink: std::sync::Arc<crate::infrastructure::push::PushSink>,
}

impl FileWatcherGateway {
    pub(crate) fn new(
        manager: Arc<FileWatcherManager>,
        sink: std::sync::Arc<crate::infrastructure::push::PushSink>,
    ) -> Self {
        Self {
            manager,
            sink,
            state_publisher: None,
        }
    }
    pub(crate) fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionPublisher,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }
}

impl FileWatchGateway for FileWatcherGateway {
    fn start(&self, path: &str) -> Result<u64, String> {
        let id = generate_watcher_id();
        let sink = self.sink.clone();
        self.manager
            .start_watching(id, path.to_string(), move |event| {
                BackendPush::FileChange(file_change_event_from_path(id, &event.path)).emit(&sink);
            })
    }
    fn start_tree(&self, path: &str) -> Result<u64, String> {
        let id = generate_watcher_id();
        let path = std::path::absolute(path).map_err(|error| error.to_string())?;
        let watch_path = path
            .ancestors()
            .find(|parent| parent.is_dir())
            .ok_or("No existing history directory ancestor")?
            .to_path_buf();
        let publisher = self
            .state_publisher
            .clone()
            .ok_or("State publisher unavailable")?;
        let changes = publisher.clone();
        self.manager.start_watching(
            id,
            watch_path.to_string_lossy().into_owned(),
            move |event| {
                if event.path.starts_with(&path) || path.starts_with(&event.path) {
                    publisher.invalidate(
                        crate::domain::state_subscription::StateChangeSource::ProviderHistory,
                    );
                }
            },
        )?;
        changes.invalidate(crate::domain::state_subscription::StateChangeSource::ProviderHistory);
        Ok(id)
    }

    fn stop(&self, watcher_id: u64) -> Result<(), String> {
        self.manager.stop_watching(watcher_id)
    }
}

pub(crate) fn file_change_event_from_path(watcher_id: u64, path: &Path) -> FileChangeEvent {
    FileChangeEvent {
        watcher_id,
        path: canonicalize_event_path(path),
        kind: "change".to_string(),
    }
}

#[cfg(test)]
#[path = "file_watcher_test.rs"]
mod file_watcher_tests;
