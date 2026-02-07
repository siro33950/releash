mod config;
mod git;
mod protocol;
mod pty;
mod qr_code;
mod search;
mod shell_integration;
mod tls;
mod vpn_detect;
mod watcher;
mod webhook;
mod ws_bridge;
mod ws_server;

use std::sync::Arc;

use config::{get_server_config, load_or_create_config, regenerate_token, update_server_port, AppConfig};
use tauri::Manager;
use ws_server::{broadcast_comments, start_server, stop_server, get_server_status, WsServerHandle};
use git::{
    get_current_branch, get_file_at_ref, get_git_log, get_git_status, get_repo_git_dir,
    get_staged_content, git_commit, git_create_branch, git_push, git_stage, git_stage_hunk,
    git_unstage, git_unstage_hunk, list_branches,
};
use pty::{kill_pty, resize_pty, spawn_pty, write_pty, PtyManager};
use search::{find_definition, find_references, search_files};
use qr_code::get_connection_qr;
use vpn_detect::{detect_vpn_tunnel, get_network_info};
use watcher::{start_watching, stop_watching, FileWatcherManager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(PtyManager::default()))
        .manage(FileWatcherManager::default())
        .manage(Arc::new(ws_bridge::WsBroadcaster::default()))
        .manage(WsServerHandle::default())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app_data_dir の取得に失敗");
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path)
                .expect("設定ファイルの読み込みに失敗");
            app.manage(Arc::new(AppConfig::new(config, config_path)));
            Ok(())
        })
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
            find_references,
            get_server_config,
            update_server_port,
            regenerate_token,
            detect_vpn_tunnel,
            get_network_info,
            get_connection_qr,
            start_server,
            stop_server,
            get_server_status,
            broadcast_comments
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
