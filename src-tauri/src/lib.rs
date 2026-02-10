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
mod ws_bridge;
mod ws_server;

use std::sync::Arc;

use config::{load_or_create_config, AppConfig};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(pty::PtyManager::default()))
        .manage(watcher::FileWatcherManager::default())
        .manage(Arc::new(ws_bridge::WsBroadcaster::default()))
        .manage(ws_server::WsServerHandle::default())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("app_data_dir の取得に失敗");
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path).expect("設定ファイルの読み込みに失敗");
            app.manage(Arc::new(AppConfig::new(config, config_path)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // PTY
            pty::spawn_pty,
            pty::write_pty,
            pty::resize_pty,
            pty::kill_pty,
            pty::list_pty_sessions,
            pty::get_or_spawn_pty,
            pty::kill_ptys_by_worktree,
            // ファイル監視
            watcher::start_watching,
            watcher::start_git_dir_watching,
            watcher::stop_watching,
            // Git: diff/content
            git::diff::get_file_at_ref,
            git::diff::get_staged_content,
            // Git: ブランチ
            git::branch::list_branches,
            git::branch::get_current_branch,
            git::branch::get_default_branch,
            git::branch::git_create_branch,
            // Git: ステータス
            git::status::get_git_status,
            git::log::get_git_log,
            // Git: ステージング
            git::stage::git_stage,
            git::stage::git_unstage,
            git::stage::git_stage_hunk,
            git::stage::git_unstage_hunk,
            // Git: コミット・プッシュ
            git::commit::git_commit,
            git::commit::git_push,
            // Git: ワークツリー
            git::worktree::get_main_repo_path,
            git::worktree::get_worktree_dirty_count,
            git::worktree::list_worktrees,
            git::worktree::list_branches_with_status,
            git::worktree::create_worktree,
            git::worktree::remove_worktree,
            // Git: 設定・ユーティリティ
            git::util::get_cwd,
            git::util::get_repo_git_dir,
            git::config::get_releash_base,
            git::config::set_releash_base,
            // 検索
            search::search_files,
            search::find_definition,
            search::find_references,
            // アプリ設定
            config::get_server_config,
            config::update_server_port,
            config::regenerate_token,
            // ネットワーク
            vpn_detect::detect_vpn_tunnel,
            vpn_detect::get_network_info,
            qr_code::get_connection_qr,
            // WebSocket サーバー
            ws_server::commands::start_server,
            ws_server::commands::stop_server,
            ws_server::commands::get_server_status,
            ws_server::commands::broadcast_comments,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
