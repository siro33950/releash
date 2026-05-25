mod agent_commands;
mod agent_message_dispatcher;
mod agent_sdk;
mod agent_status;
mod backends;
pub mod cli;
mod cli_install;
mod config;
mod diff_comment_sender;
mod diff_comment_store;
mod domain;
mod external_editor;
mod file_mention;
mod focus_tracker;
mod git;
mod git_host;
mod mcp;
mod menu;
mod native_drop;
mod notion;
mod permission;
mod protocol;
mod pty;
mod qr_code;
mod repo_registry;
mod sentry_integration;
mod session;
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

/// 起動時の初期モデル整合性チェック。
/// `agents.<backend>.model` が `agents.<backend>.models` に含まれない場合は
/// 警告ログのみ出し、書き換えは行わない（暗黙のフォールバックを避ける）。
fn warn_on_unregistered_initial_models(
    registry: &backends::AgentBackendRegistry,
    backend_ids: &[&str],
) {
    for backend_id in backend_ids {
        if let Some(message) = initial_model_resolution_warning(
            backend_id,
            &registry.initial_model_resolution_for(backend_id),
        ) {
            log::warn!("{message}");
        }
    }
}

fn initial_model_resolution_warning(
    backend_id: &str,
    resolution: &backends::InitialModelResolution,
) -> Option<String> {
    match resolution {
        backends::InitialModelResolution::Invalid { model, reason } => Some(format!(
            "backend '{backend_id}' initial model {} is invalid ({reason}); treating as unset until refreshed",
            domain::agent_session::escaped_for_log(model)
        )),
        backends::InitialModelResolution::Unregistered { model } => Some(format!(
            "backend '{backend_id}' initial model {} is not in agents.{backend_id}.models; treating as unset until refreshed",
            domain::agent_session::escaped_for_log(model)
        )),
        backends::InitialModelResolution::Registered(_) | backends::InitialModelResolution::Unset => {
            None
        }
    }
}

/// 起動時のバックエンド単位のモデル一覧同期 spawn。
/// バックエンドごとに独立した `tokio::spawn` を発行し、片方の失敗・遅延は
/// もう片方の spawn に影響しない。アプリ起動は spawn 後即時続行する。
fn spawn_startup_model_refresh_for_backends(
    app: tauri::AppHandle,
    handles: Arc<Mutex<agent_sdk::AgentProcessMap>>,
    registry: Arc<backends::AgentBackendRegistry>,
    backend_ids: &[&str],
) {
    spawn_startup_model_refresh_with(backend_ids, |backend_id| {
        let registry_clone = registry.clone();
        let handles_clone = handles.clone();
        let app_clone = app.clone();
        async move {
            backends::model_catalog_sync::refresh_models_for_backend_and_propagate(
                app_clone,
                handles_clone,
                registry_clone,
                backend_id,
            )
            .await;
        }
    });
}

/// 内部ヘルパー: backend_id ごとに独立 task を spawn する純粋ループ。
/// 各 backend の `task_factory` 呼び出し結果は独立した `tokio::spawn` に渡され、
/// 片方のタスクが完了・遅延・panic しても他方は影響を受けない。
///
/// テストでは `task_factory` に副作用カウンタを返すクロージャを渡して、
/// 「複数 backend が独立に spawn されること」「ある backend のクロージャが
/// panic しても他の backend は影響を受けないこと」を検証する。
fn spawn_startup_model_refresh_with<F, Fut>(backend_ids: &[&str], mut task_factory: F)
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    for backend_id in backend_ids {
        let fut = task_factory(backend_id.to_string());
        tokio::spawn(fut);
    }
}

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
    let backend_models_notifier =
        ws_server::gateway::WsBackendModelsUpdateNotifier::new(Arc::clone(&ws_broadcaster));

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
        .manage(Arc::clone(&ws_broadcaster))
        .manage::<usecase::backend_models::BackendModelsUpdateNotifierState>(Arc::new(
            backend_models_notifier,
        ))
        .manage(Arc::new(tokio::sync::Mutex::new(
            agent_sdk::AgentProcessMap::new(),
        )))
        .manage(Arc::new(session::OpenTabRegistry::default()))
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
            // SessionStore の状態変更通知を購読して、保持している SessionStatus を
            // 最新化＋再集約する。Closed への遷移は aggregate でフィルタされ、
            // Closed → Idle の復帰では再び集約対象に戻る。
            {
                let center_for_listener = agent_status_center.clone();
                let session_store_state = app.state::<Arc<session::SessionStore>>().inner().clone();
                session_store_state.register_state_change_listener(Arc::new(
                    move |session_id, _worktree_path, new_state| {
                        center_for_listener.on_session_state_changed(session_id, new_state.clone());
                    },
                ));
            }
            app.manage(agent_status_center);
            let workflow_engine = Arc::new(workflow::engine::WorkflowEngine::new(
                Arc::new(workflow::resolver_adapters::DefaultWorkflowDefinitionResolver),
                Arc::new(
                    workflow::resolver_adapters::AppConfigManagedWorktreeResolver::new(
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
                .state::<Arc<Mutex<agent_sdk::AgentProcessMap>>>()
                .inner()
                .clone();
            let session_store = app.state::<Arc<session::SessionStore>>().inner().clone();
            let registry = Arc::new(backends::build_registry_with_runtime(
                app_config.clone(),
                app.handle().clone(),
                agent_handles,
                session_store,
            ));
            app.manage(registry.clone());

            // 起動時の初期モデル整合性チェック（agents.<backend>.model が登録一覧に
            // 含まれない場合は警告のみ。書き換えは行わない）
            warn_on_unregistered_initial_models(
                &registry,
                &[
                    backends::bridge_common::CLAUDE_BACKEND_ID,
                    backends::bridge_common::CODEX_BACKEND_ID,
                ],
            );

            // 起動時バックグラウンドモデル一覧同期（バックエンド単位で独立、起動をブロックしない）
            let handles_for_refresh = app
                .state::<Arc<Mutex<agent_sdk::AgentProcessMap>>>()
                .inner()
                .clone();
            spawn_startup_model_refresh_for_backends(
                app.handle().clone(),
                handles_for_refresh,
                registry.clone(),
                &[
                    backends::bridge_common::CLAUDE_BACKEND_ID,
                    backends::bridge_common::CODEX_BACKEND_ID,
                ],
            );

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
                    data_dir,
                );
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
            // Agent Backend Registry
            backends::list_agent_backends,
            // Agent SDK
            agent_commands::start_agent_session,
            agent_sdk::interrupt_agent_query,
            agent_commands::close_agent_session,
            agent_sdk::set_agent_permission_mode,
            agent_sdk::set_agent_model,
            agent_sdk::set_session_backend,
            agent_sdk::respond_agent_permission,
            agent_commands::send_agent_message,
            agent_sdk::init_agent_sessions,
            agent_sdk::scan_slash_commands,
            agent_sdk::prepare_image_attachment,
            agent_sdk::prepare_image_attachments_from_paths,
            // Session
            session::list_sessions,
            agent_sdk::get_session,
            session::create_session,
            session_commands::close_session,
            session_commands::restore_session,
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
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc as StdArc;
    use std::sync::Mutex as StdMutex;

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

    #[tokio::test]
    async fn spawn_startup_model_refresh_schedules_independent_task_per_backend() {
        let scheduled: StdArc<StdMutex<Vec<String>>> = StdArc::new(StdMutex::new(Vec::new()));
        let completed = StdArc::new(AtomicUsize::new(0));

        let scheduled_clone = scheduled.clone();
        let completed_clone = completed.clone();
        spawn_startup_model_refresh_with(&["claude", "codex"], move |backend_id| {
            scheduled_clone.lock().unwrap().push(backend_id.clone());
            let completed_inner = completed_clone.clone();
            async move {
                completed_inner.fetch_add(1, AtomicOrdering::SeqCst);
                let _ = backend_id; // 各 task は独立して動く
            }
        });

        // factory は backend_id ごとに呼ばれ、戻り値の future が個別 task として spawn される。
        let scheduled_now = scheduled.lock().unwrap().clone();
        assert_eq!(
            scheduled_now,
            vec!["claude".to_string(), "codex".to_string()]
        );

        // 各 task が独立して完走する（短い処理なのですぐに完了する）。
        for _ in 0..50 {
            if completed.load(AtomicOrdering::SeqCst) == 2 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "spawned tasks did not complete in time: completed={}",
            completed.load(AtomicOrdering::SeqCst)
        );
    }

    #[tokio::test]
    async fn spawn_startup_model_refresh_isolates_one_failure_from_others() {
        // 片方の backend は task 内で panic、もう片方は成功する。
        let other_completed = StdArc::new(AtomicUsize::new(0));
        let other_completed_clone = other_completed.clone();

        spawn_startup_model_refresh_with(&["claude", "codex"], move |backend_id| {
            let counter = other_completed_clone.clone();
            async move {
                if backend_id == "claude" {
                    panic!("simulated refresh failure for claude");
                }
                counter.fetch_add(1, AtomicOrdering::SeqCst);
            }
        });

        // claude 側の panic は codex 側の spawn・完了を妨げない。
        for _ in 0..50 {
            if other_completed.load(AtomicOrdering::SeqCst) == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("non-failing backend task did not complete despite peer failure");
    }

    #[test]
    fn warn_on_unregistered_initial_models_does_not_mutate_config() {
        use crate::config::ReleashConfig;
        let mut cfg = ReleashConfig::default();
        cfg.agents.claude.model = Some("missing-from-registry".to_string());
        cfg.agents.claude.models = vec!["sonnet".to_string()];
        cfg.agents.codex.model = Some("o3".to_string());
        cfg.agents.codex.models = vec!["o3".to_string()];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let app_config = StdArc::new(AppConfig::new(cfg, tmp.path().to_path_buf()));
        let registry = backends::build_registry(app_config.clone());

        warn_on_unregistered_initial_models(&registry, &["claude", "codex"]);

        // 書き換えは行わない（暗黙のフォールバック禁止）
        let after = app_config.get_config().unwrap();
        assert_eq!(
            after.agents.claude.model.as_deref(),
            Some("missing-from-registry")
        );
        assert_eq!(after.agents.codex.model.as_deref(), Some("o3"));
    }

    #[test]
    fn initial_model_warning_messages_are_traceable() {
        assert_eq!(
            initial_model_resolution_warning(
                "claude",
                &backends::InitialModelResolution::Unregistered {
                    model: "missing-from-registry".to_string(),
                },
            ),
            Some(
                "backend 'claude' initial model \"missing-from-registry\" is not in agents.claude.models; treating as unset until refreshed"
                    .to_string()
            )
        );
        assert_eq!(
            initial_model_resolution_warning(
                "codex",
                &backends::InitialModelResolution::Invalid {
                    model: "bad model".to_string(),
                    reason: "contains whitespace".to_string(),
                },
            ),
            Some(
                "backend 'codex' initial model \"bad model\" is invalid (contains whitespace); treating as unset until refreshed"
                    .to_string()
            )
        );
        assert_eq!(
            initial_model_resolution_warning(
                "codex",
                &backends::InitialModelResolution::Registered("o3".to_string()),
            ),
            None
        );
    }
}
