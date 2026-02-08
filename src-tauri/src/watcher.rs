use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::protocol::{FileChange, GitFileStatusMsg, GitStatusSync, WsMessage};
use crate::ws_bridge::WsBroadcaster;

static WATCHER_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_watcher_id() -> u64 {
    WATCHER_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FileChangeEvent {
    pub watcher_id: u64,
    pub path: String,
    pub kind: String,
}

struct WatcherSession {
    _debouncer: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
}

#[derive(Default)]
pub struct FileWatcherManager {
    sessions: Mutex<HashMap<u64, WatcherSession>>,
}

#[tauri::command]
pub fn start_watching(
    app: AppHandle,
    state: State<'_, FileWatcherManager>,
    path: String,
) -> Result<u64, String> {
    let watcher_id = generate_watcher_id();
    let watch_path = PathBuf::from(&path);

    if !watch_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let app_clone = app.clone();
    let watcher_id_clone = watcher_id;
    let watch_path_str = path.clone();

    let debouncer = new_debouncer(
        Duration::from_millis(100),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| {
            match res {
                Ok(events) => {
                    for event in events {
                        let kind = match event.kind {
                            DebouncedEventKind::Any => "change",
                            DebouncedEventKind::AnyContinuous => "change",
                            _ => "change",
                        };
                        let event_path = event.path.to_string_lossy().to_string();
                        let _ = app_clone.emit(
                            "file-change",
                            FileChangeEvent {
                                watcher_id: watcher_id_clone,
                                path: event_path.clone(),
                                kind: kind.to_string(),
                            },
                        );

                        if let Some(ws) = app_clone.try_state::<std::sync::Arc<WsBroadcaster>>() {
                            ws.try_send(WsMessage::FileChange(FileChange {
                                path: event_path,
                                kind: kind.to_string(),
                            }));

                            if let Ok(statuses) = crate::git::get_git_status(watch_path_str.clone())
                            {
                                let files = statuses
                                    .into_iter()
                                    .map(|s| GitFileStatusMsg {
                                        path: s.path,
                                        index_status: s.index_status,
                                        worktree_status: s.worktree_status,
                                    })
                                    .collect();
                                ws.try_send(WsMessage::GitStatusSync(GitStatusSync { files }));
                            }
                        }
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

    state.sessions.lock().insert(watcher_id, session);

    Ok(watcher_id)
}

#[tauri::command]
pub fn stop_watching(state: State<'_, FileWatcherManager>, watcher_id: u64) -> Result<(), String> {
    let mut sessions = state.sessions.lock();
    sessions
        .remove(&watcher_id)
        .ok_or_else(|| format!("Watcher {} not found", watcher_id))?;
    Ok(())
}
