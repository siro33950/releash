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

    let pty_gateway =
        Arc::new(adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway::default());
    let pty_read_gateway: Arc<
        dyn usecase::pty_session::ports::PtySessionReadGateway + Send + Sync,
    > = pty_gateway.clone();
    let pty_session_read_usecase =
        Arc::new(usecase::pty_session::read_usecase::PtySessionReadUsecase::new(pty_read_gateway));
    let pty_gateway_for_setup = Arc::clone(&pty_gateway);
    let session_storage = Arc::new(adaptor::gateway::agent_session::FileSessionStorage::default());
    let session_store = Arc::new(usecase::agent_session::session::SessionStore::new(
        session_storage.clone(),
    ));
    let workspace_session_creation_usecase = Arc::new(
        usecase::agent_session::workspace_session_creation::WorkspaceSessionCreationUsecase::new(
            session_store.clone(),
        ),
    );
    let review_comment_usecase =
        Arc::new(adaptor::controller::wiring::build_review_comment_usecase());
    let prompt_suggestion_usecase = Arc::new(
        adaptor::controller::wiring::build_agent_prompt_suggestion_usecase(session_storage),
    );
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
        .manage(workspace_session_creation_usecase)
        .manage(prompt_suggestion_usecase)
        .manage(Arc::clone(&pty_gateway))
        .manage(infrastructure::file_watcher::FileWatcherManager::default())
        .manage(Arc::new(
            usecase::agent_session::session::OpenTabRegistry::default(),
        ))
        .manage::<adaptor::gateway::repository::repo_paths::SharedRepoPaths>(Arc::new(
            parking_lot::RwLock::new(Vec::new()),
        ))
        .setup(move |app| {
            pty_gateway_for_setup.start_idle_sweeper(app.handle().clone());
            let data_dir = app.path().app_data_dir()?;
            let cleanup_report =
                infrastructure::process::pid_registry::cleanup_orphan_processes(&data_dir);
            if cleanup_report.scanned > 0 || cleanup_report.failures > 0 {
                log::info!(
                    "agent orphan cleanup scanned={} processed={} skipped={} failures={}",
                    cleanup_report.scanned,
                    cleanup_report.processed,
                    cleanup_report.skipped,
                    cleanup_report.failures
                );
            }
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
            app.manage(Arc::new(
                adaptor::controller::wiring::build_agent_backend_registry(
                    agent_config_repository.clone(),
                ),
            ));

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
            adaptor::controller::wiring::spawn_startup_app_data_gc(
                data_dir.clone(),
                shared_repo_paths.clone(),
            );

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
                let branch_diff_context: Arc<
                    dyn usecase::agent_session::context::BranchDiffContextPort,
                > = Arc::new(
                    adaptor::gateway::code::branch_diff_context::CodeBranchDiffContextGateway::new(
                        code_usecase.clone(),
                    ),
                );
                app.manage(branch_diff_context);
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
            let agent_status_notifier: Arc<
                dyn usecase::agent_session::status::AgentStatusNotifier,
            > = Arc::new(
                adaptor::presenter::agent_status::TauriAgentStatusNotifier::new(
                    app.handle().clone(),
                ),
            );
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
            app.manage(agent_status_notifier.clone());
            app.manage(agent_status_center.clone());
            {
                let runtime_session_store = app
                    .state::<Arc<usecase::agent_session::session::SessionStore>>()
                    .inner()
                    .clone();
                let runtime_registry = app
                    .state::<Arc<usecase::agent_session::backend_registry::AgentBackendRegistry>>()
                    .inner()
                    .clone();
                let runtime_notifier: Arc<
                    dyn usecase::agent_session::runtime::ports::AgentSessionEventNotifier,
                > = Arc::new(
                    adaptor::presenter::agent_session::TauriAgentSessionEventNotifier::new(
                        app.handle().clone(),
                    ),
                );
                let runtime_spawner: Arc<
                    dyn usecase::agent_session::runtime::ports::AgentTaskSpawner,
                > = Arc::new(adaptor::gateway::agent_session::TokioAgentTaskSpawner);
                let runtime_branch_diff_context = app
                    .state::<Arc<dyn usecase::agent_session::context::BranchDiffContextPort>>()
                    .inner()
                    .clone();
                let runtime_instruction_source: Arc<
                    dyn usecase::agent_session::context::InstructionSourcePort,
                > = Arc::new(adaptor::gateway::agent_session::FileSystemInstructionSourceGateway);
                let runtime_data_dir = app
                    .path()
                    .app_data_dir()
                    .expect("failed to resolve app data directory");
                app.manage(Arc::new(
                    usecase::agent_session::runtime::AgentSessionRuntimeUsecase::new(
                        runtime_session_store.clone(),
                        runtime_registry,
                        agent_status_center.clone(),
                        agent_status_notifier.clone(),
                        runtime_notifier,
                        runtime_spawner,
                        Some(runtime_branch_diff_context),
                        runtime_instruction_source,
                        runtime_data_dir,
                    ),
                ));
                let stored_lifecycle_registry = app
                    .state::<Arc<usecase::agent_session::backend_registry::AgentBackendRegistry>>()
                    .inner()
                    .clone();
                let stored_lifecycle_runtime = app
                    .state::<Arc<usecase::agent_session::runtime::AgentSessionRuntimeUsecase>>()
                    .inner()
                    .clone();
                let stored_session_lifecycle = Arc::new(
                    adaptor::controller::wiring::build_stored_session_lifecycle_usecase(
                        runtime_session_store,
                        stored_lifecycle_registry,
                        stored_lifecycle_runtime,
                    ),
                );
                app.manage(stored_session_lifecycle.clone());
                let workspace_node_resolver: Arc<
                    dyn usecase::workflow::WorkspaceNodeActionResolver,
                > = app
                    .state::<adaptor::controller::state::AppState>()
                    .workflow_usecase
                    .clone();
                app.manage(Arc::new(
                    adaptor::controller::wiring::build_workspace_node_command_usecase(
                        workspace_node_resolver,
                        stored_session_lifecycle,
                        data_dir.clone(),
                    ),
                ));
            }
            let agent_runtime = app
                .state::<Arc<usecase::agent_session::runtime::AgentSessionRuntimeUsecase>>()
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
            let workflow_node_lifecycle_usecase = Arc::new(
                adaptor::controller::wiring::build_node_execution_lifecycle_usecase(
                    app.handle().clone(),
                    session_store.clone(),
                    agent_runtime.clone(),
                    open_tabs.clone(),
                ),
            );
            app.manage(workflow_node_lifecycle_usecase);
            let workflow_runtime_usecase =
                Arc::new(adaptor::controller::wiring::build_workflow_runtime_usecase(
                    app.handle().clone(),
                    adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGatewayDeps {
                        repository_usecase: repository_usecase.clone(),
                        app_config: config_repository.clone(),
                        session_store: session_store.clone(),
                        agent_runtime: agent_runtime.clone(),
                        open_tabs,
                        branch_diff_context: branch_diff_context.clone(),
                        data_dir: Some(data_dir.clone()),
                    },
                ));
            let workflow_runtime_agent_notifier = Arc::new(
                adaptor::gateway::agent_session::WorkflowRuntimeAgentSessionNotifier::new(
                    workflow_runtime_usecase.clone(),
                ),
            );
            agent_runtime
                .set_workflow_turn_complete_notifier(workflow_runtime_agent_notifier.clone());
            agent_runtime.set_workflow_stall_notifier(workflow_runtime_agent_notifier);
            app.manage(workflow_runtime_usecase.clone());

            let workflow_query_usecase = app
                .state::<adaptor::controller::state::AppState>()
                .workflow_usecase
                .clone();
            let local_api_binding =
                infrastructure::local_api::LocalApiServerBinding::bind(data_dir.clone())
                    .map_err(|error| format!("local API の起動に失敗しました: {error}"))?;
            let local_api_router = adaptor::controller::api::build_router(
                Arc::new(workflow_query_usecase.read_usecase()),
                workflow_runtime_usecase.clone(),
                local_api_binding.bearer_token(),
            );
            let local_api =
                local_api_binding.start(local_api_router, &tokio::runtime::Handle::current());
            app.manage(local_api.clone());

            // CLI / Agent / 外部編集由来の review comment 変更を UI へ通知する。
            infrastructure::comment::watcher::spawn_review_comments_watcher(
                app.handle().clone(),
                data_dir.clone(),
            );

            infrastructure::platform::menu::setup_menu(app)?;
            let tray_agent_runtime = agent_runtime.clone();
            let tray_workflow_runtime = workflow_runtime_usecase.clone();
            let tray_local_api = local_api.clone();
            infrastructure::platform::tray::setup_tray(app, move |app| {
                adaptor::controller::application_lifecycle::request_application_quit_with_runtime(
                    app,
                    tray_agent_runtime.clone(),
                    tray_workflow_runtime.clone(),
                    {
                        let local_api = tray_local_api.clone();
                        move || local_api.shutdown()
                    },
                );
            })?;
            if let Some(window) = app.get_webview_window("main") {
                infrastructure::platform::native_drop::install(&window);
            }

            infrastructure::platform::window_lifecycle::apply_startup_visibility(
                app.handle(),
                config_repository.as_ref(),
            );

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
