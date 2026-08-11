use std::path::Path;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::adaptor::controller::state::AppState;
use crate::adaptor::gateway::repository::watch::{
    canonicalize_event_path, generate_watcher_id, FileChangeEvent,
};
use crate::infrastructure::file_watcher::FileWatcherManager;

pub(super) const COMMAND_NAMES: &[&str] =
    &["start_watching", "start_git_dir_watching", "stop_watching"];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(COMMAND_NAMES, Box::new(invoke_handler()));
}

pub(crate) fn invoke_handler(
) -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![start_watching, start_git_dir_watching, stop_watching]
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
    let app_clone = app.clone();
    state.start_watching(watcher_id, path, move |event| {
        let event = file_change_event_from_path(watcher_id, &event.path);
        let _ = app_clone.emit("file-change", event);
    })
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

    state.stop_watching(watcher_id)
}

fn file_change_event_from_path(watcher_id: u64, path: &Path) -> FileChangeEvent {
    let event_path = canonicalize_event_path(path);
    FileChangeEvent {
        watcher_id,
        path: event_path,
        kind: "change".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_change_event_canonicalizes_existing_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "content").unwrap();

        let event = file_change_event_from_path(42, &file_path);

        assert_eq!(event.watcher_id, 42);
        assert_eq!(event.kind, "change");
        assert_eq!(
            event.path,
            file_path
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        );
    }

    #[test]
    fn file_change_event_preserves_deleted_file_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("deleted.txt");

        let event = file_change_event_from_path(7, &file_path);

        assert_eq!(event.watcher_id, 7);
        assert!(event.path.ends_with("deleted.txt"));
        assert!(!event.path.contains(".."));
    }

    #[test]
    fn file_change_event_fallback_returns_forward_slash_path() {
        let event = file_change_event_from_path(8, Path::new(r"C:\missing\file.txt"));

        assert_eq!(event.path, "C:/missing/file.txt");
    }
}
