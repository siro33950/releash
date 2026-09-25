use notify_debouncer_mini::new_debouncer;
use notify_debouncer_mini::notify::RecursiveMode;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

struct WatcherSession {
    _debouncer: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
}

pub struct RawFileWatchEvent {
    pub path: PathBuf,
}

#[derive(Default)]
pub struct FileWatcherManager {
    sessions: Mutex<HashMap<u64, WatcherSession>>,
}

impl FileWatcherManager {
    pub(crate) fn start_watching<F>(
        &self,
        watcher_id: u64,
        path: String,
        on_event: F,
    ) -> Result<u64, String>
    where
        F: Fn(RawFileWatchEvent) + Send + Sync + 'static,
    {
        let watch_path = PathBuf::from(&path);

        if !watch_path.exists() {
            return Err(format!("Path does not exist: {}", path));
        }

        let debouncer = new_debouncer(
            Duration::from_millis(100),
            move |res: Result<
                Vec<notify_debouncer_mini::DebouncedEvent>,
                notify_debouncer_mini::notify::Error,
            >| {
                match res {
                    Ok(events) => {
                        for event in events {
                            on_event(RawFileWatchEvent { path: event.path });
                        }
                    }
                    Err(e) => {
                        eprintln!("File watcher error: {:?}", e);
                    }
                }
            },
        )
        .map_err(|e| format!("Failed to create debouncer: {}", e))?;

        let mut debouncer = debouncer;
        debouncer
            .watcher()
            .watch(&watch_path, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch path: {}", e))?;

        let session = WatcherSession {
            _debouncer: debouncer,
        };

        self.sessions.lock().insert(watcher_id, session);

        Ok(watcher_id)
    }

    pub(crate) fn stop_watching(&self, watcher_id: u64) -> Result<(), String> {
        if self.release_watching(watcher_id) {
            Ok(())
        } else {
            Err(format!("Watcher {} not found", watcher_id))
        }
    }

    pub(crate) fn release_watching(&self, watcher_id: u64) -> bool {
        let Some(session) = self.sessions.lock().remove(&watcher_id) else {
            return false;
        };
        // debouncer の drop はブロックし得るため sessions ロックの外・別スレッドで行う（#1641）
        crate::infrastructure::dispose::dispose_in_background("file-watcher-dispose", session);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_watching_removes_session_and_errors_on_unknown_id() {
        let manager = FileWatcherManager::default();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let id = manager.start_watching(1, path, |_| {}).unwrap();
        assert_eq!(id, 1);

        manager.stop_watching(1).unwrap();
        assert!(manager.stop_watching(1).is_err());
    }
}
