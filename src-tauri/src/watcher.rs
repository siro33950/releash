use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::gateway::repository::watch::{
    canonicalize_event_path, generate_watcher_id, FileChangeEvent,
};
use crate::usecase::agent_session::context::invalidate_instruction_resolution_cache_for_path;

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
    if let Some(app_state) = app.try_state::<AppState>() {
        match app_state
            .repository_state
            .start_file_watching_if_repository(&path)
        {
            Ok(Some(watcher_id)) => return Ok(watcher_id),
            Ok(None) => {}
            Err(err) => return Err(err.to_string()),
        }
    }

    let watcher_id = generate_watcher_id();
    let watch_path = PathBuf::from(&path);

    if !watch_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let app_clone = app.clone();
    let watcher_id_clone = watcher_id;

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
                        let event_path = canonicalize_event_path(&event.path)
                            .unwrap_or_else(|| event.path.to_string_lossy().to_string());
                        invalidate_instruction_resolution_cache_for_path(Path::new(&event_path));
                        let _ = app_clone.emit(
                            "file-change",
                            FileChangeEvent {
                                watcher_id: watcher_id_clone,
                                path: event_path,
                                kind: kind.to_string(),
                            },
                        );
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
pub fn start_git_dir_watching(
    app: AppHandle,
    _state: State<'_, FileWatcherManager>,
    repo_path: String,
) -> Result<u64, String> {
    app.state::<AppState>()
        .repository_state
        .start_git_dir_watching(&repo_path)
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub fn stop_watching(
    app: AppHandle,
    state: State<'_, FileWatcherManager>,
    watcher_id: u64,
) -> Result<(), String> {
    if let Some(app_state) = app.try_state::<AppState>() {
        match app_state.repository_state.stop_watching(watcher_id) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(err) => return Err(err.to_string()),
        }
    }

    let mut sessions = state.sessions.lock();
    sessions
        .remove(&watcher_id)
        .ok_or_else(|| format!("Watcher {} not found", watcher_id))?;
    Ok(())
}
