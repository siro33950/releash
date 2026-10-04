use std::sync::Arc;

use crate::adaptor::gateway::repository::watch::generate_watcher_id;
use crate::domain::repository::file_watcher::{FileWatchGateway, WatchChangeHandler};
use crate::infrastructure::file_watcher::FileWatcherManager;

pub(crate) struct FileWatcherGateway {
    manager: Arc<FileWatcherManager>,
}

impl FileWatcherGateway {
    pub(crate) fn new(manager: Arc<FileWatcherManager>) -> Self {
        Self { manager }
    }
}

impl FileWatchGateway for FileWatcherGateway {
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
            move |event| match event {
                Ok(event) if event.path.starts_with(&path) || path.starts_with(&event.path) => {
                    changed(Ok(()))
                }
                Ok(_) => {}
                Err(error) => changed(Err(error)),
            },
        )?;
        on_change(Ok(()));
        Ok(id)
    }

    fn stop(&self, watcher_id: u64) -> Result<(), String> {
        self.manager.stop_watching(watcher_id)
    }
}

#[cfg(test)]
#[path = "file_watcher_test.rs"]
mod file_watcher_tests;
