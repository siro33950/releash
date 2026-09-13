use std::path::Path;
use std::sync::Arc;

use crate::adaptor::gateway::push::BackendPush;
use crate::adaptor::gateway::repository::watch::{
    canonicalize_event_path, generate_watcher_id, FileChangeEvent,
};
use crate::domain::repository::file_watcher::FileWatchGateway;
use crate::infrastructure::file_watcher::FileWatcherManager;

pub(crate) struct FileWatcherGateway<R: tauri::Runtime> {
    manager: Arc<FileWatcherManager>,
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> FileWatcherGateway<R> {
    pub(crate) fn new(manager: Arc<FileWatcherManager>, app: tauri::AppHandle<R>) -> Self {
        Self { manager, app }
    }
}

impl<R: tauri::Runtime> FileWatchGateway for FileWatcherGateway<R> {
    fn start(&self, path: &str) -> Result<u64, String> {
        let id = generate_watcher_id();
        let app = self.app.clone();
        self.manager
            .start_watching(id, path.to_string(), move |event| {
                BackendPush::FileChange(file_change_event_from_path(id, &event.path)).emit(&app);
            })
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
