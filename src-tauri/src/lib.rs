mod config;
mod focus_tracker;
mod git;
mod git_host;
mod hook_listener;
mod menu;
mod protocol;
mod pty;
mod qr_code;
mod search;
mod sentry_integration;
mod shell_integration;
mod tls;
mod vpn_detect;
mod watcher;
mod webhook;
mod ws_bridge;
mod ws_server;

use std::collections::HashMap;
use std::sync::Arc;

use config::{load_or_create_config, AppConfig};
use protocol::AgentStateSync;
use tauri::Manager;
use tauri_plugin_aptabase::EventTracker;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _sentry_guard = sentry_integration::init_sentry();

    // aptabase プラグインの setup 内で tokio::spawn が呼ばれるため、
    // Tauri Builder 起動前に Tokio ランタイムを共有する必要がある。
    // ref: https://github.com/aptabase/tauri-plugin-aptabase/issues/22
    let _runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let _runtime_guard = _runtime.enter();
    tauri::async_runtime::set(_runtime.handle().clone());

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let _ = fix_path_env::fix();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(Arc::new(pty::PtyManager::default()))
        .manage(watcher::FileWatcherManager::default())
        .manage(Arc::new(ws_bridge::WsBroadcaster::default()))
        .manage(ws_server::WsServerHandle::default())
        .manage(Arc::new(git_host::PrCache::new()))
        .manage(Arc::new(git_host::PrDetailCache::new()))
        .manage(Arc::new(git_host::IssueCache::new()))
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path)
                .map_err(|e| format!("設定ファイルの読み込みに失敗: {e}"))?;
            let telemetry_enabled = config.telemetry_enabled;

            app.handle()
                .plugin(tauri_plugin_aptabase::Builder::new("A-US-6336372584").build())?;

            let app_config = Arc::new(AppConfig::new(config, config_path));
            app.manage(app_config.clone());

            let agent_states: hook_listener::AgentStatesMap = Arc::new(parking_lot::Mutex::new(
                HashMap::<String, AgentStateSync>::new(),
            ));
            app.manage(agent_states.clone());

            let focus_tracker =
                Arc::new(parking_lot::Mutex::new(focus_tracker::FocusTracker::new()));

            let ft = focus_tracker.clone();
            let window = app.get_webview_window("main");
            if window.is_none() {
                log::warn!("Main window not found; focus tracking will be disabled");
            }
            if let Some(window) = window {
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(focused) = event {
                        let mut tracker = ft.lock();
                        if *focused {
                            tracker.on_focus();
                        } else {
                            tracker.on_blur();
                        }
                    }
                });
            }

            let hook_state = hook_listener::HookListenerState {
                app_config,
                app_handle: app.handle().clone(),
                broadcaster: app.state::<Arc<ws_bridge::WsBroadcaster>>().inner().clone(),
                agent_states,
                focus_tracker,
            };
            tauri::async_runtime::spawn(async move {
                if let Err(e) = hook_listener::start_hook_listener(hook_state).await {
                    log::error!("Hook listener failed to start: {e}");
                }
            });

            menu::setup_menu(app)?;

            if telemetry_enabled {
                let _ = app.track_event("app_started", None);
            }

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
            git::commands::get_file_at_ref,
            git::commands::get_staged_content,
            git::commands::get_binary_file_at_ref,
            git::commands::get_binary_staged_content,
            // Git: ブランチ
            git::commands::list_branches,
            git::commands::get_current_branch,
            git::commands::get_default_branch,
            git::commands::git_create_branch,
            git::commands::delete_branch,
            // Git: ステータス
            git::commands::get_git_status,
            git::commands::get_git_log,
            // Git: ステージング
            git::commands::git_stage,
            git::commands::git_unstage,
            git::commands::git_stage_hunk,
            git::commands::git_unstage_hunk,
            git::commands::git_discard,
            // Git: コミット・プッシュ
            git::commands::git_commit,
            git::commands::git_push,
            // Git: ワークツリー
            git::commands::get_main_repo_path,
            git::commands::get_worktree_dirty_count,
            git::commands::list_worktrees,
            git::commands::list_branches_with_status,
            git::commands::create_worktree,
            git::commands::remove_worktree,
            // Git: 設定・ユーティリティ
            git::commands::get_cwd,
            git::commands::get_repo_git_dir,
            git::commands::get_releash_base,
            git::commands::set_releash_base,
            // Git Host
            git_host::check_pr_provider_status,
            git_host::fetch_pr_status,
            git_host::get_cached_pr_status,
            git_host::get_pr_detail,
            git_host::fetch_issues,
            git_host::get_cached_issues,
            // 検索
            search::search_files,
            search::find_definition,
            search::find_references,
            // アプリ設定
            config::get_server_config,
            config::update_server_port,
            config::regenerate_token,
            config::generate_hooks_config,
            config::apply_hooks_config,
            config::get_hooks_status,
            config::update_telemetry_enabled,
            config::get_notify_config,
            config::update_notify_config,
            config::get_crash_reporting_enabled,
            config::update_crash_reporting,
            config::update_webhook_url,
            // Hook Listener
            hook_listener::get_agent_states,
            // ネットワーク
            vpn_detect::detect_vpn_tunnel,
            vpn_detect::get_network_info,
            qr_code::get_connection_qr,
            // WebSocket サーバー
            ws_server::commands::start_server,
            ws_server::commands::stop_server,
            ws_server::commands::get_server_status,
            ws_server::commands::get_server_info,
            ws_server::commands::broadcast_comments,
            ws_server::commands::update_server_repo_paths,
            // Menu
            menu::set_menu_items_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokio_runtime_context_is_available_after_setup() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        tauri::async_runtime::set(runtime.handle().clone());

        // aptabase 等のプラグインが tokio::spawn を直接呼ぶため、
        // スレッドローカルのランタイムコンテキストが必要
        let handle = tokio::spawn(async { 42 });
        let result = runtime.block_on(handle).unwrap();
        assert_eq!(result, 42);
    }
}
