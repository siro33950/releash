mod git;
mod pty;
mod watcher;

use git::{get_file_at_ref, get_repo_git_dir, get_staged_content, list_branches};
use pty::{kill_pty, resize_pty, spawn_pty, write_pty, PtyManager};
use watcher::{start_watching, stop_watching, FileWatcherManager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(PtyManager::default())
        .manage(FileWatcherManager::default())
        .invoke_handler(tauri::generate_handler![
            spawn_pty,
            write_pty,
            resize_pty,
            kill_pty,
            start_watching,
            stop_watching,
            get_file_at_ref,
            get_staged_content,
            list_branches,
            get_repo_git_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
