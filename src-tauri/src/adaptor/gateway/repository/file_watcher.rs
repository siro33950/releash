use std::path::Path;
use std::sync::Arc;

use crate::adaptor::gateway::push::BackendPush;
use crate::adaptor::gateway::repository::watch::{
    canonicalize_event_path, generate_watcher_id, FileChangeEvent,
};
use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
use crate::infrastructure::file_watcher::FileWatcherManager;

pub(crate) struct FileWatcherGateway {
    manager: Arc<FileWatcherManager>,
    sink: std::sync::Arc<dyn crate::usecase::push::PushOutput>,
}

impl FileWatcherGateway {
    pub(crate) fn new(
        manager: Arc<FileWatcherManager>,
        sink: std::sync::Arc<dyn crate::usecase::push::PushOutput>,
    ) -> Self {
        Self { manager, sink }
    }
}

impl FileWatchGateway for FileWatcherGateway {
    fn start(&self, path: &str) -> Result<u64, String> {
        let id = generate_watcher_id();
        let sink = self.sink.clone();
        self.manager
            .start_watching(id, path.to_string(), move |event| {
                BackendPush::FileChange(file_change_event_from_path(id, &event.path))
                    .emit(sink.as_ref());
            })
    }
    fn start_tree(&self, path: &str, on_change: WatchChangeHandler) -> Result<u64, String> {
        let id = generate_watcher_id();
        let path = std::path::absolute(path).map_err(|error| error.to_string())?;
        let watch_path = path
            .ancestors()
            .find(|parent| parent.is_dir())
            .ok_or("No existing watch directory ancestor")?
            .to_path_buf();
        let changed = on_change.clone();
        self.manager.start_watching(
            id,
            watch_path.to_string_lossy().into_owned(),
            move |event| {
                if event.path.starts_with(&path) || path.starts_with(&event.path) {
                    changed();
                }
            },
        )?;
        on_change();
        Ok(id)
    }

    fn release(&self, watcher_id: u64) {
        self.manager.release_watching(watcher_id);
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
