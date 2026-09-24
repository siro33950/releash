use crate::{adaptor, domain, infrastructure, other, terminal_surface, usecase};
use adaptor::gateway::app_config::{load_or_create_config, AppConfig};
use domain::app_config::{ConfigRepository, ConfigSecretRepository, NotionConfigRepository};
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) struct Daemon {
    shutdown: Arc<dyn domain::application_lifecycle::ApplicationShutdownGateway>,
    exit: tokio::sync::mpsc::Receiver<i32>,
}

impl Daemon {
    pub(crate) async fn wait(mut self) -> Result<std::convert::Infallible, String> {
        let code = self.exit.recv().await.ok_or("daemon exit channel closed")?;
        self.exit.close();
        usecase::application_lifecycle::shutdown(self.shutdown.as_ref()).await;
        println!("releash-shutdown-complete");
        std::process::exit(code)
    }
}

pub(crate) async fn compose(
    data_dir: PathBuf,
    #[cfg(any(target_os = "macos", target_os = "linux"))] provider_initial_search_path: Result<
        std::ffi::OsString,
        infrastructure::process::search_path::LoginShellPathError,
    >,
) -> Result<Daemon, Box<dyn std::error::Error>> {
    other::telemetry::set_startup_origin(std::time::Instant::now());
    let (exit_sender, exit_receiver) = tokio::sync::mpsc::channel(1);
    let app_data = super::app_data_composition::ProductionAppDataComposition::new(data_dir.clone());
    let local_event_store = app_data.open_local_event_store().map_err(|error| {
        let failure =
            usecase::application_startup::StartupFailure::new(classify_startup_failure(error));
        log::error!(
            "application startup admission failed: {} ({})",
            failure.safe_description,
            failure.correlation_id
        );
        std::io::Error::other(format!(
            "{} ({})",
            failure.safe_description, failure.correlation_id
        ))
    })?;
    let startup_authority =
        Arc::new(usecase::application_startup::ApplicationStartupAuthority::ready());
    let push_sink = Arc::new(infrastructure::push::PushSink::new());

    let projected_local_event_repository: Arc<
        dyn domain::local_event::LocalEventTransactionRepository,
    > = local_event_store.clone();
    let terminal_surface_runtime = terminal_surface::TerminalSurfaceRuntime::new(data_dir.clone());
    let terminal_surface = terminal_surface_runtime.application();
    let review_comment_usecase =
        Arc::new(adaptor::controller::wiring::build_review_comment_usecase());
    let file_watchers = Arc::new(infrastructure::file_watcher::FileWatcherManager::default());
    let shared_repo_paths: adaptor::gateway::repository::repo_paths::SharedRepoPaths =
        Arc::new(parking_lot::RwLock::new(Vec::new()));
    let workspace_state_store = Arc::new(
        adaptor::gateway::workspace_state::WorkspaceStateStore::new(data_dir.clone()),
    );

    let config_path = data_dir.join("releash.toml");
    let config = load_or_create_config(&config_path)
        .map_err(|e| format!("設定ファイルの読み込みに失敗: {e}"))?;
    let telemetry = infrastructure::telemetry::init_telemetry(
        config.telemetry.crash_reporting,
        config.telemetry.performance_telemetry,
    );

    let app_config = Arc::new(AppConfig::new(config, config_path));
    let config_repository: Arc<dyn ConfigRepository> = app_config.clone();
    let config_secret_repository: Arc<dyn ConfigSecretRepository> = app_config.clone();
    let notion_config_repository: Arc<dyn NotionConfigRepository> = app_config.clone();
    let notion_api_gateway: Arc<dyn domain::notion::NotionApiGateway> =
        Arc::new(adaptor::gateway::notion::NotionApiGatewayImpl::new());

    let provider_executable_config: Arc<
        dyn domain::agent_session::ProviderExecutableConfigRepository,
    > = if let Some(fixture) = performance_provider_fixture_executable() {
        let (claude, codex) = select_provider_agent_executables(
            "claude".to_string(),
            "codex".to_string(),
            Some(fixture),
        );
        Arc::new(
            adaptor::gateway::agent_session::InMemoryProviderExecutableConfigRepository::new(
                Some(claude),
                Some(codex),
            )
            .map_err(|error| {
                format!("Provider performance fixture設定の初期化に失敗: {error:?}")
            })?,
        )
    } else {
        app_config.clone()
    };
    let provider_history_home =
        dirs::home_dir().unwrap_or_else(|| data_dir.join("provider-history-unavailable"));
    let agent_sessions =
                adaptor::controller::agent_session_wiring::compose_agent_sessions(
                    adaptor::controller::agent_session_wiring::AgentSessionCompositionInput {
                        store: local_event_store.clone(),
                        data_dir: data_dir.clone(),
                        provider_executable_config,
                        provider_executable_probe: Arc::new(
                            #[cfg(any(target_os = "macos", target_os = "linux"))]
                            adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::with_initial_search_path(
                                provider_initial_search_path.clone(),
                            ),
                            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                            adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::new(),
                        ),
                        claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| provider_history_home.join(".claude")),
                        codex_home: std::env::var_os("CODEX_HOME")
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| provider_history_home.join(".codex")),
                        cli_binary: infrastructure::platform::path_aliases::alias_name_for_profile(
                            infrastructure::platform::path_aliases::BuildProfile::current(),
                        )
                        .to_string(),
                        terminal: terminal_surface.clone(),
                        change_notifier: Arc::new(
                            adaptor::gateway::push::ClientAgentSessionChangeNotifier::new(
                                push_sink.clone(),
                            ),
                        ),
                    },
                )
                .map_err(|error| format!("Provider availability初期化失敗: {error:?}"))?;
    let agent_session_launch = agent_sessions.launch.clone();
    let agent_session_initial_instruction = agent_sessions.initial_instruction.clone();
    let agent_session_lifecycle = agent_sessions.lifecycle.clone();
    let agent_session_exit = agent_sessions.exit.clone();
    let provider_availability = agent_sessions.availability_reader.clone();
    let provider_lifecycle_ingress = agent_sessions.lifecycle_ingress.clone();
    let provider_execution_tree_stops = agent_sessions.execution_tree_stops.clone();
    let started_execution_tree_registrations = agent_sessions.execution_tree_registrations.clone();
    let provider_session_title_ingestion = agent_sessions.provider_session_title_ingestion.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(domain::agent_session::PROVIDER_SESSION_TITLE_TICK_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            provider_session_title_ingestion.ingest_due().await;
        }
    });
    let provider_agent_terminal_events = terminal_surface.subscribe_events();
    let shutdown_provider_exit_observer: Arc<dyn Fn() + Send + Sync> = Arc::new({
        let cancellation = provider_agent_terminal_events.cancellation.clone();
        move || cancellation.cancel()
    });
    tokio::spawn(
        adaptor::controller::agent_session_exit_observer::run_agent_session_exit_observer(
            provider_agent_terminal_events,
            agent_session_exit.clone(),
        ),
    );

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

    let repository_usecase = Arc::new(
        adaptor::controller::wiring::build_repository_usecase_with_worktree_terminals(
            terminal_surface.clone(),
            Arc::new(usecase::worktree_operation::WorktreeOperations::new(Arc::new(
                adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks::new(&data_dir),
            ))),
        ),
    );

    use adaptor::controller::state::AppState;
    use adaptor::gateway::repository::repo_paths::RepoPathsGateway;
    use usecase::repo_paths_usecase::RepoPathsUsecase;

    let repo_paths_gateway =
        RepoPathsGateway::new(shared_repo_paths.clone(), config_repository.clone());

    let state_subscriptions = usecase::state_subscription::StateSubscriptionUsecase::new(
        shared_repo_paths.read().clone(),
        Arc::new(adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let repo_paths_notifier = Arc::new(
        adaptor::gateway::repository::notify::RepoPathsNotifyGateway::new(
            state_subscriptions.publisher(),
        ),
    );
    let repo_paths_usecase = Arc::new(RepoPathsUsecase::new(
        Arc::new(repo_paths_gateway),
        repo_paths_notifier,
    ));

    let code_usecase = Arc::new(adaptor::controller::wiring::build_code_usecase());
    let git_host_usecase = Arc::new(adaptor::controller::wiring::build_git_host_usecase());
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
    let repository_state = Arc::new(usecase::repository_state::RepositoryStateService::new(
        repository_state_repository,
        repository_scanner,
        Arc::new(
            adaptor::gateway::repository::state::ClientRepositoryStateNotifier::new(
                push_sink.clone(),
            ),
        ),
        Arc::new(
            adaptor::gateway::repository::state::NotifyRepositoryStateWatcher::new(
                repository_usecase.clone(),
            ),
        ),
        Arc::new(adaptor::gateway::repository::state::TokioRepositoryStateWorkerRuntime),
        Arc::new(adaptor::gateway::repository::state::FsWorktreePathNormalizer),
    ));

    let review_usecase = Arc::new(usecase::review_usecase::ReviewUsecase::new(
        repository_state.clone(),
        code_usecase.clone(),
    ));
    let workflows_dir =
        adaptor::gateway::workflow::WorkflowDefinitionFileRepository::default_workflows_dir();
    if let Err(error) = adaptor::gateway::workflow::lua::generate_editor_support(&workflows_dir) {
        log::warn!("Lua editor support generation failed at startup: {error}");
    }
    let node_processes = Arc::new(
        adaptor::gateway::workflow::node_process::WorkflowNodeProcesses::new(
            terminal_surface.clone(),
        ),
    );
    let (workflow_usecase, workspace_query_service) =
        adaptor::controller::wiring::build_workflow_services_with_repository_worktrees(
            data_dir.clone(),
            repository_usecase.clone(),
            config_repository.clone(),
            config_secret_repository.clone(),
            local_event_store.clone(),
            node_processes.clone(),
        );
    let workflow_usecase = Arc::new(workflow_usecase);

    let notion_usecase = Arc::new(usecase::notion::usecase::NotionUsecase::new(
        notion_config_repository.clone(),
        notion_api_gateway.clone(),
    ));

    let repository_state_for_watcher = repository_state.clone();
    let workspace_list = Arc::new(
        crate::adaptor::controller::wiring::build_workspace_list_usecase(
            repo_paths_usecase.clone(),
            repository_state.clone(),
            workflow_usecase.clone(),
            git_host_usecase.clone(),
        )
        .with_notifier({
            let push = push_sink.clone();
            move || crate::adaptor::gateway::push::BackendPush::WorkspaceListChanged.emit(&push)
        }),
    );
    let app_state = AppState {
        workspace_list,
        repository_usecase: repository_usecase.clone(),
        repository_state,
        repo_paths_usecase,
        code_usecase,
        review_usecase,
        notion_usecase,
        workflow_usecase: workflow_usecase.clone(),
        terminal_surface: terminal_surface.clone(),
        git_host_usecase,
    };
    let workflow_runtime_usecase = Arc::new(
        adaptor::controller::wiring::build_workflow_runtime_usecase(
            adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies {
                processes: node_processes.clone(),
                store: Some(local_event_store.clone()),
                config: Some(config_repository.clone()),
                secrets: Some(config_secret_repository.clone()),
                push: push_sink.clone(),
            },
            adaptor::gateway::workflow::WorkflowRuntimeCommandGatewayDeps {
                node_processes,
                repository_usecase: repository_usecase.clone(),
                app_config: config_repository.clone(),
                workspace_query: workspace_query_service.clone(),
                agent_session_launch: agent_session_launch.clone(),
                agent_session_initial_instruction: agent_session_initial_instruction.clone(),
                agent_session_lifecycle: agent_session_lifecycle.clone(),
                provider_availability: provider_availability.clone(),
                isolated_worktrees: Arc::new(
                    adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway,
                ),
            },
        )
        .map_err(|error| format!("workflow recovery admission failed: {error}"))?,
    );
    let workspace_node_resolver: Arc<dyn usecase::workflow::WorkspaceNodeActionResolver> =
        workflow_usecase.clone();
    let workspace_node_command_usecase = Arc::new(
        adaptor::controller::wiring::build_workspace_node_command_usecase(
            workspace_node_resolver,
            workflow_runtime_usecase.clone(),
            agent_sessions.rename.clone(),
        ),
    );
    provider_execution_tree_stops.bind(workflow_runtime_usecase.clone());
    started_execution_tree_registrations.bind(workflow_runtime_usecase.clone());
    migrate_legacy_execution_archives(
        &data_dir,
        local_event_store.clone(),
        &workflow_runtime_usecase,
    )
    .await?;
    let pending_workflow_recovery = workflow_runtime_usecase.clone();
    tokio::spawn(async move {
        if let Err(error) = pending_workflow_recovery.recover_startup().await {
            log::warn!("workflow startup advancement failed: {error}");
        }
    });

    let workflow_query_usecase = workflow_usecase.clone();
    infrastructure::comment::watcher::spawn_review_comments_watcher(
        adaptor::gateway::comment::state_dir(&data_dir),
        Arc::new({
            let app = push_sink.clone();
            move || adaptor::gateway::push::BackendPush::ReviewCommentsChanged("*").emit(&app)
        }),
    );

    adaptor::controller::wiring::spawn_startup_app_data_gc(
        app_data.clone(),
        shared_repo_paths.clone(),
        projected_local_event_repository.clone(),
        workflow_runtime_usecase.clone(),
    );
    let local_api_binding =
        infrastructure::local_api::LocalApiServerBinding::bind(data_dir.clone())
            .map_err(|error| format!("local API の起動に失敗しました: {error}"))?;
    let mut client_dispatch = adaptor::controller::client::ClientCommandDispatch::new(
        repository_usecase.clone(),
        startup_authority.clone(),
    );
    let dependencies = super::client::ClientDependencies {
        application_startup_authority: Some(startup_authority),
        workspace_node_command_usecase: Some(workspace_node_command_usecase),
        app_state: Some(app_state),
        workspace_state_store: Some(workspace_state_store),
        agent_session_lifecycle_usecase: Some(agent_session_lifecycle),
        agent_session_launch_usecase: Some(agent_session_launch),
        agent_session_read_usecase: Some(agent_sessions.read),
        provider_availability_usecase: Some(agent_sessions.provider_availability),
        agent_session_history_read_usecase: Some(agent_sessions.history_read),
        provider_hook_health_read_usecase: Some(agent_sessions.hook_health_read),
        review_comment_usecase: Some(review_comment_usecase),
        config_repository: Some(config_repository.clone()),
        workflow_runtime_usecase: Some(workflow_runtime_usecase.clone()),
        editor_launcher: Arc::new(adaptor::gateway::external_editor::NativeEditorLauncherGateway),
        watcher: Arc::new(usecase::watcher::WatcherUsecase::new(
            Some(repository_state_for_watcher),
            Arc::new(
                adaptor::gateway::repository::file_watcher::FileWatcherGateway::new(
                    file_watchers,
                    push_sink.clone(),
                ),
            ),
        )),
        data_dir: Ok(data_dir),
        comment_notify: Arc::new(adaptor::gateway::push::CommentChangeGateway::new(
            push_sink.clone(),
        )),
        process_port: Arc::new(
            adaptor::gateway::application_lifecycle::DaemonProcessActionPort(exit_sender),
        ),
    };
    client_dispatch.register_dependencies(&dependencies);
    let client_dispatch = Arc::new(client_dispatch);

    let local_api_router = adaptor::controller::api::build_router(
        Arc::new(workflow_query_usecase.read_usecase()),
        workflow_runtime_usecase.clone(),
        local_api_binding.bearer_token(),
        local_api_binding.client_bearer_token(),
        Some(adaptor::controller::api::TerminalApiDeps::new(
            terminal_surface.clone(),
        )),
        Some(
            adaptor::controller::api::ClientApiDeps::new(
                client_dispatch.clone(),
                adaptor::gateway::push::ClientPushGateway::new(push_sink.clone()),
                dependencies.watcher.clone(),
            )
            .with_state_subscriptions(state_subscriptions)
            .with_desktop_settings(usecase::app_config::AppConfigUsecase::new(
                config_repository,
            )),
        ),
        Some(provider_lifecycle_ingress.clone()),
    );
    let local_api = local_api_binding.start(local_api_router, &tokio::runtime::Handle::current());

    Ok(Daemon {
        shutdown: Arc::new(
            adaptor::gateway::application_lifecycle::DaemonShutdownGateway {
                server: local_api,
                workflow: workflow_runtime_usecase,
                terminal: terminal_surface,
                stop_observer: shutdown_provider_exit_observer,
                telemetry: parking_lot::Mutex::new(telemetry),
            },
        ),
        exit: exit_receiver,
    })
}

async fn migrate_legacy_execution_archives(
    data_dir: &std::path::Path,
    store: Arc<adaptor::gateway::local_event_store::LocalEventStore>,
    runtime: &usecase::workflow::WorkflowRuntimeUsecase,
) -> Result<(), String> {
    let repository =
        adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::new(store, data_dir);
    runtime
        .migrate_execution_archives(&repository)
        .await
        .map_err(|error| format!("execution archive migration failed: {error}"))
}

fn select_provider_agent_executables(
    claude_executable: String,
    codex_executable: String,
    fixture_executable: Option<String>,
) -> (String, String) {
    match fixture_executable.filter(|path| !path.trim().is_empty()) {
        Some(path) => (path.clone(), path),
        None => (claude_executable, codex_executable),
    }
}

#[cfg(feature = "performance")]
fn performance_provider_fixture_executable() -> Option<String> {
    std::env::var("RELEASH_PERFORMANCE_PROVIDER_FIXTURE_EXECUTABLE").ok()
}

#[cfg(not(feature = "performance"))]
fn performance_provider_fixture_executable() -> Option<String> {
    None
}

fn classify_startup_failure(
    error: adaptor::gateway::local_event_store::store::LocalEventStoreOpenError,
) -> usecase::application_startup::StartupFailureKind {
    use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
    use usecase::application_startup::StartupFailureKind as K;

    match error {
        E::WriterLockHeld => K::StoreInUse,
        E::StorageUnavailable => K::StorageUnavailable,
        E::UnsupportedRuntime => K::UnsupportedRuntime,
        E::UnsupportedStoreVersion => K::UnsupportedStoreVersion,
        E::InitializationStateInvalid => K::InitializationStateInvalid,
        E::StoreValidationFailed => K::StoreValidationFailed,
        E::SchemaEvolutionFailed => K::SchemaEvolutionFailed,
    }
}

#[cfg(test)]
#[path = "daemon_test.rs"]
mod daemon_tests;
