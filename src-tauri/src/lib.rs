mod git;
mod pty;
mod search;
mod watcher;

use git::{
    get_current_branch, get_file_at_ref, get_git_log, get_git_status, get_repo_git_dir,
    get_staged_content, git_commit, git_create_branch, git_push, git_stage, git_stage_hunk,
    git_unstage, git_unstage_hunk, list_branches,
};
use pty::{kill_pty, resize_pty, spawn_pty, write_pty, PtyManager};
use search::{find_definition, find_references, search_files};
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
            get_repo_git_dir,
            get_git_status,
            get_git_log,
            get_current_branch,
            git_stage,
            git_unstage,
            git_stage_hunk,
            git_unstage_hunk,
            git_commit,
            git_push,
            git_create_branch,
            search_files,
            find_definition,
            find_references
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
