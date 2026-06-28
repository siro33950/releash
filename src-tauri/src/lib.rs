mod adaptor;
pub mod cli;
mod domain;
mod infrastructure;
mod other;
// Test-only helpers are intentionally kept as a root module.
#[cfg(test)]
mod test_support;
mod usecase;

use std::sync::Arc;
use std::time::Instant;

use adaptor::gateway::app_config::{load_or_create_config, AppConfig};
use domain::app_config::{
    AgentConfigRepository, ConfigRepository, ConfigSecretRepository, NotionConfigRepository,
};
use tauri::Manager;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_started = Instant::now();
    other::telemetry::set_startup_origin(startup_started);

    // OTLP exporter and async commands share the Tokio runtime installed for Tauri.
    let _runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let _runtime_guard = _runtime.enter();
    tauri::async_runtime::set(_runtime.handle().clone());

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let _ = fix_path_env::fix();

    let ws_broadcaster =
        Arc::new(adaptor::gateway::shared::ws_broadcaster::WsBroadcaster::default());
    let pty_gateway =
        Arc::new(adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway::default());
    let pty_read_gateway: Arc<
        dyn usecase::pty_session::ports::PtySessionReadGateway + Send + Sync,
    > = pty_gateway.clone();
    let pty_session_read_usecase =
        Arc::new(usecase::pty_session::read_usecase::PtySessionReadUsecase::new(pty_read_gateway));
    let pty_gateway_for_setup = Arc::clone(&pty_gateway);
    let pty_replay_reader: Arc<dyn usecase::pty_session::query_service::PtySessionReplayReader> =
        Arc::new(
            usecase::pty_session::query_service::PtySessionReplayQueryService::new(Arc::clone(
                &pty_gateway,
            )),
        );
    let session_storage = Arc::new(adaptor::gateway::agent_session::FileSessionStorage::default());
    let session_store = Arc::new(usecase::agent_session::session::SessionStore::new(
        session_storage.clone(),
    ));
    let review_comment_usecase =
        Arc::new(adaptor::controller::wiring::build_review_comment_usecase());
    let prompt_suggestion_usecase = Arc::new(
        adaptor::controller::wiring::build_agent_prompt_suggestion_usecase(session_storage),
    );
    let cleanup_gate = Arc::new(infrastructure::agent_session::runtime::CleanupGate::new(
        !cfg!(unix),
    ));
    #[cfg(unix)]
    let cleanup_gate_for_setup = Arc::clone(&cleanup_gate);

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(review_comment_usecase)
        .manage(session_store)
        .manage(prompt_suggestion_usecase)
        .manage(Arc::clone(&pty_gateway))
        .manage(pty_replay_reader)
        .manage(infrastructure::file_watcher::FileWatcherManager::default())
        .manage(Arc::clone(&ws_broadcaster))
        .manage(Arc::new(tokio::sync::Mutex::new(
            infrastructure::agent_session::runtime::AgentProcessMap::new(),
        )))
        .manage(Arc::new(
            usecase::agent_session::session::OpenTabRegistry::default(),
        ))
        .manage(infrastructure::middleware::WsServerHandle::default())
        .manage(cleanup_gate)
        .manage::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>(Arc::new(
            parking_lot::RwLock::new(Vec::new()),
        ))
        .setup(move |app| {
            pty_gateway_for_setup.start_idle_sweeper(app.handle().clone());
            let data_dir = app.path().app_data_dir()?;
            app.manage(Arc::new(
                adaptor::gateway::workspace_state::WorkspaceStateStore::new(data_dir.clone()),
            ));
            // spec issues-1054 Implementation Freedom (L104): 別 Releash binary 由来の
            // RELEASH_DATA_DIR inherit (例: prod 版 Releash の Terminal Panel から起動
            // した shell から dev binary を起動した場合) を「ユーザー明示指定」と誤認しないよう、
            // 起動初期に env を自プロセス alias data_dir で正す。
            crate::infrastructure::platform::path_aliases::ensure_release_data_dir_env_for_app(
                app.handle(),
            );
            infrastructure::platform::cli_install::ensure_cli_symlink_installed();
            let config_path = data_dir.join("releash.toml");
            let config = load_or_create_config(&config_path)
                .map_err(|e| format!("設定ファイルの読み込みに失敗: {e}"))?;
            if let Some(telemetry_guard) = infrastructure::telemetry::init_telemetry(&config) {
                app.manage(telemetry_guard);
            }

            {
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                let handles_state =
                    app.state::<Arc<
                        tokio::sync::Mutex<infrastructure::agent_session::runtime::AgentProcessMap>,
                    >>()
                    .inner()
                    .clone();
                let stream_resync_read_model: Arc<
                    dyn usecase::agent_session::session::AgentStreamResyncReadModel,
                > = Arc::new(
                    infrastructure::agent_session::runtime_gateway::AgentStreamResyncRuntimeReadModel::new(
                        session_store_state.clone(),
                        handles_state.clone(),
                        data_dir.clone(),
                    ),
                );
                app.manage(stream_resync_read_model);
                app.manage(Arc::new(
                    adaptor::controller::wiring::build_stored_session_lifecycle_usecase(
                        app.handle().clone(),
                        session_store_state,
                        handles_state,
                    ),
                ));
            }

            let app_config = Arc::new(AppConfig::new(config, config_path));
            let config_repository: Arc<dyn ConfigRepository> = app_config.clone();
            let agent_config_repository: Arc<dyn AgentConfigRepository> = app_config.clone();
            let config_secret_repository: Arc<dyn ConfigSecretRepository> = app_config.clone();
            let notion_config_repository: Arc<dyn NotionConfigRepository> = app_config.clone();
            let notion_api_gateway: Arc<dyn domain::notion::NotionApiGateway> =
                Arc::new(adaptor::gateway::notion::NotionApiGatewayImpl::new());
            app.manage(config_repository.clone());
            app.manage(agent_config_repository.clone());
            app.manage(config_secret_repository.clone());

            // Initialize shared repo_paths from config
            let shared_repo_paths = app
                .state::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>()
                .inner()
                .clone();
            {
                if let Ok(cfg) = config_repository.load() {
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
                    RepoPathsGateway::new(shared_repo_paths.clone(), config_repository.clone());
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
                let code_usecase = Arc::new(
                    adaptor::controller::wiring::build_code_usecase_with_app(app.handle().clone()),
                );
                let base_branch_resolver: Arc<
                    dyn infrastructure::agent_session::resolver_ports::BaseBranchResolverPort,
                > = code_usecase.clone();
                let mention_resolver: Arc<
                    dyn infrastructure::agent_session::resolver_ports::MentionResolverPort,
                > = code_usecase.clone();
                let branch_diff_context: Arc<
                    dyn usecase::agent_session::context::BranchDiffContextPort,
                > = code_usecase.clone();
                app.manage(base_branch_resolver);
                app.manage(mention_resolver);
                app.manage(branch_diff_context);
                let agent_session_usecase = Arc::new(
                    adaptor::controller::wiring::build_agent_session_usecase(app.handle().clone()),
                );
                let git_host_usecase =
                    Arc::new(adaptor::controller::wiring::build_git_host_usecase());
                let repository_scanner = Arc::new(
                    adaptor::gateway::repository::scanner::DefaultRepositoryScanner::new(
                        repository_usecase.clone(),
                        code_usecase.clone(),
                    ),
                );
                let repository_state_repository = Arc::new(
                    adaptor::gateway::repository::state::RepositoryStateRepositoryGateway::new(
                        repository_usecase.clone(),
                    ),
                );
                let repository_state =
                    Arc::new(usecase::repository_state::RepositoryStateService::new(
                        repository_state_repository,
                        repository_scanner,
                        Arc::new(
                            adaptor::gateway::repository::state::TauriRepositoryStateNotifier::new(
                                app.handle().clone(),
                                ws_broadcaster.clone(),
                            ),
                        ),
                        Arc::new(
                            adaptor::gateway::repository::state::NotifyRepositoryStateWatcher::new(
                                repository_usecase.clone(),
                            ),
                        ),
                        Arc::new(
                            adaptor::gateway::repository::state::TokioRepositoryStateWorkerRuntime,
                        ),
                        Arc::new(adaptor::gateway::repository::state::FsWorktreePathNormalizer),
                    ));
                app.manage(repository_state.clone());
                let review_usecase = Arc::new(usecase::review_usecase::ReviewUsecase::new(
                    repository_state.clone(),
                    code_usecase.clone(),
                ));
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                let workflow_usecase = Arc::new(
                    adaptor::controller::wiring::build_workflow_usecase_with_repository_worktrees(
                        data_dir.clone(),
                        repository_usecase.clone(),
                        config_repository.clone(),
                        config_secret_repository.clone(),
                        session_store_state,
                        app.handle().clone(),
                    ),
                );
                let notion_usecase = Arc::new(usecase::notion::usecase::NotionUsecase::new(
                    notion_config_repository.clone(),
                    notion_api_gateway.clone(),
                ));

                app.manage(AppState {
                    repository_usecase: repository_usecase.clone(),
                    repository_state,
                    repo_paths_usecase,
                    code_usecase,
                    review_usecase,
                    agent_session_usecase,
                    notion_usecase,
                    workflow_usecase,
                    pty_session_read_usecase,
                    git_host_usecase,
                });
            }

            let focus_tracker = Arc::new(parking_lot::Mutex::new(
                infrastructure::platform::focus_tracker::FocusTracker::new(),
            ));
            app.manage(focus_tracker.clone());
            infrastructure::platform::focus_tracker::install(app, focus_tracker.clone());

            {
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                let notification_usecase = Arc::new(
                    usecase::notification::usecase::AgentSessionNotificationUsecase::new(
                        Arc::new(
                            adaptor::gateway::notification::NotificationSettingsConfigGateway::new(
                                config_repository.clone(),
                            ),
                        ),
                        Arc::new(
                            adaptor::gateway::notification::FocusNotificationInactivityGateway::new(
                                focus_tracker.clone(),
                            ),
                        ),
                        Arc::new(adaptor::gateway::notification::ReqwestWebhookSenderGateway),
                    ),
                );
                adaptor::controller::notification_wiring::register_agent_notification_listener(
                    session_store_state,
                    notification_usecase,
                );
            }

            // AgentStatusCenter を構築・登録
            let agent_status_center =
                Arc::new(usecase::agent_session::status::AgentStatusCenter::new());
            let agent_status_notifier: Arc<dyn usecase::agent_session::status::AgentStatusNotifier> =
                Arc::new(adaptor::presenter::agent_status::TauriAgentStatusNotifier::new(
                    app.handle().clone(),
                    app.state::<Arc<adaptor::gateway::shared::ws_broadcaster::WsBroadcaster>>()
                        .inner()
                        .clone(),
                ));
            // SessionStore の状態変更通知を購読して、保持している SessionStatus を
            // 最新化＋再集約する。Closed への遷移は aggregate でフィルタされ、
            // Closed → Idle の復帰では再び集約対象に戻る。
            {
                let session_store_state = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                adaptor::controller::agent_status_wiring::register_agent_status_listener(
                    session_store_state,
                    agent_status_center.clone(),
                    agent_status_notifier.clone(),
                );
            }
            app.manage(agent_status_notifier);
            app.manage(agent_status_center);
            let pending_data_dir = app.path().app_data_dir().ok();
            // AgentBackendRegistry を構築・登録
            let agent_handles = app
                .state::<Arc<Mutex<infrastructure::agent_session::runtime::AgentProcessMap>>>()
                .inner()
                .clone();
            let session_store = app
                .state::<Arc<usecase::agent_session::session::SessionStore>>()
                .inner()
                .clone();
            let open_tabs = app
                .state::<Arc<usecase::agent_session::session::OpenTabRegistry>>()
                .inner()
                .clone();
            let branch_diff_context = app
                .state::<Arc<dyn usecase::agent_session::context::BranchDiffContextPort>>()
                .inner()
                .clone();
            let workflow_step_lifecycle_usecase = Arc::new(
                adaptor::controller::wiring::build_workflow_step_lifecycle_usecase(
                    app.handle().clone(),
                    session_store.clone(),
                    agent_handles.clone(),
                    open_tabs,
                ),
            );
            app.manage(workflow_step_lifecycle_usecase);
            let workflow_runtime_usecase =
                Arc::new(adaptor::controller::wiring::build_workflow_runtime_usecase(
                    app.handle().clone(),
                    repository_usecase.clone(),
                    config_repository.clone(),
                    session_store.clone(),
                    agent_handles.clone(),
                    branch_diff_context.clone(),
                    pending_data_dir.clone(),
                ));
            app.manage(workflow_runtime_usecase);
            let registry = Arc::new(
                infrastructure::agent_session::runtime::build_registry_with_runtime(
                    agent_config_repository.clone(),
                    app.handle().clone(),
                    agent_handles,
                    session_store,
                    branch_diff_context,
                ),
            );
            app.manage(registry.clone());

            // [06] CLI mutating CLI 経路の file watcher を起動する。初回 pickup は
            // setup 済みの WorkflowRuntimeUsecase / AgentBackendRegistry を前提に dispatch
            // するため、workflow 依存 state の登録完了後にだけ spawn する。
            //
            // data_dir が解決できなければ watcher は spawn せず、稼働中アプリでも CLI
            // pending command は pickup されない（spec [06] CLI 起動独立性境界:
            // それでも CLI 側の書き込み完了境界は保たれる）。
            if let Some(data_dir) = pending_data_dir {
                adaptor::controller::wiring::spawn_workflow_pending_command_watcher(
                    app.handle().clone(),
                    data_dir.clone(),
                );

                // [issues-1022] CLI からの Thread / Comment 書き込みを UI へ
                // 反映するため、`<data_dir>/review-comments/` を file watch して
                // `review-comments-changed` を発火する。Tauri コマンド経由の
                // `emit_changed` とは別系統の通知経路で、CLI / Agent / 外部編集
                // 由来の書き込みも拾う。
                infrastructure::comment::watcher::spawn_review_comments_watcher(
                    app.handle().clone(),
                    data_dir,
                );
            }

            infrastructure::platform::menu::setup_menu(app)?;
            infrastructure::platform::tray::setup_tray(
                app,
                adaptor::controller::application_lifecycle::request_application_quit,
            )?;
            if let Some(window) = app.get_webview_window("main") {
                infrastructure::platform::native_drop::install(&window);
            }

            infrastructure::platform::window_lifecycle::apply_startup_visibility(
                app.handle(),
                config_repository.as_ref(),
            );

            // Clean up orphan agent processes from previous crashes in the background.
            // Agent process spawn paths wait on cleanup_gate just before OS spawn.
            #[cfg(unix)]
            {
                infrastructure::agent_session::startup::spawn_startup_orphan_cleanup(
                    data_dir.clone(),
                    Arc::clone(&cleanup_gate_for_setup),
                    infrastructure::agent_session::runtime::cleanup_orphan_processes,
                );
            }

            other::telemetry::record_startup_from_origin(other::telemetry::Startup::AppStartup);

            Ok(())
        });
    let builder =
        adaptor::controller::command::code::review_blob::register_review_blob_protocol(builder);

    let builder = adaptor::controller::command::register_all(builder);
    builder
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            infrastructure::platform::window_lifecycle::handle_run_event(app_handle, event);
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokio_runtime_context_is_available_after_setup() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        tauri::async_runtime::set(runtime.handle().clone());

        // Tauri-side async work needs a thread-local runtime context.
        let handle = tokio::spawn(async { 42 });
        let result = runtime.block_on(handle).unwrap();
        assert_eq!(result, 42);
    }
}
