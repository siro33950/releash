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
    pub fn start_watching<F>(
        &self,
        watcher_id: u64,
        path: String,
        on_event: F,
    ) -> Result<u64, String>
    where
        F: Fn(Result<RawFileWatchEvent, String>) + Send + Sync + 'static,
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
                            on_event(Ok(RawFileWatchEvent { path: event.path }));
                        }
                    }
                    Err(e) => {
                        on_event(Err(e.to_string()));
                    }
                }
            },
        )
        .map_err(|e| format!("Failed to create debouncer: {}", e))?;

        let mut debouncer = debouncer;
        watch_tree(debouncer.watcher(), &watch_path)
            .map_err(|e| format!("Failed to watch path: {}", e))?;

        let session = WatcherSession {
            _debouncer: debouncer,
        };

        self.sessions.lock().insert(watcher_id, session);

        Ok(watcher_id)
    }

    pub fn stop_watching(&self, watcher_id: u64) -> Result<(), String> {
        if self.release_watching(watcher_id) {
            Ok(())
        } else {
            Err(format!("Watcher {} not found", watcher_id))
        }
    }

    fn release_watching(&self, watcher_id: u64) -> bool {
        let Some(session) = self.sessions.lock().remove(&watcher_id) else {
            return false;
        };
        // debouncer の drop はブロックし得るため sessions ロックの外・別スレッドで行う（#1641）
        crate::infrastructure::dispose::dispose_in_background("file-watcher-dispose", session);
        true
    }
}

pub(crate) fn watch_tree(
    watcher: &mut dyn notify_debouncer_mini::notify::Watcher,
    path: &std::path::Path,
) -> notify_debouncer_mini::notify::Result<()> {
    use notify_debouncer_mini::notify::{Error, ErrorKind};

    let mut pending = vec![path.to_path_buf()];
    let mut seen = std::collections::HashSet::new();
    while let Some(directory) = pending.pop() {
        if let Err(error) = watcher.watch(&directory, RecursiveMode::Recursive) {
            if !cfg!(target_os = "linux")
                || !matches!(&error.kind, ErrorKind::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied)
            {
                return Err(error);
            }
            let canonical = std::fs::canonicalize(&directory).map_err(Error::io)?;
            if !seen.insert(canonical) {
                continue;
            }
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error)
                    if directory != path
                        && error.kind() == std::io::ErrorKind::PermissionDenied =>
                {
                    continue;
                }
                Err(error) => return Err(Error::io(error)),
            };
            // inotify は失敗した子より前の監視を保持するため、残りの読める枝を登録する。
            for entry in entries {
                let entry = entry.map_err(Error::io)?;
                if entry.path().is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(())
}
