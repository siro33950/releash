mod agent_sdk;
mod agent_status;
mod config;
mod diff_comment_sender;
mod diff_comment_store;
mod external_editor;
mod file_mention;
mod focus_tracker;
mod git;
mod git_host;
mod mcp;
mod menu;
mod native_drop;
mod notion;
mod protocol;
mod pty;
mod qr_code;
mod repo_registry;
mod sentry_integration;
mod session;
mod shell_integration;
mod tls;
mod tray;
mod vpn_detect;
mod watcher;
mod webhook;
mod workflow;
mod workspace_state_store;
mod ws_bridge;
mod ws_server;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use config::{load_or_create_config, AppConfig};
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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(Arc::new(
            workspace_state_store::WorkspaceStateStore::default(),
        ))
        .manage(Arc::new(diff_comment_store::DiffCommentStore::default()))
        .manage(Arc::new(session::SessionStore::default()))
        .manage(Arc::new(pty::PtyManager::default()))
        .manage(watcher::FileWatcherManager::default())
        .manage(Arc::new(ws_bridge::WsBroadcaster::default()))
        .manage(Arc::new(tokio::sync::Mutex::new(
            agent_sdk::AgentProcessMap::new(),
        )))
        .manage(ws_server::WsServerHandle::default())
        .manage(Arc::new(git_host::PrCache::new()))
        .manage(Arc::new(git_host::IssueCache::new()))
        .manage(mcp::McpServerHandle::default())
        .manage::<repo_registry::SharedRepoPaths>(Arc::new(parking_lot::RwLock::new(Vec::new())))
        .setup(|app| {
            // OneShotPtyManager shares the same PtyManager instance
            let pty_mgr = app.state::<Arc<pty::PtyManager>>();
            app.manage(Arc::new(pty::oneshot::OneShotPtyManager::new(Arc::clone(
                pty_mgr.inner(),
            ))));

            let data_dir = app.path().app_data_dir()?;
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path)
                .map_err(|e| format!("設定ファイルの読み込みに失敗: {e}"))?;
            let telemetry_enabled = config.telemetry_enabled;

            app.handle()
                .plugin(tauri_plugin_aptabase::Builder::new("A-US-6336372584").build())?;

            let app_config = Arc::new(AppConfig::new(config, config_path));
            app.manage(app_config.clone());

            // Initialize shared repo_paths from config
            {
                let shared_repo_paths = app.state::<repo_registry::SharedRepoPaths>();
                if let Ok(cfg) = app_config.get_config() {
                    let paths: Vec<String> = cfg
                        .app
                        .last_repo_paths
                        .iter()
                        .filter(|p| !p.is_empty())
                        .cloned()
                        .collect();
                    *shared_repo_paths.write() = paths;
                }
            }

            let focus_tracker =
                Arc::new(parking_lot::Mutex::new(focus_tracker::FocusTracker::new()));
            app.manage(focus_tracker.clone());

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

            // AgentStatusCenter を構築・登録
            let broadcaster = app.state::<Arc<ws_bridge::WsBroadcaster>>().inner().clone();
            let agent_status_center = Arc::new(agent_status::AgentStatusCenter::new(
                app.handle().clone(),
                broadcaster,
            ));
            app.manage(agent_status_center);
            app.manage(Arc::new(workflow::engine::WorkflowEngine::new()));

            menu::setup_menu(app)?;
            tray::setup_tray(app)?;
            tray::listen_server_status(app.handle());

            if let Some(window) = app.get_webview_window("main") {
                native_drop::install(&window);
            }

            // --hidden flag: auto-launch scenario
            let is_hidden = std::env::args().any(|a| a == "--hidden");
            if is_hidden {
                let start_minimized = app_config.get_config().is_ok_and(|c| c.app.start_minimized);
                if start_minimized {
                    if let Some(window) = app.get_webview_window("main") {
                        let close_to_tray =
                            app_config.get_config().is_ok_and(|c| c.app.close_to_tray);
                        if close_to_tray {
                            let _ = window.hide();
                        } else {
                            let _ = window.minimize();
                        }
                    }
                }
            }

            // Auto-start server if configured
            let auto_start_config = app_config.get_config().ok().filter(|c| c.remote.auto_start);
            if let Some(cfg) = auto_start_config {
                let bind_ip = cfg.app.last_bind_ip.clone();
                if !bind_ip.is_empty() {
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) =
                            ws_server::commands::start_server_core(&handle, bind_ip).await
                        {
                            log::error!("Auto-start server failed: {e}");
                        }
                    });
                }
            }

            // Auto-start MCP server
            {
                let mcp_app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = mcp::auto_start_mcp_server(&mcp_app_handle).await {
                        log::error!("MCP server auto-start failed: {e}");
                    }
                });
            }

            // Clean up orphan agent processes from previous crashes.
            // Must complete before init_agent_sessions() to prevent killing newly spawned processes.
            #[cfg(unix)]
            {
                let data_dir_clone = data_dir.clone();
                let _ = std::thread::spawn(move || {
                    agent_sdk::cleanup_orphan_processes(&data_dir_clone);
                })
                .join();
            }

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
            pty::gc_ptys_for_worktree,
            // ファイル監視
            watcher::start_watching,
            watcher::start_git_dir_watching,
            watcher::stop_watching,
            // Git: diff/content
            git::commands::get_file_at_ref,
            git::commands::get_staged_content,
            git::commands::get_binary_staged_content,
            git::commands::get_file_at_branch_base,
            git::commands::get_binary_file_at_branch_base,
            git::commands::get_binary_file_at_ref,
            git::commands::get_branch_diff_summary,
            git::commands::build_diff_file_tree,
            git::commands::get_file_navigation,
            // Git: hunk/patch
            git::commands::compute_diff_hunks,
            git::commands::compute_hidden_ranges,
            git::commands::compute_hidden_ranges_from_content,
            git::commands::compute_visible_markdown_blocks,
            git::commands::generate_group_patch,
            git::commands::get_language_from_path,
            git::commands::get_relative_path,
            // Git: ブランチ
            git::commands::list_branches,
            git::commands::get_current_branch,
            git::commands::get_default_branch,
            git::commands::git_create_branch,
            git::commands::delete_branch,
            // Git: ステータス
            git::commands::get_git_status,
            git::commands::get_status_diff_stats,
            git::commands::get_git_log,
            // Git: ステージング
            git::commands::git_stage,
            git::commands::git_unstage,
            git::commands::git_stage_hunk,
            git::commands::git_unstage_hunk,
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
            git::commands::get_branch_base,
            git::commands::set_branch_base,
            // Git Host
            git_host::check_pr_provider_status,
            git_host::fetch_pr_status,
            git_host::get_cached_pr_status,
            git_host::fetch_issues,
            git_host::get_cached_issues,
            // Notion
            notion::query_notion_tasks,
            notion::fetch_notion_label_options,
            notion::save_notion_config,
            notion::get_notion_config,
            notion::delete_notion_config,
            notion::validate_notion_config,
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
            config::get_remote_config,
            config::update_remote_config,
            config::get_app_settings,
            config::update_app_settings,
            config::update_last_server_context,
            config::get_crash_reporting_enabled,
            config::update_crash_reporting,
            config::update_webhook_url,
            config::get_external_editor,
            config::update_external_editor,
            config::get_mcp_config,
            config::update_mcp_config,
            config::regenerate_mcp_token,
            // External Editor
            external_editor::detect_editors,
            external_editor::open_in_editor,
            external_editor::open_folder_in_editor,
            // Agent Status (Rust 中央管理)
            agent_status::get_session_status,
            agent_status::get_workspace_status,
            agent_status::list_workspace_statuses,
            agent_status::list_session_statuses,
            // ネットワーク
            vpn_detect::detect_vpn_tunnel,
            vpn_detect::get_network_info,
            qr_code::get_connection_qr,
            // WebSocket サーバー
            ws_server::commands::start_server,
            ws_server::commands::stop_server,
            ws_server::commands::get_server_status,
            ws_server::commands::get_server_info,
            ws_server::commands::update_terminal_startup_command,
            // Repo registry
            repo_registry::get_repo_paths,
            repo_registry::add_repo_path,
            repo_registry::remove_repo_path,
            // MCP Server
            mcp::start_mcp_server,
            mcp::stop_mcp_server,
            mcp::get_mcp_server_status,
            mcp::get_mcp_connection_info,
            mcp::mcp_json::get_configured_agents,
            mcp::mcp_json::remove_agent_mcp_config,
            mcp::mcp_json::save_and_generate_mcp_configs,
            mcp::mcp_json::generate_agent_mcp_config,
            mcp::mcp_json::preview_agent_mcp_config,
            // Workspace state
            workspace_state_store::load_workspace_state,
            workspace_state_store::save_workspace_state,
            // Diff comments
            diff_comment_store::load_diff_comments,
            diff_comment_store::add_diff_comment,
            diff_comment_store::update_diff_comment,
            diff_comment_store::delete_diff_comment,
            diff_comment_sender::send_diff_comments_to_agent,
            diff_comment_sender::mark_diff_comments_sent,
            // OneShot PTY
            pty::oneshot::spawn_oneshot_pty,
            pty::oneshot::cancel_oneshot_pty,
            pty::oneshot::get_oneshot_pty_status,
            pty::oneshot::list_oneshot_ptys,
            pty::oneshot::find_oneshot_pty,
            // File Mention
            file_mention::list_mentionable_files,
            // Agent SDK
            agent_sdk::start_agent_session,
            agent_sdk::execute_agent_query,
            agent_sdk::interrupt_agent_query,
            agent_sdk::close_agent_session,
            agent_sdk::set_agent_permission_mode,
            agent_sdk::set_agent_model,
            agent_sdk::respond_agent_permission,
            agent_sdk::send_agent_message,
            agent_sdk::init_agent_sessions,
            agent_sdk::scan_slash_commands,
            agent_sdk::prepare_image_attachment,
            agent_sdk::prepare_image_attachments_from_paths,
            // Session
            session::list_sessions,
            agent_sdk::get_session,
            session::create_session,
            session::close_session,
            session::restore_session,
            session::list_closed_sessions,
            session::add_message,
            session::update_session_state,
            session::update_session_agent_info,
            // Workflow
            workflow::commands::list_workflows,
            workflow::commands::get_workflow,
            workflow::commands::save_workflow,
            workflow::commands::delete_workflow,
            workflow::commands::open_workflow_in_editor,
            workflow::commands::start_workflow,
            workflow::commands::abort_workflow,
            workflow::commands::get_workflow_state,
            workflow::commands::approve_workflow_step,
            workflow::commands::complete_interactive_step,
            workflow::commands::list_workflow_executions,
            workflow::commands::get_workflow_execution_log,
            workflow::commands::get_workflow_execution_state,
            workflow::commands::list_facets,
            workflow::commands::get_facet,
            workflow::commands::save_facet,
            workflow::commands::delete_facet,
            // Menu
            menu::set_menu_items_enabled,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::WindowEvent {
                event: tauri::WindowEvent::CloseRequested { api, .. },
                label,
                ..
            } => {
                api.prevent_close();
                let close_to_tray = app_handle
                    .try_state::<Arc<AppConfig>>()
                    .and_then(|cfg| cfg.get_config().ok())
                    .is_none_or(|c| c.app.close_to_tray);

                if let Some(window) = app_handle.get_webview_window(&label) {
                    if close_to_tray {
                        let _ = window.hide();
                    } else {
                        let _ = window.minimize();
                    }
                }
            }
            tauri::RunEvent::ExitRequested { api, .. }
                if !tray::QUIT_REQUESTED.load(Ordering::SeqCst) =>
            {
                api.prevent_exit();
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        });
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
