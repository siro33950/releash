mod adaptor;
mod agent_commands;
mod agent_message_dispatcher;
mod agent_status_commands;
mod agent_status_events;
mod app_data_dir;
pub mod cli;
mod cli_install;
mod config;
mod domain;
mod external_editor;
mod focus_tracker;
mod git;
mod git_host;
mod infrastructure;
mod mcp;
mod menu;
mod native_drop;
mod notion;
mod other;
mod path_aliases;
mod permission;
mod protocol;
mod pty;
mod qr_code;
mod review_comments;
mod sentry_integration;
mod session_commands;
mod shell_integration;
mod tls;
mod tray;
mod usecase;
mod vpn_detect;
mod watcher;
mod webhook;
mod workflow;
mod workflow_state_events;
mod workflow_state_presenter;
mod workflow_step_lifecycle;
mod workflow_step_lifecycle_adapters;
mod workspace_state_store;
mod ws_bridge;
mod ws_server;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use config::{load_or_create_config, AppConfig};
use tauri::Manager;
use tauri_plugin_aptabase::EventTracker;
use tokio::sync::Mutex;

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

    let ws_broadcaster = Arc::new(ws_bridge::WsBroadcaster::default());

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
        .manage(Arc::new(review_comments::ReviewCommentStore::default()))
        .manage(Arc::new(
            usecase::agent_session::session::SessionStore::default(),
        ))
        .manage(Arc::new(pty::PtyManager::default()))
        .manage(watcher::FileWatcherManager::default())
        .manage(Arc::clone(&ws_broadcaster))
        .manage(Arc::new(tokio::sync::Mutex::new(
            infrastructure::agent_session::runtime::AgentProcessMap::new(),
        )))
        .manage(Arc::new(
            usecase::agent_session::session::OpenTabRegistry::default(),
        ))
        .manage(ws_server::WsServerHandle::default())
        .manage(Arc::new(git_host::PrCache::new()))
        .manage(Arc::new(git_host::IssueCache::new()))
        .manage(mcp::McpServerHandle::default())
        .manage::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>(Arc::new(
            parking_lot::RwLock::new(Vec::new()),
        ))
        .setup(|app| {
            // OneShotPtyManager shares the same PtyManager instance
            let pty_mgr = app.state::<Arc<pty::PtyManager>>();
            app.manage(Arc::new(pty::oneshot::OneShotPtyManager::new(Arc::clone(
                pty_mgr.inner(),
            ))));

            let data_dir = app.path().app_data_dir()?;
            // spec issues-1054 Implementation Freedom (L104): 別 Releash binary 由来の
            // RELEASH_DATA_DIR inherit (例: prod 版 Releash の Terminal Panel から起動
            // した shell から dev binary を起動した場合) を「ユーザー明示指定」と誤認しないよう、
            // 起動初期に env を自プロセス alias data_dir で正す。
            crate::path_aliases::ensure_release_data_dir_env_for_app(app.handle());
            cli_install::ensure_cli_symlink_installed();
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path)
                .map_err(|e| format!("設定ファイルの読み込みに失敗: {e}"))?;
            let telemetry_enabled = config.telemetry_enabled;

            app.handle()
                .plugin(tauri_plugin_aptabase::Builder::new("A-US-6336372584").build())?;

            let app_config = Arc::new(AppConfig::new(config, config_path));
            app.manage(app_config.clone());

            // Initialize shared repo_paths from config
            let shared_repo_paths = app
                .state::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>()
                .inner()
                .clone();
            {
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

            // repository ドメインの DI 配線（起動時に AppState を組み立てて manage）。
            // git ベースの usecase / query service はステートレス、repo_paths は
            // SharedRepoPaths + AppConfig を共有する。repository usecase は 1 度だけ
            // 組み立て、AppState・単体 State（workflow コマンド注入用）・watcher・
            // workflow リゾルバへ Arc 共有する（各エントリは注入で受け取る）。
            let repository_usecase =
                Arc::new(adaptor::controller::wiring::build_repository_usecase());
            app.manage(repository_usecase.clone());
            {
                use adaptor::controller::state::AppState;
                use adaptor::gateway::repository::repo_paths::RepoPathsGateway;
                use usecase::repo_paths_usecase::RepoPathsUsecase;

                let repo_paths_gateway =
                    RepoPathsGateway::new(shared_repo_paths.clone(), app_config.clone());
                // 変更通知（repo-paths-changed）の送信 infra を NotifyGateway として注入。
                let repo_paths_notifier = Arc::new(
                    adaptor::gateway::repository::notify::RepoPathsNotifyGateway::new(
                        app.handle().clone(),
                    ),
                );
                let repo_paths_usecase = Arc::new(RepoPathsUsecase::new(
                    Arc::new(repo_paths_gateway),
                    repo_paths_notifier,
                ));

                // code ドメインの DI 配線（gateway 実装はステートレス）。
                let code_usecase = Arc::new(adaptor::controller::wiring::build_code_usecase());

                app.manage(AppState {
                    repository_usecase: repository_usecase.clone(),
                    repo_paths_usecase,
                    code_usecase,
                });
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
            let agent_status_center =
                Arc::new(usecase::agent_session::status::AgentStatusCenter::new());
            // SessionStore の状態変更通知を購読して、保持している SessionStatus を
            // 最新化＋再集約する。Closed への遷移は aggregate でフィルタされ、
            // Closed → Idle の復帰では再び集約対象に戻る。
            {
                let center_for_listener = agent_status_center.clone();
                let app_for_status_listener = app.handle().clone();
                let broadcaster_for_status_listener =
                    app.state::<Arc<ws_bridge::WsBroadcaster>>().inner().clone();
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                session_store_state.register_state_change_listener(Arc::new(
                    move |session_id, _worktree_path, new_state| {
                        let changes = center_for_listener
                            .on_session_state_changed(session_id, new_state.clone());
                        agent_status_events::emit_agent_status_changes(
                            &app_for_status_listener,
                            Some(&broadcaster_for_status_listener),
                            changes,
                        );
                    },
                ));
            }
            app.manage(agent_status_center);
            let workflow_engine = Arc::new(workflow::engine::WorkflowEngine::new(
                Arc::new(workflow::resolver_adapters::DefaultWorkflowDefinitionResolver),
                Arc::new(
                    workflow::resolver_adapters::AppConfigManagedWorktreeResolver::new(
                        repository_usecase.clone(),
                        app_config.clone(),
                    ),
                ),
            ));
            // Run Store の永続化先（data_dir）を設定する。失敗しても起動は止めない。
            let pending_data_dir = app.path().app_data_dir().ok();
            if let Some(data_dir) = pending_data_dir.clone() {
                let engine_for_init = workflow_engine.clone();
                let app_handle_for_init = app.handle().clone();
                tauri::async_runtime::block_on(async move {
                    engine_for_init.set_run_store_data_dir(data_dir).await;
                    // 前回起動中に terminal event が書かれないまま終了した run を Aborted に
                    // 強制遷移させる。in-memory `executions` map が空のこのタイミングで 1 度だけ
                    // 走らせる（NDJSON 末尾の RunAborted append + metadata 更新）。
                    engine_for_init
                        .recover_orphan_runs(&app_handle_for_init)
                        .await;
                });
            }
            app.manage(workflow_engine);

            // AgentBackendRegistry を構築・登録
            let agent_handles = app
                .state::<Arc<Mutex<infrastructure::agent_session::runtime::AgentProcessMap>>>()
                .inner()
                .clone();
            let session_store = app
                .state::<Arc<usecase::agent_session::session::SessionStore>>()
                .inner()
                .clone();
            let registry = Arc::new(
                infrastructure::agent_session::runtime::build_registry_with_runtime(
                    app_config.clone(),
                    app.handle().clone(),
                    agent_handles,
                    session_store,
                ),
            );
            app.manage(registry.clone());

            // [06] CLI mutating CLI 経路の file watcher を起動する。初回 pickup は
            // setup 済みの engine / AgentBackendRegistry を前提に dispatch するため、
            // workflow 依存 state の登録完了後にだけ spawn する。
            //
            // data_dir が解決できなければ watcher は spawn せず、稼働中アプリでも CLI
            // pending command は pickup されない（spec [06] CLI 起動独立性境界:
            // それでも CLI 側の書き込み完了境界は保たれる）。
            if let Some(data_dir) = pending_data_dir {
                workflow::pending_command_watcher::spawn_pending_command_watcher(
                    app.handle().clone(),
                    data_dir.clone(),
                );

                // [issues-1022] CLI からの Thread / Comment 書き込みを UI へ
                // 反映するため、`<data_dir>/review-comments/` を file watch して
                // `review-comments-changed` を発火する。Tauri コマンド経由の
                // `emit_changed` とは別系統の通知経路で、CLI / Agent / 外部編集
                // 由来の書き込みも拾う。
                review_comments::spawn_review_comments_watcher(app.handle().clone(), data_dir);
            }

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
                    infrastructure::agent_session::runtime::cleanup_orphan_processes(
                        &data_dir_clone,
                    );
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
            // Git: diff/content（code ドメイン）
            adaptor::controller::command::code::file_content::get_file_at_ref,
            adaptor::controller::command::code::file_content::get_staged_content,
            adaptor::controller::command::code::file_content::get_binary_staged_content,
            adaptor::controller::command::code::file_content::get_file_at_branch_base,
            adaptor::controller::command::code::file_content::get_binary_file_at_branch_base,
            adaptor::controller::command::code::file_content::get_binary_file_at_ref,
            adaptor::controller::command::code::diff::get_branch_diff_summary,
            adaptor::controller::command::code::diff::build_diff_file_tree,
            adaptor::controller::command::code::diff::get_file_navigation,
            // Git: hunk/patch（code ドメイン）
            adaptor::controller::command::code::hunk::compute_diff_hunks,
            adaptor::controller::command::code::hunk::compute_hidden_ranges,
            adaptor::controller::command::code::hunk::compute_hidden_ranges_from_content,
            adaptor::controller::command::code::hunk::compute_visible_markdown_blocks,
            adaptor::controller::command::code::hunk::generate_group_patch,
            adaptor::controller::command::code::language::get_language_from_path,
            adaptor::controller::command::code::diff::get_relative_path,
            // Git: ブランチ（repository ドメイン）
            adaptor::controller::command::repository::branch::list_branches,
            adaptor::controller::command::repository::branch::get_current_branch,
            adaptor::controller::command::repository::branch::get_default_branch,
            adaptor::controller::command::repository::branch::git_create_branch,
            adaptor::controller::command::repository::branch::delete_branch,
            // Git: ステータス（repository ドメイン）
            adaptor::controller::command::repository::status::get_git_status,
            adaptor::controller::command::repository::status::get_status_diff_stats,
            adaptor::controller::command::repository::log::get_git_log,
            // Git: ステージング（code ドメイン）
            adaptor::controller::command::code::staging::git_stage,
            adaptor::controller::command::code::staging::git_unstage,
            adaptor::controller::command::code::staging::git_stage_hunk,
            adaptor::controller::command::code::staging::git_unstage_hunk,
            // Git: ワークツリー（repository ドメイン）
            adaptor::controller::command::repository::worktree::get_main_repo_path,
            adaptor::controller::command::repository::worktree::get_worktree_dirty_count,
            adaptor::controller::command::repository::worktree::list_worktrees,
            adaptor::controller::command::repository::worktree::list_branches_with_status,
            adaptor::controller::command::repository::worktree::create_worktree,
            adaptor::controller::command::repository::worktree::remove_worktree,
            // Git: 設定・ユーティリティ（repository ドメイン）
            adaptor::controller::command::repository::util::get_cwd,
            adaptor::controller::command::repository::util::get_repo_git_dir,
            adaptor::controller::command::repository::git_config::get_releash_base,
            adaptor::controller::command::repository::git_config::set_releash_base,
            adaptor::controller::command::repository::git_config::get_branch_base,
            adaptor::controller::command::repository::git_config::set_branch_base,
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
            config::get_workflow_config,
            config::update_workflow_config,
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
            agent_status_commands::get_session_status,
            agent_status_commands::get_workspace_status,
            agent_status_commands::list_workspace_statuses,
            agent_status_commands::list_session_statuses,
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
            // Repo paths（repository ドメイン）
            adaptor::controller::command::repository::repo_paths::get_repo_paths,
            adaptor::controller::command::repository::repo_paths::add_repo_path,
            adaptor::controller::command::repository::repo_paths::remove_repo_path,
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
            // Review comments
            review_comments::list_review_threads,
            review_comments::get_review_thread,
            review_comments::create_review_thread,
            review_comments::append_review_comment,
            review_comments::resolve_review_thread,
            review_comments::delete_review_thread,
            review_comments::get_review_thread_history,
            review_comments::build_review_thread_handoff,
            // OneShot PTY
            pty::oneshot::spawn_oneshot_pty,
            pty::oneshot::cancel_oneshot_pty,
            pty::oneshot::get_oneshot_pty_status,
            pty::oneshot::list_oneshot_ptys,
            pty::oneshot::find_oneshot_pty,
            // File Mention（code ドメイン）
            adaptor::controller::command::code::mention::list_mentionable_files,
            // Agent Backend Registry
            infrastructure::agent_session::runtime::list_agent_backends,
            // Agent SDK
            agent_commands::start_agent_session,
            infrastructure::agent_session::runtime::interrupt_agent_query,
            agent_commands::close_agent_session,
            infrastructure::agent_session::runtime::set_agent_permission_mode,
            infrastructure::agent_session::runtime::set_agent_model,
            infrastructure::agent_session::runtime::set_session_backend,
            infrastructure::agent_session::runtime::respond_agent_permission,
            agent_commands::send_agent_message,
            infrastructure::agent_session::runtime::init_agent_sessions,
            infrastructure::agent_session::runtime::scan_slash_commands,
            infrastructure::agent_session::runtime::prepare_image_attachment,
            infrastructure::agent_session::runtime::prepare_image_attachments_from_paths,
            // Session
            session_commands::list_sessions,
            infrastructure::agent_session::runtime::get_session,
            session_commands::create_session,
            session_commands::close_session,
            session_commands::restore_session,
            session_commands::list_closed_sessions,
            session_commands::add_message,
            session_commands::update_session_state,
            session_commands::update_session_agent_info,
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
            workflow::commands::send_workflow_approval_chat_message,
            workflow::commands::open_workflow_step_tab,
            workflow::commands::list_workflow_runs,
            workflow::commands::get_workflow_run,
            workflow::commands::get_workflow_run_log,
            workflow::commands::get_workflow_run_state,
            workflow::commands::get_workflow_step_detail,
            workflow::commands::resolve_active_run_by_worktree,
            workflow::commands::resolve_worktree_by_run,
            workflow::commands::list_facets,
            workflow::commands::get_facet,
            workflow::commands::save_facet,
            workflow::commands::delete_facet,
            workflow::commands::diagnose_all_cmd,
            workflow::commands::list_facet_summaries,
            workflow::commands::duplicate_workflow,
            workflow::commands::duplicate_facet,
            workflow::commands::open_facet_in_editor,
            workflow::commands::render_facet_preview,
            workflow::commands::get_automation_config_dir,
            workflow::commands::workflow_submit_output,
            workflow::commands::workflow_validate_output,
            workflow::commands::workflow_get_output,
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
