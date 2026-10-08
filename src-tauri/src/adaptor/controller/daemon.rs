use crate::adaptor::controller::terminal_surface_runtime as terminal_surface;
use crate::{adaptor, domain, infrastructure, usecase};
use adaptor::gateway::app_config::{load_or_create_config, AppConfig};
use domain::app_config::{ConfigRepository, ConfigSecretRepository, NotionConfigRepository};
use std::path::PathBuf;
use std::sync::Arc;

fn client_priority_level(path: &str) -> Option<&'static str> {
    match path.rsplit('/').next().unwrap_or_default() {
        "GetServerInfo" | "ReportTerminalProcessed" => None,
        "WriteTerminalSurface"
        | "WritePathsToTerminalSurface"
        | "ResizeTerminalSurface"
        | "StartStateSubscription"
        | "StopStateSubscription" => Some("interactive"),
        "StartWorkflow"
        | "AbortWorkflow"
        | "ApproveWorkspaceNode"
        | "ApproveWorkflowNode"
        | "RetryWorkspaceNode"
        | "ResumeWorkspaceSessionNode"
        | "WorkflowSubmitOutput"
        | "ReceiveProviderSignal" => Some("workflow"),
        _ => Some("default"),
    }
}

fn service_timeout(name: &str) -> std::time::Duration {
    use crate::adaptor::presenter::client::descriptor;
    let options = descriptor::client_service().options();
    std::time::Duration::from_millis(
        descriptor::option(&options, name)
            .as_u32()
            .expect("timeout option")
            .into(),
    )
}
pub fn default_timeout() -> std::time::Duration {
    service_timeout("default_timeout_ms")
}
fn shutdown_timeout() -> std::time::Duration {
    service_timeout("shutdown_timeout_ms")
}

pub async fn shutdown_with_deadline(
    gateway: &dyn domain::application_lifecycle::ApplicationShutdownGateway,
    server: &infrastructure::local_api::LocalApiServer,
) {
    let timeout = shutdown_timeout();
    let context = crate::common::operation_context::current();
    tokio::select! {
        biased;
        _ = crate::common::operation_context::wait(&context, tokio::time::sleep(timeout)) => {
            log::error!(
                "application shutdown: {} second deadline exceeded; exiting",
                timeout.as_secs()
            );
        }
        () = usecase::application_lifecycle::shutdown(gateway) => {}
    }
    server.shutdown();
}

pub fn client_priority_interceptor(
) -> adaptor::controller::api::client_priority::PriorityInterceptor {
    let limits = Arc::new(crate::common::concurrency::PriorityLimits::new(
        64,
        &[("interactive", 30), ("workflow", 40), ("default", 120)],
        50,
    ));
    let events = Arc::new(adaptor::controller::api::client_priority::PriorityFailureReporter);
    adaptor::controller::api::client_priority::PriorityInterceptor {
        gate: Arc::new(crate::common::priority::PriorityGate::new(
            limits,
            client_priority_level,
            events,
        )),
    }
}

pub struct Daemon {
    shutdown: Arc<dyn domain::application_lifecycle::ApplicationShutdownGateway>,
    server: Arc<infrastructure::local_api::LocalApiServer>,
    exit: tokio::sync::mpsc::Receiver<i32>,
    daemon: usecase::daemon::DaemonUsecase,
}

impl Daemon {
    pub async fn wait(mut self) -> Result<std::convert::Infallible, String> {
        let code = self.exit.recv().await.ok_or("daemon exit channel closed")?;
        self.exit.close();
        shutdown_with_deadline(self.shutdown.as_ref(), &self.server).await;
        self.daemon.stopped().await;
        std::process::exit(code)
    }
}

pub async fn compose(
    data_dir: PathBuf,
    #[cfg(any(target_os = "macos", target_os = "linux"))] provider_initial_search_path: Result<
        std::ffi::OsString,
        infrastructure::process::search_path::LoginShellPathError,
    >,
) -> Result<Daemon, Box<dyn std::error::Error>> {
    let pid = std::process::id();
    let identity = domain::daemon::DaemonIdentity {
        daemon_id: uuid::Uuid::new_v4().simple().to_string(),
        pid,
        process_started_at: infrastructure::local_api::process_start_time(pid)
            .ok_or("failed to resolve daemon process identity")?,
    };
    let protocol = adaptor::presenter::client::descriptor::protocol();
    let installation = Arc::new(usecase::installation::InstallationUsecase(Arc::new(
        adaptor::gateway::installation::LocalInstallationService,
    )));
    let cli_installation = installation
        .cli_installation()
        .map_err(|error| error.to_string())?;
    let daemon_repository = Arc::new(adaptor::gateway::daemon::InMemoryDaemonRepository::new(
        identity,
        env!("CARGO_PKG_VERSION").into(),
        protocol,
        cli_installation,
    ));
    let daemon = usecase::daemon::DaemonUsecase::new(daemon_repository.clone());
    let retry_limiter = Arc::new(crate::common::retry::RetryLimiter::new());
    let failure_store = Arc::new(adaptor::gateway::failure_records::FailureRecordStore::default());
    infrastructure::telemetry::metrics::set_startup_origin(std::time::Instant::now());
    let (exit_sender, exit_receiver) = tokio::sync::mpsc::channel(1);
    let app_data = super::app_data_composition::ProductionAppDataComposition::new(
        data_dir.clone(),
        retry_limiter.clone(),
    );
    let local_event_store = match app_data.open_local_event_store() {
        Ok(store) => store,
        Err(error) => {
            daemon
                .fail(domain::daemon::StartupFailure::new(
                    error.into(),
                    uuid::Uuid::new_v4().to_string(),
                ))
                .await;
            let domain::daemon::ServingStatus::Failed(failure) = daemon.info().await.serving_status
            else {
                unreachable!("failed startup")
            };
            log::error!(
                "application startup admission failed: {} ({})",
                failure.safe_description,
                failure.correlation_id
            );
            return Err(std::io::Error::other(format!(
                "{} ({})",
                failure.safe_description, failure.correlation_id
            ))
            .into());
        }
    };
    let state_presenter =
        Arc::new(adaptor::presenter::state_subscription::StateSubscriptionPresenter::new());
    let state_subscriptions =
        usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
            state_presenter.clone(),
            adaptor::controller::state_subscription::drive(Arc::new(|| {
                let period = domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration();
                Box::pin(infrastructure::timer::ticks_after(period, period))
            })),
        );
    let daemon = daemon.with_state_publisher(state_subscriptions.clone());
    let failure_output: Arc<usecase::failure::FailureRecordingUsecase> =
        Arc::new(usecase::failure::FailureRecordingUsecase::new(
            failure_store.clone(),
            Some(state_subscriptions.clone()),
        ));
    let retrying = usecase::retry::Retrying::new(retry_limiter.clone(), failure_output.clone());

    let projected_local_event_repository: Arc<
        dyn domain::local_event::LocalEventTransactionRepository,
    > = local_event_store.clone();
    let terminal_surface_runtime = terminal_surface::TerminalSurfaceRuntime::new(
        Arc::new(terminal_surface::BackgroundWork::new(
            retrying.clone(),
            failure_store.clone(),
            tokio::runtime::Handle::current(),
        )),
        data_dir.clone(),
    );
    let terminal_surface = terminal_surface_runtime.application();
    let terminal_presenter = Arc::new(
        adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
            &state_presenter,
        ),
    );
    terminal_surface.connect_state(terminal_presenter.clone())?;
    let terminal_subscriptions =
        usecase::terminal_surface::subscription::TerminalSubscriptionUsecase::new(
            terminal_presenter,
            Some(terminal_surface.clone()),
            crate::adaptor::controller::terminal_subscription::start(),
        );

    let review_comment_usecase = Arc::new(
        adaptor::controller::wiring::build_review_comment_usecase()
            .with_subscriptions(state_subscriptions.clone()),
    );
    let session_review_usecase = Arc::new(usecase::comment::SessionReviewUsecase::new(
        usecase::comment::ReviewContextUsecase::new(
            Arc::new(
                adaptor::gateway::agent_session::LocalAgentSessionRepository::new(
                    local_event_store.clone(),
                ),
            ),
            Arc::new(
                adaptor::gateway::workflow::worktree_context::StoredWorkspaceWorktreePathQuery::new(
                    data_dir.clone(),
                    retry_limiter.clone(),
                ),
            ),
        ),
        review_comment_usecase.clone(),
    ));
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
    let notion_api_gateway: Arc<dyn domain::notion::NotionApiGateway> = Arc::new(
        adaptor::gateway::notion::NotionApiGatewayImpl::new(retry_limiter.clone()),
    );

    let provider_executable_config: Arc<
        dyn domain::agent_session::ProviderExecutableConfigRepository,
    > = app_config.clone();
    let provider_history_home =
        dirs::home_dir().unwrap_or_else(|| data_dir.join("provider-history-unavailable"));
    let history_paths = vec![
        std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| provider_history_home.join(".claude"))
            .to_string_lossy()
            .into_owned(),
        std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| provider_history_home.join(".codex"))
            .to_string_lossy()
            .into_owned(),
    ];
    let hook_token_value = Arc::<str>::from(infrastructure::local_api::generate_token());
    let hook_token = infrastructure::local_api::BearerToken::from(hook_token_value.clone());
    let agent_sessions =
                adaptor::controller::agent_session_wiring::compose_agent_sessions(
                    adaptor::controller::agent_session_wiring::AgentSessionCompositionInput {
                        hook_token: hook_token_value,
                        retrying: retrying.clone(),
                        state_publisher: Some(state_subscriptions.clone()),
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
                        subscriptions: state_subscriptions.clone(),
                        launch_retention: adaptor::controller::agent_session_launch_retention::run(infrastructure::timer::delays(adaptor::controller::agent_session_launch_retention::RETENTION)),
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
    tokio::spawn(adaptor::controller::provider_session_title::run(
        retrying.clone(),
        provider_session_title_ingestion.clone(),
        Box::pin(infrastructure::timer::ticks(
            domain::agent_session::PROVIDER_SESSION_TITLE_TICK_INTERVAL,
        )),
    ));
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
        ).with_state_publisher(state_subscriptions.clone()),
    );

    use adaptor::controller::state::AppState;
    use adaptor::gateway::repository::repo_paths::RepoPathsGateway;
    use usecase::repo_paths_usecase::RepoPathsUsecase;

    let repo_paths_gateway =
        RepoPathsGateway::new(shared_repo_paths.clone(), config_repository.clone());

    let repo_paths_usecase = Arc::new(RepoPathsUsecase::new(
        Arc::new(repo_paths_gateway),
        state_subscriptions.clone(),
    ));

    repo_paths_usecase.initialize_from_cwd(&repository_usecase)?;

    let code_usecase = Arc::new(adaptor::controller::wiring::build_code_usecase());
    let git_host_usecase = Arc::new(
        adaptor::controller::wiring::build_git_host_usecase()
            .with_state_publisher(state_subscriptions.clone()),
    );
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
    let repository_scan_runtime =
        Arc::new(adaptor::controller::repository_scan::RepositoryScanWorkerRuntime::new());
    let repository_state = Arc::new(usecase::repository_state::RepositoryStateService::new(
        repository_state_repository,
        repository_scanner,
        state_subscriptions.clone(),
        Arc::new(
            adaptor::gateway::repository::state::NotifyRepositoryStateWatcher::new(
                repository_usecase.clone(),
            ),
        ),
        repository_scan_runtime.clone(),
        Arc::new(adaptor::gateway::repository::state::FsWorktreePathNormalizer),
        adaptor::controller::repository_scan::start(
            retrying.clone(),
            repository_scan_runtime.clone(),
            infrastructure::timer::delays(adaptor::controller::repository_scan::DEBOUNCE),
        ),
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
            failure_store.clone(),
            data_dir.clone(),
            repository_usecase.clone(),
            config_repository.clone(),
            local_event_store.clone(),
            node_processes.clone(),
        );
    let workflow_usecase = Arc::new(workflow_usecase);

    let notion_usecase = Arc::new(
        usecase::notion::usecase::NotionUsecase::new(
            notion_config_repository.clone(),
            app_config.clone(),
            notion_api_gateway.clone(),
        )
        .with_state_publisher(state_subscriptions.clone()),
    );

    let repository_state_for_watcher = repository_state.clone();
    let workspace_list = Arc::new(
        crate::adaptor::controller::wiring::build_workspace_list_usecase(
            repo_paths_usecase.clone(),
            repository_usecase.clone(),
            repository_state.clone(),
            workflow_usecase.clone(),
            git_host_usecase.clone(),
        ),
    );
    let app_state = AppState {
        workspace_list,
        repository_usecase: repository_usecase.clone(),
        repo_paths_usecase,
        code_usecase,
        review_usecase,
        notion_usecase,
        workflow_usecase: workflow_usecase.clone(),
        terminal_surface: terminal_surface.clone(),
        git_host_usecase,
    };
    let (workflow_runtime_usecase, workflow_startup) =
        adaptor::controller::wiring::build_workflow_runtime_usecase(
            retrying.clone(),
            adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies {
                store: Some(local_event_store.clone()),
                config: Some(config_repository.clone()),
                secrets: Some(config_secret_repository.clone()),
                state_changes: state_subscriptions.clone(),
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
            daemon_repository.clone(),
        )
        .map_err(|error| format!("workflow recovery admission failed: {error}"))?;
    let workflow_runtime_usecase = Arc::new(workflow_runtime_usecase);
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

    let review_comments_dir = adaptor::gateway::comment::state_dir(&data_dir);
    adaptor::controller::wiring::spawn_startup_app_data_gc(
        app_data.clone(),
        shared_repo_paths.clone(),
        projected_local_event_repository.clone(),
        workflow_runtime_usecase.clone(),
    );
    let info = daemon.info().await;
    let local_api_binding = infrastructure::local_api::LocalApiServerBinding::bind(
        data_dir.clone(),
        info.identity.daemon_id,
        info.identity.pid,
        info.identity.process_started_at,
        hook_token,
    )
    .map_err(|error| format!("local API の起動に失敗しました: {error}"))?;
    let mut client_dispatch =
        adaptor::controller::client::ClientCommandDispatch::new(daemon.clone())
            .with_state_publisher(state_subscriptions.clone());
    let reads_data_dir = data_dir.clone();
    let review_usecase_for_reads = app_state.review_usecase.clone();
    let review_comment_usecase_for_reads = review_comment_usecase.clone();
    let app_config_usecase = Arc::new(
        usecase::app_config::AppConfigUsecase::new(config_repository.clone(), app_config.clone())
            .with_state_publisher(state_subscriptions.clone()),
    );
    let hook_health_markers = data_dir
        .join("provider-launches")
        .to_string_lossy()
        .into_owned();
    let dependencies = super::client::ClientDependencies {
        installation_usecase: Some(installation),
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
        session_review_usecase: Some(session_review_usecase.clone()),
        config_repository: Some(config_repository.clone()),
        app_config_usecase: Some(app_config_usecase.clone()),
        workflow_runtime_usecase: Some(workflow_runtime_usecase.clone()),
        editor_launcher: Arc::new(adaptor::gateway::external_editor::NativeEditorLauncherGateway),
        watcher: Arc::new(usecase::watcher::WatcherUsecase::new(
            Some(repository_state_for_watcher),
            Arc::new(
                adaptor::gateway::repository::file_watcher::FileWatcherGateway::new(file_watchers),
            ),
        )),
        data_dir: Ok(data_dir),
        daemon: daemon.clone(),
        process_port: exit_sender,
    };
    let state_subscriptions = state_subscriptions.with_reads(
        Arc::new(
            adaptor::gateway::state_subscription_reads::StateSubscriptionReads(
                usecase::state_subscription::WorkspaceStateReads {
                    daemon: daemon.clone(),
                    repositories: dependencies
                        .app_state
                        .as_ref()
                        .unwrap()
                        .repo_paths_usecase
                        .clone(),
                    repository: repository_usecase.clone(),
                    workflow: workflow_usecase.clone(),
                    workspaces: dependencies
                        .app_state
                        .as_ref()
                        .unwrap()
                        .workspace_list
                        .clone(),
                    sessions: dependencies.agent_session_read_usecase.clone().unwrap(),
                    history: dependencies
                        .agent_session_history_read_usecase
                        .clone()
                        .unwrap(),
                    providers: dependencies.provider_availability_usecase.clone().unwrap(),
                    git_host: dependencies
                        .app_state
                        .as_ref()
                        .unwrap()
                        .git_host_usecase
                        .clone(),
                    workspace_state: dependencies.workspace_state_store.clone().unwrap(),
                    review: review_usecase_for_reads,
                    comments: review_comment_usecase_for_reads,
                    session_comments: session_review_usecase,
                    data_dir: reads_data_dir,
                    review_comments_dir,
                    workflows_dir: workflows_dir.clone(),
                    app_config: app_config_usecase,
                    notion: dependencies
                        .app_state
                        .as_ref()
                        .unwrap()
                        .notion_usecase
                        .clone(),
                    editor_settings: Arc::new(
                        adaptor::gateway::external_editor::EditorSettingsConfigGateway::new(
                            config_repository,
                        ),
                    ),
                    editor_scanner: Arc::new(
                        adaptor::gateway::external_editor::MacInstalledEditorGateway,
                    ),
                    hook_health: dependencies
                        .provider_hook_health_read_usecase
                        .clone()
                        .unwrap(),
                },
            ),
        ),
        Some(dependencies.watcher.clone()),
        history_paths,
        hook_health_markers,
    );
    client_dispatch.register_dependencies(&dependencies);
    let client_dispatch = Arc::new(client_dispatch);

    let priority = client_priority_interceptor();
    let local_api_router = adaptor::controller::api::build_router(
        adaptor::controller::api::auth::ClientTokens {
            operator: local_api_binding.client_bearer_token(),
            hook: local_api_binding.hook_bearer_token(),
        },
        Some(
            adaptor::controller::api::ClientApiDeps::new(client_dispatch.clone(), priority)
                .with_provider_lifecycle(provider_lifecycle_ingress.clone())
                .with_state_subscriptions(adaptor::controller::api::StateSubscriptionDeps::new(
                    state_subscriptions,
                    state_presenter,
                    terminal_subscriptions,
                )),
        ),
        default_timeout(),
    );
    let local_api =
        local_api_binding.start(local_api_router, &tokio::runtime::Handle::current())?;
    daemon.serve().await;
    local_api.publish_discovery()?;
    if let Some(startup) = workflow_startup {
        let retrying = retrying.clone();
        tokio::spawn(async move {
            if let Err(error) =
                adaptor::controller::workflow_startup::recover(&retrying, &startup).await
            {
                log::warn!("workflow startup advancement failed: {error}");
            }
        });
    }

    Ok(Daemon {
        server: local_api.clone(),
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
        daemon,
    })
}

pub async fn migrate_legacy_execution_archives(
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

#[cfg(test)]
#[path = "daemon_test.rs"]
mod daemon_tests;

#[cfg(feature = "test-support")]
impl Daemon {
    pub fn test_new(
        shutdown: Arc<dyn domain::application_lifecycle::ApplicationShutdownGateway>,
        server: Arc<infrastructure::local_api::LocalApiServer>,
        exit: tokio::sync::mpsc::Receiver<i32>,
        daemon: usecase::daemon::DaemonUsecase,
    ) -> Self {
        Self {
            shutdown,
            server,
            exit,
            daemon,
        }
    }
}
