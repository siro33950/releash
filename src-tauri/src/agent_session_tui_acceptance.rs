use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;

use crate::adaptor::controller::agent_session_wiring::{
    compose_agent_sessions, AgentSessionCompositionInput,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::node_session_boundary::{
    ProviderWorkflowAgentSessionPort, WorkflowAgentSessionPort,
};
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
use crate::adaptor::gateway::workflow::WorkflowRuntimeCommandGateway;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::WorkflowDefinition;
use crate::infrastructure::local_api::LocalApiServer;
use crate::terminal_subscription_acceptance::TerminalSubscriptionHarness as TerminalSurfaceRuntime;
use crate::terminal_surface::TerminalSurfaceOwnerV1;
use crate::usecase::agent_session::{
    AgentSessionHistoryReadUsecase, AgentSessionInitialInstructionUsecase,
    AgentSessionLaunchUsecase, AgentSessionLifecycleUsecase, AgentSessionReadUsecase,
};
use crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase;
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct AcceptanceSearchPathSource(std::ffi::OsString);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl crate::infrastructure::process::search_path::SearchPathSource for AcceptanceSearchPathSource {
    fn load(
        &self,
    ) -> Result<std::ffi::OsString, crate::infrastructure::process::search_path::LoginShellPathError>
    {
        Ok(self.0.clone())
    }
}

#[derive(Debug, Clone)]
pub struct AgentSessionTuiAcceptanceConfig {
    pub data_dir: PathBuf,
    pub claude_executable: Option<PathBuf>,
    pub codex_executable: Option<PathBuf>,
    pub provider_search_path: Option<std::ffi::OsString>,
    pub provider_refresh_search_path: Option<std::ffi::OsString>,
    pub claude_config_dir: PathBuf,
    pub codex_home: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AcceptanceProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AcceptanceAgentSessionLifecycle {
    Open,
    Paused,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceAgentSessionTreeLocation {
    pub tree_id: String,
    pub node_execution_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceAgentSession {
    pub id: String,
    pub provider: AcceptanceProvider,
    pub lifecycle: AcceptanceAgentSessionLifecycle,
    pub tree_location: AcceptanceAgentSessionTreeLocation,
    pub provider_session_id: Option<String>,
    pub transcript_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceHistoryCandidate {
    pub provider: AcceptanceProvider,
    pub provider_session_id: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceHookWarning {
    pub provider: AcceptanceProvider,
    pub launch_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AcceptanceTerminalLaunchPerformanceSample {
    pub phase: String,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceArchiveOutcome {
    Archived,
    AlreadyArchived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceOpenOutcome {
    Attached,
    Resumed,
    Restored,
    Paused,
    Indeterminate,
    GarbageCollected,
}

struct AcceptanceUnusedWorkflowDefinitionResolver;

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for AcceptanceUnusedWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
        Err(WorkflowDefinitionResolverError::InvalidWorkflow(format!(
            "agent-session acceptance host does not start workflow '{workflow_name}'"
        )))
    }
}

struct AcceptanceManagedWorktreeResolver;

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AcceptanceManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}

pub struct AgentSessionTuiAcceptanceHost<R: tauri::Runtime> {
    _app: tauri::App<R>,
    client_api: Arc<LocalApiServer>,
    client_endpoint: crate::client_api_acceptance::ClientEndpoint,
    exit_observer: tauri::async_runtime::JoinHandle<()>,
    exit_observer_cancellation:
        Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceEventCancellation>,
    terminal: TerminalSurfaceRuntime,
    runtime: Arc<WorkflowRuntimeUsecase>,
    workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
    local_api: std::sync::Mutex<Arc<LocalApiServer>>,
    local_api_data_dir: PathBuf,
    provider_lifecycle_ingress:
        Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>,
    store: Arc<LocalEventStore>,
}

impl<R: tauri::Runtime> AgentSessionTuiAcceptanceHost<R> {
    pub fn start(
        config: AgentSessionTuiAcceptanceConfig,
        app: tauri::App<R>,
    ) -> Result<Self, String> {
        let queue = crate::terminal_surface::initialize_background_work_for_acceptance();
        app.manage(Arc::new(crate::infrastructure::push::PushSink::new()));
        std::fs::create_dir_all(&config.data_dir).map_err(|error| error.to_string())?;
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(config.data_dir.clone()))
                .map_err(|error| error.to_string())?;
        let terminal = TerminalSurfaceRuntime::new(queue.clone(), config.data_dir.clone());
        let data_dir = config.data_dir.clone();
        let subscriptions = terminal.subscriptions();
        let composition = compose_agent_sessions(AgentSessionCompositionInput {
            queue: queue.clone(),
            state_publisher: None,
            store: store.clone(),
            data_dir: data_dir.clone(),
            provider_executable_config: Arc::new(
                crate::adaptor::gateway::agent_session::InMemoryProviderExecutableConfigRepository::new(
                    config.claude_executable.as_ref().map(|path| path.to_string_lossy().into_owned()),
                    config.codex_executable.as_ref().map(|path| path.to_string_lossy().into_owned()),
                )
                .map_err(|error| format!("Provider executable Config初期化失敗: {error:?}"))?,
            ),
            provider_executable_probe: {
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                {
                    match config.provider_refresh_search_path {
                        Some(refresh_search_path) => Arc::new(
                            crate::adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::with_search_path_source(
                                config.provider_search_path,
                                Arc::new(AcceptanceSearchPathSource(refresh_search_path)),
                            ),
                        ) as Arc<dyn crate::domain::agent_session::ProviderExecutableProbeGateway>,
                        None => Arc::new(
                            crate::adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::with_search_path(
                                config.provider_search_path,
                            ),
                        ),
                    }
                }
                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                {
                    Arc::new(
                        crate::adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::with_search_path(
                            config.provider_search_path,
                        ),
                    )
                }
            },
            claude_config_dir: config.claude_config_dir,
            codex_home: config.codex_home,
            cli_binary: "releash-dev".to_string(),
            terminal: terminal.application(),
            change_notifier: Arc::new(
                crate::adaptor::gateway::push::ClientAgentSessionChangeNotifier::new(
                    subscriptions.publisher(),
                ),
            ),
        })
        .map_err(|error| format!("Provider availability初期化失敗: {error:?}"))?;
        let local_api_binding =
            crate::infrastructure::local_api::LocalApiServerBinding::bind(data_dir.clone())
                .map_err(|error| error.to_string())?;
        let provider_lifecycle_ingress: Arc<
            dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort,
        > = composition.lifecycle_ingress.clone();
        let local_api_router = crate::adaptor::controller::api::authenticated(
            crate::adaptor::controller::api::provider_lifecycle::router(Some(
                provider_lifecycle_ingress.clone(),
            )),
            local_api_binding.bearer_token(),
        );
        let local_api =
            local_api_binding.start(local_api_router, &tokio::runtime::Handle::current());
        let workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort> =
            Arc::new(ProviderWorkflowAgentSessionPort::new(
                composition.launch.clone(),
                composition.initial_instruction.clone(),
                composition.lifecycle.clone(),
                composition.availability_reader.clone(),
            ));
        app.manage(store.clone());
        let workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService> =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::with_repository(
                queue.clone(),
                crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(
                    store.clone(),
                ),
                Arc::new(
                    crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::new(
                        store.clone(),
                        data_dir.clone(),
                    ),
                ),
            );
        let mut driver = WorkflowRuntimeHost::new_canonical(
            queue.clone(),
            Arc::new(AcceptanceUnusedWorkflowDefinitionResolver),
            Arc::new(AcceptanceManagedWorktreeResolver),
            workspace_query,
            composition.launch.clone(),
            composition.initial_instruction.clone(),
            composition.lifecycle.clone(),
            composition.availability_reader.clone(),
            Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway),
        );
        let node_processes = Arc::new(
            crate::adaptor::gateway::workflow::node_process::WorkflowNodeProcesses::new(
                terminal.application(),
            ),
        );
        driver.node_processes = node_processes.clone();
        let driver = Arc::new(driver);
        let dependencies = crate::desktop_test_support::workflow_dependencies(app.handle());
        let startup = crate::adaptor::controller::wiring::wire_workflow_startup(
            queue.clone(),
            dependencies.clone(),
            driver.clone(),
        );
        let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            dependencies,
            driver,
        ));
        let operations = Arc::new(crate::usecase::worktree_operation::WorktreeOperations::new(Arc::new(
            crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks::new(&data_dir),
        )));
        let runtime = Arc::new(
            WorkflowRuntimeUsecase::new_with_worktree_operations(
                queue.clone(),
                gateway,
                Arc::new(
                    crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::new(
                        store.clone(),
                        data_dir.clone(),
                    ),
                ),
                operations.clone(),
            )
            .with_startup(startup),
        );
        composition.execution_tree_stops.bind(runtime.clone());
        composition
            .execution_tree_registrations
            .bind(runtime.clone());
        let terminal_events = terminal.application().subscribe_events();
        let exit_observer_cancellation = terminal_events.cancellation.clone();
        let exit_observer = tauri::async_runtime::spawn(
            crate::adaptor::controller::agent_session_exit_observer::run_agent_session_exit_observer(
                terminal_events,
                composition.exit.clone(),
            ),
        );
        app.manage(composition.history_read.clone());
        app.manage(composition.hook_health_read.clone());
        app.manage(composition.launch.clone());
        app.manage(composition.initial_instruction.clone());
        app.manage(composition.lifecycle.clone());
        app.manage(composition.read.clone());
        app.manage(composition.provider_availability.clone());
        let authority =
            Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready());
        app.manage(authority.clone());
        app.manage(Arc::new(
            crate::infrastructure::file_watcher::FileWatcherManager::default(),
        ));
        let mut dispatch =
            crate::adaptor::controller::client::ClientCommandDispatch::new(authority);
        dispatch.register_dependencies(&crate::desktop_test_support::build_client_dependencies(
            app.handle(),
        ));
        let dispatch = Arc::new(dispatch);
        let client_binding = crate::infrastructure::local_api::LocalApiServerBinding::bind(
            data_dir.join("desktop-client"),
        )
        .map_err(|error| error.to_string())?;
        let client_endpoint = crate::client_api_acceptance::ClientEndpoint {
            url: format!("http://127.0.0.1:{}", client_binding.port()),
            token: client_binding.terminal_bearer_token().to_string(),
            launch_id: String::new(),
        };
        let subscriptions = subscriptions.with_reads(
            Arc::new(AcceptanceSessionReads {
                sessions: composition.read.clone(),
                history: composition.history_read.clone(),
                providers: composition.provider_availability.clone(),
            }),
            None,
            vec![],
        );
        let client_router = crate::adaptor::controller::api::authenticated(
            crate::adaptor::controller::api::client::router(Some(
                crate::adaptor::controller::api::ClientApiDeps::new(
                    dispatch,
                    crate::adaptor::gateway::push::ClientPushGateway::new(
                        app.state::<Arc<crate::infrastructure::push::PushSink>>()
                            .inner()
                            .clone(),
                    ),
                    crate::desktop_test_support::build_watcher_usecase(app.handle()),
                )
                .with_state_subscriptions(subscriptions),
            )),
            client_binding.terminal_bearer_token(),
        );
        let client_api = client_binding.start(client_router, &tokio::runtime::Handle::current());
        Ok(Self {
            _app: app,
            client_api,
            client_endpoint,
            exit_observer,
            exit_observer_cancellation,
            terminal,
            runtime,
            workflow_agent_sessions,
            local_api: std::sync::Mutex::new(local_api),
            local_api_data_dir: data_dir,
            provider_lifecycle_ingress,
            store,
        })
    }

    pub fn client_endpoint(&self) -> &crate::client_api_acceptance::ClientEndpoint {
        &self.client_endpoint
    }

    pub fn terminal(&self) -> &TerminalSurfaceRuntime {
        &self.terminal
    }

    pub fn stop_local_api(&self) -> Result<(), String> {
        self.local_api
            .lock()
            .map_err(|_| "lock local API server".to_string())?
            .shutdown();
        Ok(())
    }

    pub fn restart_local_api(&self) -> Result<(), String> {
        let binding = crate::infrastructure::local_api::LocalApiServerBinding::bind(
            self.local_api_data_dir.clone(),
        )
        .map_err(|error| error.to_string())?;
        let router = crate::adaptor::controller::api::authenticated(
            crate::adaptor::controller::api::provider_lifecycle::router(Some(
                self.provider_lifecycle_ingress.clone(),
            )),
            binding.bearer_token(),
        );
        let server = binding.start(router, &tokio::runtime::Handle::current());
        *self
            .local_api
            .lock()
            .map_err(|_| "lock local API server".to_string())? = server;
        Ok(())
    }

    pub fn hook_health_marker_contents(&self) -> Result<Vec<String>, String> {
        crate::infrastructure::provider_lifecycle::read_provider_hook_local_api_failures(
            &self.local_api_data_dir,
            16,
        )
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|marker| String::from_utf8(marker.contents).map_err(|error| error.to_string()))
        .collect()
    }

    pub fn start_terminal_launch_performance_collection(&self) {
        crate::other::telemetry::start_terminal_launch_sample_collection();
    }

    pub fn take_terminal_launch_performance_samples(
        &self,
    ) -> Vec<AcceptanceTerminalLaunchPerformanceSample> {
        crate::other::telemetry::take_terminal_launch_samples()
            .into_iter()
            .map(|sample| AcceptanceTerminalLaunchPerformanceSample {
                phase: sample.phase.to_string(),
                duration_ms: sample.duration_ms,
            })
            .collect()
    }

    pub async fn launch_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
        workflow_execution_id: &str,
        node_execution_id: &str,
        initial_instruction: &str,
    ) -> Result<String, String> {
        let seeded_session_id = crate::domain::agent_session::launch_resource_id(
            "agent-session",
            &format!("workflow-node-launch-{node_execution_id}"),
        )
        .ok_or_else(|| "failed to derive acceptance AgentSession id".to_string())?;
        seed_workflow_session_facts(
            &self.store,
            WorkflowSessionFactSeed {
                workflow_name: "acceptance-workflow",
                request: "acceptance",
                worktree_path,
                provider: provider_kind(provider),
                workflow_execution_id,
                node_execution_id,
                session_id: &seeded_session_id,
                initial_instruction_admitted: true,
            },
        )
        .await?;
        let session = self
            .workflow_agent_sessions
            .prepare_workflow_agent_session(
                worktree_path,
                worktree_path,
                crate::adaptor::gateway::workflow::node_session_boundary::WorkflowSessionLaunchConfig {
                    provider: provider_kind(provider),
                    model: None,
                    permission: None,
                },
                workflow_execution_id,
                node_execution_id,
                initial_instruction,
            )
            .await
            .map_err(|error| format!("{error:?}"))?;
        if session.id != seeded_session_id {
            return Err("workflow AgentSession id differs from seeded attachment".to_string());
        }
        self.workflow_agent_sessions
            .activate_workflow_agent_session(&session.id, node_execution_id)
            .await
            .map_err(|error| format!("{error:?}"))?;
        Ok(session.id)
    }

    pub async fn resume_session_node(&self, node_execution_id: &str) -> Result<(), String> {
        self.runtime
            .resume_session_node_by_id(node_execution_id.to_string())
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn wait_until_exited(
        &self,
        workspace_identity: &str,
        agent_session_id: &str,
    ) -> Result<(), String> {
        let owner = TerminalSurfaceOwnerV1::Session {
            workspace_path: workspace_identity.to_string(),
            session_id: agent_session_id.to_string(),
        };
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self
                    .terminal
                    .get(owner.clone())
                    .is_ok_and(|surface| surface.is_exited)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "timed out waiting for Provider process exit".to_string())
    }

    #[allow(deprecated)]
    pub async fn shutdown(self) -> Result<(), String> {
        let Self {
            _app: app,
            client_api,
            client_endpoint: _,
            exit_observer,
            exit_observer_cancellation,
            terminal,
            runtime: _runtime,
            workflow_agent_sessions,
            local_api,
            local_api_data_dir: _,
            provider_lifecycle_ingress,
            store,
        } = self;
        exit_observer_cancellation.cancel();
        exit_observer
            .await
            .map_err(|error| format!("join AgentSession exit observer: {error}"))?;
        terminal.shutdown()?;
        client_api
            .shutdown_and_wait()
            .await
            .map_err(|error| error.to_string())?;
        let local_api = local_api
            .into_inner()
            .map_err(|_| "lock local API server".to_string())?;
        local_api
            .shutdown_and_wait()
            .await
            .map_err(|error| format!("join local API server: {error}"))?;
        let launch = app
            .try_state::<Arc<AgentSessionLaunchUsecase>>()
            .map(|state| state.inner().clone());
        if let Some(launch) = launch {
            launch
                .wait_for_background_tasks()
                .await
                .map_err(|error| format!("join AgentSession background task: {error}"))?;
        }
        app.unmanage::<Arc<AgentSessionHistoryReadUsecase>>();
        app.unmanage::<Arc<ProviderHookHealthReadUsecase>>();
        app.unmanage::<Arc<AgentSessionLaunchUsecase>>();
        app.unmanage::<Arc<AgentSessionInitialInstructionUsecase>>();
        app.unmanage::<Arc<AgentSessionLifecycleUsecase>>();
        app.unmanage::<Arc<crate::adaptor::controller::client::ClientCommandDispatch>>();
        app.unmanage::<Arc<AgentSessionReadUsecase>>();
        app.unmanage::<Arc<crate::usecase::agent_session::ProviderAvailabilityUsecase>>();
        app.unmanage::<Arc<LocalEventStore>>();
        drop((
            local_api,
            _runtime,
            workflow_agent_sessions,
            provider_lifecycle_ingress,
            terminal,
            client_api,
            app,
        ));
        drain_and_close_store(store).await
    }
}

async fn drain_and_close_store(mut store: Arc<LocalEventStore>) -> Result<(), String> {
    let store = tokio::time::timeout(Duration::from_secs(10), async move {
        loop {
            match Arc::try_unwrap(store) {
                Ok(store) => return store,
                Err(shared) => store = shared,
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        "timed out waiting for local event store references during shutdown".to_string()
    })?;
    store.drain_and_close();
    Ok(())
}

fn provider_kind(provider: AcceptanceProvider) -> ProviderKind {
    match provider {
        AcceptanceProvider::Claude => ProviderKind::Claude,
        AcceptanceProvider::Codex => ProviderKind::Codex,
    }
}

#[cfg(test)]
#[path = "agent_session_tui_acceptance_test.rs"]
mod agent_session_tui_acceptance_tests;

struct AcceptanceSessionReads {
    sessions: Arc<AgentSessionReadUsecase>,
    history: Arc<AgentSessionHistoryReadUsecase>,
    providers: Arc<crate::usecase::agent_session::ProviderAvailabilityUsecase>,
}
#[async_trait::async_trait]
impl crate::usecase::state_subscription::StateSubscriptionRead for AcceptanceSessionReads {
    async fn read(
        &self,
        target: &crate::domain::state_subscription::SubscriptionTarget,
    ) -> Result<
        crate::usecase::state_subscription::StateValue,
        crate::usecase::state_subscription::StateReadError,
    > {
        use crate::domain::failure::ClassifiedFailure;
        use crate::domain::state_subscription::SubscriptionTarget as T;
        use crate::usecase::state_subscription::{StateReadError, StateValue};
        match target {
            T::AgentSession(id) => self
                .sessions
                .get(id)
                .await
                .map(StateValue::AgentSession)
                .map_err(|e| StateReadError {
                    kind: e.failure_kind(),
                    message: format!("{e:?}"),
                }),
            T::SessionHistory(path, count) => self
                .history
                .list(crate::usecase::agent_session::AgentSessionHistoryRequest {
                    worktree_path: path.clone(),
                    visible_count: *count,
                })
                .await
                .map(StateValue::SessionHistory)
                .map_err(|e| StateReadError {
                    kind: e.failure_kind(),
                    message: format!("{e:?}"),
                }),
            T::Providers => self
                .providers
                .available_providers()
                .map(|providers| {
                    StateValue::Providers(
                        providers
                            .into_iter()
                            .map(|provider| match provider {
                                ProviderKind::Claude => {
                                    crate::usecase::agent_session::AgentSessionProviderDto::Claude
                                }
                                ProviderKind::Codex => {
                                    crate::usecase::agent_session::AgentSessionProviderDto::Codex
                                }
                            })
                            .collect(),
                    )
                })
                .map_err(|e| StateReadError {
                    kind: e.failure_kind(),
                    message: format!("{e:?}"),
                }),
            _ => Err(StateReadError {
                kind: crate::domain::failure::FailureKind::Missing,
                message: "Unsupported acceptance state".into(),
            }),
        }
    }
    async fn refresh_workspaces(
        &self,
        _: Option<crate::domain::state_subscription::StateChangeSource>,
    ) {
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}
