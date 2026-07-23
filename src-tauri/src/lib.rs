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

type LocalApiShutdownTarget = Arc<parking_lot::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupMaintenanceAdmission {
    AwaitCutover,
    Admitted,
    Abandoned,
}

fn startup_maintenance_admission(
    normal_admission_ready: bool,
    migration_blocked: bool,
) -> StartupMaintenanceAdmission {
    match (normal_admission_ready, migration_blocked) {
        (true, false) => StartupMaintenanceAdmission::Admitted,
        (false, false) => StartupMaintenanceAdmission::AwaitCutover,
        (_, true) => StartupMaintenanceAdmission::Abandoned,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupRecoveryWorkerExit {
    Quiescent,
    AdmissionAbandoned,
}

/// Drives bounded recovery passes only under normal admission. Every pass
/// starts with a fresh pending-index snapshot; two empty passes define
/// quiescence, so work inserted while the first empty page was being observed
/// is not lost. Transient failures retain the worker with capped backoff.
async fn run_startup_recovery_after_normal_admission<Admission, Recover, Future, Error>(
    worker_name: &'static str,
    mut admission: Admission,
    mut recover_pass: Recover,
    admission_poll_delay: std::time::Duration,
    initial_retry_delay: std::time::Duration,
    maximum_retry_delay: std::time::Duration,
) -> StartupRecoveryWorkerExit
where
    Admission: FnMut() -> StartupMaintenanceAdmission,
    Recover: FnMut() -> Future,
    Future: std::future::Future<Output = Result<usize, Error>>,
    Error: std::fmt::Debug,
{
    let mut retry_delay = initial_retry_delay;
    let mut consecutive_empty_passes = 0u8;
    loop {
        match admission() {
            StartupMaintenanceAdmission::AwaitCutover => {
                tokio::time::sleep(admission_poll_delay).await;
            }
            StartupMaintenanceAdmission::Abandoned => {
                return StartupRecoveryWorkerExit::AdmissionAbandoned;
            }
            StartupMaintenanceAdmission::Admitted => match recover_pass().await {
                Ok(0) => {
                    consecutive_empty_passes = consecutive_empty_passes.saturating_add(1);
                    if consecutive_empty_passes >= 2 {
                        return StartupRecoveryWorkerExit::Quiescent;
                    }
                    tokio::task::yield_now().await;
                }
                Ok(_) => {
                    consecutive_empty_passes = 0;
                    retry_delay = initial_retry_delay;
                    // A claimed/reconciliation row can remain indexed until
                    // explicit readback. Keep supervising it, but do not spin
                    // a hot loop while no new durable progress is visible.
                    tokio::time::sleep(initial_retry_delay).await;
                }
                Err(error) => {
                    consecutive_empty_passes = 0;
                    log::warn!("{worker_name} startup recovery will retry: {error:?}");
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = retry_delay.saturating_mul(2).min(maximum_retry_delay);
                }
            },
        }
    }
}

fn spawn_startup_maintenance_after_normal_admission(
    data_dir: std::path::PathBuf,
    shared_repo_paths: adaptor::gateway::repository::repo_paths::SharedRepoPaths,
    local_event_store: Arc<adaptor::gateway::local_event_store::LocalEventStore>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            match startup_maintenance_admission(
                local_event_store.normal_admission_ready(),
                local_event_store.migration_blocked(),
            ) {
                StartupMaintenanceAdmission::AwaitCutover => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                StartupMaintenanceAdmission::Abandoned => break,
                StartupMaintenanceAdmission::Admitted => {
                    let cleanup_data_dir = data_dir.clone();
                    match tauri::async_runtime::spawn_blocking(move || {
                        infrastructure::process::pid_registry::cleanup_orphan_processes(
                            &cleanup_data_dir,
                        )
                    })
                    .await
                    {
                        Ok(report) if report.scanned > 0 || report.failures > 0 => {
                            log::info!(
                                "agent orphan cleanup scanned={} processed={} skipped={} failures={}",
                                report.scanned,
                                report.processed,
                                report.skipped,
                                report.failures
                            );
                        }
                        Ok(_) => {}
                        Err(error) => log::error!("agent orphan cleanup task failed: {error}"),
                    }
                    adaptor::controller::wiring::spawn_startup_app_data_gc(
                        data_dir,
                        shared_repo_paths,
                    );
                    break;
                }
            }
        }
    });
}

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
            let local_event_store =
                adaptor::gateway::local_event_store::LocalEventStore::open(
                    adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                        data_dir.clone(),
                    ),
                )
                .map_err(|error| format!("failed to open permanent local event store: {error}"))?;
            app.manage(local_event_store.clone());
            let session_store = app
                .state::<Arc<usecase::agent_session::session::SessionStore>>()
                .inner()
                .clone();
            let session_event_repository: Arc<
                dyn domain::local_event::LocalEventTransactionRepository,
            > = local_event_store.clone();
            let canonical_generation = local_event_store.generation_id().to_string();
            // Close every legacy session mutation/repair path immediately
            // after SQLite opens. Until verified cutover, reads may use the
            // immutable legacy source but commands fail migration-closed.
            session_store.set_local_event_repository(
                session_event_repository.clone(),
                canonical_generation.clone(),
            );
            let install_canonical_session_authority = {
                let session_store = session_store.clone();
                move |repository: Arc<dyn domain::local_event::LocalEventTransactionRepository>| {
                    session_store.set_local_event_repository_with_projection_codec(
                        repository,
                        canonical_generation.clone(),
                        Arc::new(
                            adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
                        ),
                    );
                }
            };
            if local_event_store.normal_admission_ready() {
                install_canonical_session_authority(session_event_repository);
            } else {
                let store = local_event_store.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        if store.cutover_ready() {
                            let repository: Arc<
                                dyn domain::local_event::LocalEventTransactionRepository,
                            > = store.clone();
                            install_canonical_session_authority(repository);
                            if !store.open_normal_admission_after_authority_install() {
                                log::error!(
                                    "local event store cutover admission acknowledgement was lost"
                                );
                            }
                            break;
                        }
                        if store.migration_blocked() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                });
            }
            let session_feedback_usecase = Arc::new(
                usecase::agent_session::feedback::SessionFeedbackUsecase::new(
                    local_event_store.clone(),
                    local_event_store.generation_id().to_string(),
                ),
            );
            let abandoned_feedback_recovery = session_feedback_usecase.clone();
            let abandoned_feedback_store = local_event_store.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if abandoned_feedback_store.migration_blocked() {
                        break;
                    }
                    if !abandoned_feedback_store.normal_admission_ready() {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        continue;
                    }
                    match abandoned_feedback_recovery
                        .recover_abandoned_reservations()
                        .await
                    {
                        Ok(recovered) => {
                            if recovered > 0 {
                                log::warn!(
                                    "recovered {recovered} abandoned session feedback reservations"
                                );
                            }
                            break;
                        }
                        Err(error) => {
                            log::warn!(
                                "abandoned session feedback recovery will retry: {error:?}"
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                    }
                }
            });
            let agent_session_notice_usecase = Arc::new(
                adaptor::controller::agent_session_notice_wiring::build_agent_session_notice_usecase(),
            );
            app.manage(session_feedback_usecase.clone());
            app.manage(agent_session_notice_usecase);
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
            spawn_startup_maintenance_after_normal_admission(
                data_dir.clone(),
                shared_repo_paths.clone(),
                local_event_store.clone(),
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
                let (workflow_usecase, workspace_tree_query_service) =
                    adaptor::controller::wiring::build_workflow_services_with_repository_worktrees(
                        data_dir.clone(),
                        repository_usecase.clone(),
                        config_repository.clone(),
                        config_secret_repository.clone(),
                        session_store_state,
                        app.handle().clone(),
                        local_event_store.clone(),
                    );
                let workflow_usecase = Arc::new(workflow_usecase);
                app.manage(Arc::new(workspace_tree_query_service));
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
                let notice_usecase = app
                    .state::<Arc<usecase::agent_session::notice::AgentSessionNoticeUsecase>>()
                    .inner()
                    .clone();
                adaptor::controller::agent_session_notice_wiring::register_agent_session_notice_publisher(
                    notice_usecase,
                    app.handle().clone(),
                );
            }

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
                let runtime_usecase = Arc::new(
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
                );
                adaptor::controller::event_log_recovery_wiring::register_event_log_recovery_listener(
                    runtime_session_store.clone(),
                    &runtime_usecase,
                );
                app.manage(runtime_usecase);
                let stored_lifecycle_registry = app
                    .state::<Arc<usecase::agent_session::backend_registry::AgentBackendRegistry>>()
                    .inner()
                    .clone();
                let stored_lifecycle_runtime = app
                    .state::<Arc<usecase::agent_session::runtime::AgentSessionRuntimeUsecase>>()
                    .inner()
                    .clone();
                let stored_lifecycle_notice = app
                    .state::<Arc<usecase::agent_session::notice::AgentSessionNoticeUsecase>>()
                    .inner()
                    .clone();
                let stored_lifecycle_open_tabs = app
                    .state::<Arc<usecase::agent_session::session::OpenTabRegistry>>()
                    .inner()
                    .clone();
                let workflow_node_restorer = Arc::new(
                    adaptor::controller::wiring::build_node_execution_lifecycle_usecase(
                        app.handle().clone(),
                        runtime_session_store.clone(),
                        stored_lifecycle_runtime.clone(),
                        stored_lifecycle_open_tabs,
                    ),
                );
                app.manage(workflow_node_restorer.clone());
                let stored_session_lifecycle = Arc::new(
                    adaptor::controller::wiring::build_stored_session_lifecycle_usecase(
                        runtime_session_store,
                        stored_lifecycle_registry,
                        stored_lifecycle_runtime,
                        workflow_node_restorer,
                        stored_lifecycle_notice,
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
            let session_feedback_load_usecase = Arc::new(
                usecase::agent_session::session_feedback_load::SessionFeedbackLoadUsecase::new(
                    agent_runtime.clone(),
                    session_feedback_usecase.clone(),
                ),
            );
            app.manage(session_feedback_load_usecase.clone());
            let operation_gate = Arc::new(
                adaptor::controller::agent_session_operation_wiring::RuntimeAgentSessionOperationGate::new(
                    agent_runtime.clone(),
                    session_store.clone(),
                    data_dir.clone(),
                ),
            );
            let operation_repository: Arc<
                dyn domain::local_event::LocalEventTransactionRepository,
            > = local_event_store.clone();
            let operation_authority: Arc<
                dyn usecase::agent_session::operation::OperationBindingAuthority,
            > = local_event_store.clone();
            let lifecycle_gate: Arc<
                dyn usecase::agent_session::operation::SessionLifecycleGate,
            > = operation_gate.clone();
            let stop_gate: Arc<dyn usecase::agent_session::operation::StopAdmissionGate> =
                operation_gate.clone();
            let send_operation_gate = Arc::new(
                adaptor::controller::agent_session_operation_wiring::RuntimeSendOperationGate::new(
                    agent_runtime.clone(),
                    session_store.clone(),
                    data_dir.clone(),
                ),
            );
            let send_gate: Arc<dyn usecase::agent_session::operation::SendAdmissionGate> =
                send_operation_gate.clone();
            let lifecycle_operation = Arc::new(
                usecase::agent_session::operation::SessionLifecycleOperationUsecase::new(
                    operation_repository.clone(),
                    operation_authority.clone(),
                    lifecycle_gate,
                    local_event_store.generation_id().to_string(),
                ),
            );
            app.manage(lifecycle_operation.clone());
            let stop_operation = Arc::new(
                usecase::agent_session::operation::StopOperationUsecase::new(
                    operation_repository.clone(),
                    operation_authority.clone(),
                    stop_gate,
                    local_event_store.generation_id().to_string(),
                ),
            );
            operation_gate.bind_stop_operation(Arc::downgrade(&stop_operation));
            adaptor::controller::agent_session_operation_wiring::bind_runtime_durable_stop_driver(
                &agent_runtime,
                stop_operation.clone(),
            );
            app.manage(stop_operation.clone());
            let send_operation = Arc::new(
                usecase::agent_session::operation::AgentSendOperationUsecase::new(
                    local_event_store.clone(),
                    local_event_store.clone(),
                    send_gate,
                    local_event_store.generation_id().to_string(),
                ),
            );
            operation_gate.bind_send_operation(Arc::downgrade(&send_operation));
            send_operation_gate.bind_status_sink(Arc::downgrade(&send_operation));
            adaptor::controller::agent_session_operation_wiring::bind_runtime_durable_workflow_send_driver(
                &agent_runtime,
                send_operation.clone(),
                session_store.clone(),
                data_dir.clone(),
            );
            adaptor::controller::agent_session_operation_wiring::bind_runtime_terminal_operation_participant_provider(
                &session_store,
                stop_operation.clone(),
                send_operation.clone(),
            );
            let pending_stop_recovery = stop_operation.clone();
            let pending_stop_recovery_store = local_event_store.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery_after_normal_admission(
                    "pending accepted Stop",
                    || {
                        startup_maintenance_admission(
                            pending_stop_recovery_store.normal_admission_ready(),
                            pending_stop_recovery_store.migration_blocked(),
                        )
                    },
                    || {
                        let recovery = pending_stop_recovery.clone();
                        async move { recovery.recover_pending_stops_pass().await }
                    },
                    std::time::Duration::from_millis(100),
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            let pending_send_recovery = send_operation.clone();
            let pending_send_recovery_store = local_event_store.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery_after_normal_admission(
                    "pending accepted send",
                    || {
                        startup_maintenance_admission(
                            pending_send_recovery_store.normal_admission_ready(),
                            pending_send_recovery_store.migration_blocked(),
                        )
                    },
                    || {
                        let recovery = pending_send_recovery.clone();
                        async move { recovery.recover_pending_provider_effects_pass().await }
                    },
                    std::time::Duration::from_millis(100),
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            app.manage(send_operation.clone());
            let permission_response_gate: Arc<
                dyn usecase::agent_session::operation::PermissionResponseGate,
            > = Arc::new(
                adaptor::controller::agent_session_operation_wiring::RuntimePermissionResponseOperationGate::new(
                    agent_runtime.clone(),
                    session_store.clone(),
                ),
            );
            let permission_response_operation = Arc::new(
                usecase::agent_session::operation::PermissionResponseOperationUsecase::new(
                    operation_repository.clone(),
                    operation_authority.clone(),
                    permission_response_gate,
                    local_event_store.generation_id().to_string(),
                ),
            );
            let pending_permission_recovery = permission_response_operation.clone();
            let pending_permission_recovery_store = local_event_store.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery_after_normal_admission(
                    "pending permission response",
                    || {
                        startup_maintenance_admission(
                            pending_permission_recovery_store.normal_admission_ready(),
                            pending_permission_recovery_store.migration_blocked(),
                        )
                    },
                    || {
                        let recovery = pending_permission_recovery.clone();
                        async move { recovery.recover_pending_permission_responses_pass().await }
                    },
                    std::time::Duration::from_millis(100),
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            app.manage(permission_response_operation.clone());
            let recovery_operation = Arc::new(
                usecase::agent_session::operation::RecoveryActionUsecase::new(
                    local_event_store.clone(),
                    local_event_store.clone(),
                    Arc::new(
                        adaptor::controller::agent_session_operation_wiring::ConservativeRecoveryExecutor::new(
                            stop_operation.clone(),
							lifecycle_operation.clone(),
							operation_gate.clone(),
							send_operation.clone(),
                            permission_response_operation.clone(),
                            local_event_store.clone(),
                        ),
                    ),
                    local_event_store.generation_id().to_string(),
                ),
            );
            app.manage(recovery_operation.clone());
            let caller_journal = Arc::new(
                usecase::agent_session::operation::CallerAttemptJournal::new(
                    local_event_store.clone(),
                    local_event_store.clone(),
                    local_event_store.generation_id().to_string(),
                ),
            );
            app.manage(caller_journal.clone());
            let open_tabs = app
                .state::<Arc<usecase::agent_session::session::OpenTabRegistry>>()
                .inner()
                .clone();
            let branch_diff_context = app
                .state::<Arc<dyn usecase::agent_session::context::BranchDiffContextPort>>()
                .inner()
                .clone();
            let workflow_runtime_usecase = Arc::new(
                adaptor::controller::wiring::build_workflow_runtime_usecase(
                    app.handle().clone(),
                    adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGatewayDeps {
                        repository_usecase: repository_usecase.clone(),
                        app_config: config_repository.clone(),
                        session_store: session_store.clone(),
                        agent_runtime: agent_runtime.clone(),
                        open_tabs,
                        branch_diff_context: branch_diff_context.clone(),
                        data_dir: Some(data_dir.clone()),
                        local_event_repository: local_event_store.clone(),
                        local_event_generation_id: local_event_store
                            .generation_id()
                            .to_string(),
                    },
                )
                .map_err(|error| format!("workflow recovery admission failed: {error}"))?,
            );
            send_operation_gate
                .bind_workflow_runtime(Arc::downgrade(&workflow_runtime_usecase));
            let workflow_runtime_agent_notifier = Arc::new(
                adaptor::gateway::agent_session::WorkflowRuntimeAgentSessionNotifier::new(
                    workflow_runtime_usecase.clone(),
                    session_store.clone(),
                ),
            );
            agent_runtime
                .set_workflow_turn_complete_notifier(workflow_runtime_agent_notifier.clone());
            agent_runtime
                .set_workflow_stall_notifier(workflow_runtime_agent_notifier.clone());
            let pending_workflow_recovery = workflow_runtime_usecase.clone();
            let pending_workflow_recovery_store = local_event_store.clone();
            let pending_turn_completion_recovery = workflow_runtime_agent_notifier.clone();
            tauri::async_runtime::spawn(async move {
                run_startup_recovery_after_normal_admission(
                    "pending workflow turn-completion/orphan",
                    || {
                        startup_maintenance_admission(
                            pending_workflow_recovery_store.normal_admission_ready(),
                            pending_workflow_recovery_store.migration_blocked(),
                        )
                    },
                    || {
                        let workflow = pending_workflow_recovery.clone();
                        let turn_completion = pending_turn_completion_recovery.clone();
                        async move {
                            // Never orphan-interrupt an execution while its
                            // exact turn-completion handoff cannot be replayed.
                            let recovered = turn_completion
                                .recover_pending_turn_completions()
                                .await?;
                            workflow
                                .recover_startup()
                                .await
                                .map_err(|error| error.to_string())?;
                            Ok::<usize, String>(recovered)
                        }
                    },
                    std::time::Duration::from_millis(100),
                    std::time::Duration::from_millis(50),
                    std::time::Duration::from_secs(1),
                )
                .await;
            });
            app.manage(workflow_runtime_usecase.clone());

            let workflow_query_usecase = app
                .state::<adaptor::controller::state::AppState>()
                .workflow_usecase
                .clone();
            let local_api_shutdown_target: LocalApiShutdownTarget =
                Arc::new(parking_lot::RwLock::new(None));
            let shutdown_local_api: Arc<dyn Fn() + Send + Sync> = Arc::new({
                let target = local_api_shutdown_target.clone();
                move || {
                    if let Some(shutdown) = target.read().clone() {
                        shutdown();
                    }
                }
            });
            let shutdown_coordinator =
                adaptor::controller::application_lifecycle::build_shutdown_coordinator(
                    local_event_store.clone(),
                    agent_runtime.clone(),
                    workflow_runtime_usecase.clone(),
                    lifecycle_operation,
                    shutdown_local_api,
                );
            let process_actions = Arc::new(
                adaptor::controller::application_lifecycle::ApplicationProcessActionDispatcher::default(),
            );
            app.manage(process_actions.clone());
            let migration_quit_boot_settlement = shutdown_coordinator.clone();
            tauri::async_runtime::spawn(async move {
                let mut retry_delay = std::time::Duration::from_millis(50);
                loop {
                    match migration_quit_boot_settlement
                        .settle_previous_boot_migration_quit()
                        .await
                    {
                        Ok(_) => break,
                        Err(error) => {
                            log::warn!(
                                "previous-boot migration quit settlement will retry: {error:?}"
                            );
                            tokio::time::sleep(retry_delay).await;
                            retry_delay = retry_delay
                                .saturating_mul(2)
                                .min(std::time::Duration::from_secs(1));
                        }
                    }
                }
            });
            let local_api_binding =
                infrastructure::local_api::LocalApiServerBinding::bind(data_dir.clone())
                    .map_err(|error| format!("local API の起動に失敗しました: {error}"))?;
            let local_api_router = adaptor::controller::api::build_router(
                Arc::new(workflow_query_usecase.read_usecase()),
                workflow_runtime_usecase.clone(),
                local_api_binding.bearer_token(),
                Some(adaptor::controller::api::AgentSessionApiDeps::new(
                    send_operation,
                    permission_response_operation,
                    stop_operation,
                    recovery_operation,
                    session_feedback_usecase,
                    session_feedback_load_usecase,
                    shutdown_coordinator.clone(),
                    process_actions.clone(),
                    local_event_store.clone(),
                    caller_journal.clone(),
                    app.handle().clone(),
                )),
            );
            let local_api =
                local_api_binding.start(local_api_router, &tokio::runtime::Handle::current());
            *local_api_shutdown_target.write() = Some(Arc::new({
                let local_api = local_api.clone();
                move || local_api.shutdown()
            }));
            app.manage(local_api.clone());

            // CLI / Agent / 外部編集由来の review comment 変更を UI へ通知する。
            infrastructure::comment::watcher::spawn_review_comments_watcher(
                app.handle().clone(),
                data_dir.clone(),
            );

            infrastructure::platform::menu::setup_menu(app)?;
            app.manage(shutdown_coordinator.clone());
            let quit_app = app.handle().clone();
            let quit_process_actions = process_actions.clone();
            let quit_ingress = Arc::new(
                adaptor::controller::application_lifecycle::ApplicationQuitIngress::new(
                    move |intent| {
                        adaptor::controller::application_lifecycle::request_application_quit(
                            quit_app.clone(),
                            shutdown_coordinator.clone(),
                            quit_process_actions.clone(),
                            intent,
                        );
                    },
                ),
            );
            app.manage(quit_ingress.clone());
            infrastructure::platform::tray::setup_tray(app, move |_app| {
                quit_ingress.request(
                    usecase::shutdown_coordinator::ApplicationQuitIntent::Exit { code: 0 },
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
    use super::{
        run_startup_recovery_after_normal_admission, startup_maintenance_admission,
        StartupMaintenanceAdmission, StartupRecoveryWorkerExit,
    };
    use std::sync::Arc;

    #[test]
    fn startup_effects_are_admitted_only_after_verified_normal_cutover() {
        assert_eq!(
            startup_maintenance_admission(false, false),
            StartupMaintenanceAdmission::AwaitCutover
        );
        assert_eq!(
            startup_maintenance_admission(false, true),
            StartupMaintenanceAdmission::Abandoned
        );
        assert_eq!(
            startup_maintenance_admission(true, true),
            StartupMaintenanceAdmission::Abandoned,
            "an inconsistent blocked authority must fail closed"
        );
        assert_eq!(
            startup_maintenance_admission(true, false),
            StartupMaintenanceAdmission::Admitted
        );
    }

    #[tokio::test]
    async fn f12_startup_recovery_retries_first_page_and_drive_failures_then_reaches_fresh_page_quiescence(
    ) {
        use std::collections::VecDeque;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let admission_calls = Arc::new(AtomicUsize::new(0));
        let scripted = Arc::new(std::sync::Mutex::new(VecDeque::from([
            Err("first pending page query failed"),
            Ok(1),
            Err("permission drive failed before effect reservation"),
            // A pending identity inserted while the preceding page was being
            // consumed appears only in this fresh pass.
            Ok(1),
            Ok(0),
            Ok(0),
        ])));
        let recovery_calls = Arc::new(AtomicUsize::new(0));

        let exit = run_startup_recovery_after_normal_admission(
            "f12-scripted",
            {
                let admission_calls = admission_calls.clone();
                move || {
                    if admission_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        StartupMaintenanceAdmission::AwaitCutover
                    } else {
                        StartupMaintenanceAdmission::Admitted
                    }
                }
            },
            {
                let scripted = scripted.clone();
                let recovery_calls = recovery_calls.clone();
                move || {
                    recovery_calls.fetch_add(1, Ordering::SeqCst);
                    let outcome = scripted
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .pop_front()
                        .expect("bounded scripted pass");
                    async move { outcome }
                }
            },
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        )
        .await;

        assert_eq!(exit, StartupRecoveryWorkerExit::Quiescent);
        assert_eq!(recovery_calls.load(Ordering::SeqCst), 6);
        assert!(admission_calls.load(Ordering::SeqCst) > recovery_calls.load(Ordering::SeqCst));
        assert!(
            scripted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "the worker must not stop after either transient failure or the first empty page"
        );
    }

    #[tokio::test]
    async fn f12_startup_and_manual_overlap_claim_the_same_effect_identity_once() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let claimed = Arc::new(AtomicBool::new(false));
        let effects = Arc::new(AtomicUsize::new(0));
        let passes = Arc::new(AtomicUsize::new(0));
        let exit = run_startup_recovery_after_normal_admission(
            "f12-manual-overlap",
            || StartupMaintenanceAdmission::Admitted,
            {
                let claimed = claimed.clone();
                let effects = effects.clone();
                let passes = passes.clone();
                move || {
                    let pass = passes.fetch_add(1, Ordering::SeqCst);
                    let claimed = claimed.clone();
                    let effects = effects.clone();
                    async move {
                        if pass == 0 {
                            // Manual action wins the same durable reservation;
                            // startup observes the CAS loser and performs no
                            // replacement effect.
                            if claimed
                                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                                .is_ok()
                            {
                                effects.fetch_add(1, Ordering::SeqCst);
                            }
                            if claimed
                                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                                .is_ok()
                            {
                                effects.fetch_add(1, Ordering::SeqCst);
                            }
                            Ok::<usize, &'static str>(1)
                        } else {
                            Ok(0)
                        }
                    }
                }
            },
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        )
        .await;

        assert_eq!(exit, StartupRecoveryWorkerExit::Quiescent);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(passes.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn f12_blocked_migration_never_enters_a_recovery_pass() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let exit = run_startup_recovery_after_normal_admission(
            "f12-blocked",
            || StartupMaintenanceAdmission::Abandoned,
            {
                let calls = calls.clone();
                move || {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { Ok::<usize, &'static str>(0) }
                }
            },
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        )
        .await;

        assert_eq!(exit, StartupRecoveryWorkerExit::AdmissionAbandoned);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

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
