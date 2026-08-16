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
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::infrastructure::local_api::LocalApiServer;
use crate::terminal_surface::{TerminalSurfaceOwnerV1, TerminalSurfaceRuntime};
use crate::usecase::agent_session::{
    AgentSessionHistoryReadUsecase, AgentSessionInitialInstructionUsecase,
    AgentSessionLaunchUsecase, AgentSessionLifecycleUsecase, AgentSessionReadUsecase,
};
use crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase;

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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcceptanceAgentSessionOrigin {
    Standalone,
    WorkflowNode {
        workflow_execution_id: String,
        node_execution_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptanceAgentSession {
    pub id: String,
    pub provider: AcceptanceProvider,
    pub lifecycle: AcceptanceAgentSessionLifecycle,
    pub origin: AcceptanceAgentSessionOrigin,
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
    DeleteConfirmationRequired,
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

pub struct AgentSessionTuiAcceptanceHost<R: tauri::Runtime> {
    _app: tauri::App<R>,
    window: tauri::WebviewWindow<R>,
    exit_observer: tauri::async_runtime::JoinHandle<()>,
    exit_observer_cancellation:
        Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceEventCancellation>,
    terminal: TerminalSurfaceRuntime,
    workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
    local_api: std::sync::Mutex<Arc<LocalApiServer>>,
    local_api_data_dir: PathBuf,
    provider_lifecycle_ingress:
        Arc<dyn crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort>,
}

impl<R: tauri::Runtime> AgentSessionTuiAcceptanceHost<R> {
    pub fn start(
        config: AgentSessionTuiAcceptanceConfig,
        app: tauri::App<R>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&config.data_dir).map_err(|error| error.to_string())?;
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(config.data_dir.clone()))
                .map_err(|error| error.to_string())?;
        let terminal = TerminalSurfaceRuntime::new_with_data_dir(
            app.handle().clone(),
            config.data_dir.clone(),
        );
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let data_dir = config.data_dir.clone();
        let composition = compose_agent_sessions(AgentSessionCompositionInput {
            repository,
            installation_id: store.installation_id().to_string(),
            data_dir: config.data_dir,
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
                crate::adaptor::presenter::agent_session_changed::TauriAgentSessionChangeNotifier::new(
                    app.handle().clone(),
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
                composition.interrupt.clone(),
                composition.availability_reader.clone(),
            ));
        terminal.bind_agent_session_activity(composition.activity.clone());
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
        let window = tauri::WebviewWindowBuilder::new(
            &app,
            "agent-session-product-driver",
            Default::default(),
        )
        .build()
        .map_err(|error| error.to_string())?;
        Ok(Self {
            _app: app,
            window,
            exit_observer,
            exit_observer_cancellation,
            terminal,
            workflow_agent_sessions,
            local_api: std::sync::Mutex::new(local_api),
            local_api_data_dir: data_dir,
            provider_lifecycle_ingress,
        })
    }

    pub fn window(&self) -> &tauri::WebviewWindow<R> {
        &self.window
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
        let session = self
            .workflow_agent_sessions
            .prepare_workflow_agent_session(
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
        self.workflow_agent_sessions
            .activate_workflow_agent_session(&session.id, node_execution_id)
            .await
            .map_err(|error| format!("{error:?}"))?;
        Ok(session.id)
    }

    pub async fn dispatch_initial_instruction(
        &self,
        agent_session_id: &str,
        node_execution_id: &str,
        instruction: &str,
    ) -> Result<(), String> {
        self.workflow_agent_sessions
            .dispatch_initial_instruction(agent_session_id, node_execution_id, instruction)
            .await
            .map_err(|error| format!("{error:?}"))
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
            _app,
            window,
            exit_observer,
            exit_observer_cancellation,
            terminal,
            workflow_agent_sessions,
            local_api,
            local_api_data_dir: _,
            provider_lifecycle_ingress: _,
        } = self;
        exit_observer_cancellation.cancel();
        exit_observer
            .await
            .map_err(|error| format!("join AgentSession exit observer: {error}"))?;
        terminal.shutdown()?;
        local_api
            .into_inner()
            .map_err(|_| "lock local API server".to_string())?
            .shutdown();
        _app.unmanage::<Arc<AgentSessionHistoryReadUsecase>>();
        _app.unmanage::<Arc<ProviderHookHealthReadUsecase>>();
        _app.unmanage::<Arc<AgentSessionLaunchUsecase>>();
        _app.unmanage::<Arc<AgentSessionInitialInstructionUsecase>>();
        _app.unmanage::<Arc<AgentSessionLifecycleUsecase>>();
        _app.unmanage::<Arc<AgentSessionReadUsecase>>();
        _app.unmanage::<Arc<crate::usecase::agent_session::ProviderAvailabilityUsecase>>();
        drop((workflow_agent_sessions, window, _app));
        Ok(())
    }
}

pub fn product_agent_session_invoke_handler<R: tauri::Runtime>(
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    crate::adaptor::controller::command::agent_session::agent_session_invoke_handler()
}

fn provider_kind(provider: AcceptanceProvider) -> ProviderKind {
    match provider {
        AcceptanceProvider::Claude => ProviderKind::Claude,
        AcceptanceProvider::Codex => ProviderKind::Codex,
    }
}
