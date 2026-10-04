use std::path::Path;
use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::controller::api::ClientApiDeps;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::command::CommandRouter;
use crate::adaptor::controller::terminal_surface_runtime::TerminalSurfaceRuntime;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::usecase::application_startup::ApplicationStartupAuthority;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub use crate::adaptor::gateway::repository::branch::BranchGateway;
pub use crate::adaptor::presenter::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;
pub use crate::adaptor::presenter::workflow_wire::*;
pub use crate::domain::repository::{Branch, BranchRepository, RepositoryError};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientEndpoint {
    pub url: String,
    pub token: String,
    pub launch_id: String,
}

pub fn desktop_connection_app<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
    data_dir: &Path,
    executable: &Path,
) -> tauri::App<R> {
    let mut router: CommandRouter<Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>> =
        CommandRouter::new(Box::new(|_| false));
    crate::adaptor::controller::command::client::register(&mut router);
    crate::adaptor::controller::command::desktop_lifecycle::register(&mut router);
    let status_presenter =
        Arc::new(crate::adaptor::presenter::daemon_status::DaemonStatusPresenter::new());
    let supervisor = crate::usecase::daemon_supervision::DaemonSupervisionUsecase::start(
        Arc::new(
            crate::adaptor::gateway::daemon_supervision::DaemonProcessGateway::new(
                executable.into(),
                data_dir.into(),
                Arc::new(crate::common::retry::RetryLimiter::new()),
            ),
        ),
        status_presenter.clone(),
    );
    builder
        .manage(Arc::new(ApplicationStartupAuthority::ready()))
        .manage(crate::usecase::client_connection::ClientConnectionUsecase(
            Box::new(supervisor.clone()),
        ))
        .manage(status_presenter)
        .manage(supervisor)
        .invoke_handler(move |invoke| router.handle(invoke))
        .build(crate::application_context())
        .unwrap()
}

pub fn desktop_supervision_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> serde_json::Value {
    serde_json::to_value(
        crate::adaptor::presenter::daemon_status::DaemonStatusMessage::from(
            app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
                .status(),
        ),
    )
    .unwrap()
}

pub fn stop_desktop_daemon<R: tauri::Runtime>(app: &tauri::AppHandle<R>, restart: bool) {
    use crate::domain::daemon_supervision::StopIntent;
    app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
        .stop(if restart {
            StopIntent::Restart
        } else {
            StopIntent::Quit(0)
        })
        .unwrap();
}

pub struct ClientApiAcceptanceHost<R: tauri::Runtime> {
    pub app: tauri::App<R>,
    server: Arc<LocalApiServer>,
    pub master_subprotocol: String,
}

impl<R: tauri::Runtime> ClientApiAcceptanceHost<R> {
    pub fn start(
        builder: tauri::Builder<R>,
        data_dir: &Path,
        branch: Arc<dyn BranchRepository>,
    ) -> Self {
        let work = crate::terminal_surface::initialize_background_work_for_acceptance();
        use crate::adaptor::gateway::repository;
        let operations = Arc::new(crate::usecase::worktree_operation::WorktreeOperations::new(
            Arc::new(repository::worktree_operation::FileWorktreeOperationLocks::new(data_dir)),
        ));
        let repository = RepositoryUsecase::new(
            branch,
            Arc::new(repository::status::StatusGateway),
            Arc::new(repository::worktree::WorktreeGateway),
            Arc::new(repository::git_config::GitConfigGateway),
            Arc::new(repository::util::RepoLocatorGateway),
            Arc::new(repository::worktree_terminal::NoopWorktreeTerminalGateway),
            operations.clone(),
        );
        let authority = Arc::new(ApplicationStartupAuthority::ready());
        let repository = Arc::new(repository);
        let state_presenter = Arc::new(
            crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new(),
        );
        let state = crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
            state_presenter.clone(),
            crate::adaptor::controller::state_subscription::drive(Arc::new(|| {
                let period = crate::domain::git_host::CacheTtl::EXTERNAL_INFORMATION.duration();
                Box::pin(crate::infrastructure::timer::ticks_after(period, period))
            })),
        )
        .with_reads(
            Arc::new(AcceptanceStateReads(repository.clone())),
            None,
            vec![],
            String::new(),
        );
        let mut dispatch = ClientCommandDispatch::new(authority.clone());
        dispatch.register_domain(
            &["get_language_from_path"],
            Box::new(move |command| {
                let repository = repository.clone();
                Box::pin(async move {
                    let crate::adaptor::presenter::client::command_request::Command::GetLanguageFromPath(
                        args,
                    ) = command
                    else {
                        unreachable!()
                    };
                    let path =
                        crate::adaptor::controller::client::required(args.file_path, "filePath")?;
                    let result =
                        crate::adaptor::controller::client::repository::run_blocking(move || {
                            repository.get_current_branch(&path)
                        })
                        .await;
                    crate::adaptor::controller::client::outcome(result).map(
                        crate::adaptor::presenter::client::command_result::Command::GetLanguageFromPath,
                    )
                })
            }),
        );
        let dispatch = Arc::new(dispatch);
        let router: CommandRouter<Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>> =
            CommandRouter::new(Box::new(|_| false));
        let binding = LocalApiServerBinding::bind(data_dir.to_path_buf()).unwrap();
        let master_subprotocol = format!(
            "{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}{}",
            binding.bearer_token()
        );
        let app = builder
            .manage(authority)
            .manage(dispatch.clone())
            .manage(crate::usecase::client_connection::ClientConnectionUsecase(
                Box::new(
                    crate::adaptor::gateway::local_api::ClientConnectionFileQuery(
                        data_dir.to_path_buf(),
                    ),
                ),
            ))
            .invoke_handler(move |invoke| router.handle(invoke))
            .build(crate::application_context())
            .unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            data_dir.to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .unwrap();
        let workflow = crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
            data_dir, None,
        )
        .unwrap();
        let runtime = WorkflowRuntimeUsecase::new_with_worktree_operations(
            work.retrying.clone(),
            Arc::new(
                crate::provider_lifecycle_acceptance::AcceptanceWorkflowRuntimeGateway::default(),
            ),
            Arc::new(
                crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::new(
                    store, data_dir,
                ),
            ),
            operations,
        );
        let terminal = TerminalSurfaceRuntime::new(work.clone(), data_dir.to_path_buf());
        let output = Arc::new(
            crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::new(
                &state_presenter,
            ),
        );
        terminal
            .application()
            .connect_state(output.clone())
            .unwrap();
        let terminal_subscriptions =
            crate::usecase::terminal_surface::subscription::TerminalSubscriptionUsecase::new(
                output,
                Some(terminal.application()),
                crate::adaptor::controller::terminal_subscription::start(),
            );

        let priority = crate::adaptor::controller::daemon::client_priority_interceptor();
        let local_gate = priority.gate.clone();
        let router = crate::adaptor::controller::api::build_router(
            Arc::new(workflow),
            Arc::new(runtime),
            binding.bearer_token(),
            binding.client_bearer_token(),
            Some(
                ClientApiDeps::new(dispatch, priority).with_state_subscriptions(
                    crate::adaptor::controller::api::StateSubscriptionDeps::new(
                        state,
                        state_presenter,
                        terminal_subscriptions,
                    ),
                ),
            ),
            None,
            (
                local_gate,
                crate::adaptor::controller::daemon::default_timeout(),
            ),
        );
        Self {
            app,
            server: binding.start(router, &tokio::runtime::Handle::current()),
            master_subprotocol,
        }
    }

    pub fn endpoint(&self) -> ClientEndpoint {
        let endpoint = self
            .app
            .state::<crate::usecase::client_connection::ClientConnectionUsecase>()
            .endpoint()
            .unwrap();
        ClientEndpoint {
            url: endpoint.url,
            token: endpoint.token,
            launch_id: String::new(),
        }
    }
}

impl<R: tauri::Runtime> Drop for ClientApiAcceptanceHost<R> {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

pub use crate::adaptor::presenter::connect_wire::rpc;
pub type NativeClient = rpc::ClientServiceClient<connectrpc::client::HttpClient>;

pub fn connect_client(endpoint: &ClientEndpoint) -> NativeClient {
    crate::adaptor::gateway::desktop_client::client(
        &crate::usecase::client_connection::ClientConnectionDto {
            url: endpoint.url.clone(),
            token: endpoint.token.clone(),
        },
    )
    .unwrap()
}

pub async fn request_client(
    client: &NativeClient,
    name: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, connectrpc::ConnectError> {
    use crate::adaptor::presenter::client as wire;
    let command = wire::CommandRequest::from_value(name, args)
        .unwrap()
        .command
        .unwrap();
    let result = crate::adaptor::gateway::desktop_client::call(client, command).await?;
    Ok(wire::CommandResult {
        command: Some(result),
    }
    .into_value()
    .unwrap()
    .1)
}

pub fn decode_client_value(
    value: impl crate::adaptor::presenter::client::ClientValue,
) -> serde_json::Value {
    crate::adaptor::presenter::client::from_value(value).unwrap()
}

#[derive(Default, Debug, PartialEq)]
pub struct ClientRecoveryState {
    pub crash_reporting: bool,
    pub mounted_xterms: u64,
    pub effects: Vec<String>,
}

pub struct ClientRecoveryAcceptanceHost {
    pub urls: Vec<String>,
    pub state: Arc<std::sync::Mutex<ClientRecoveryState>>,
    servers: Vec<tokio::task::JoinHandle<()>>,
}

impl ClientRecoveryAcceptanceHost {
    pub async fn start() -> Self {
        use crate::adaptor::presenter::client as wire;
        let state = Arc::new(std::sync::Mutex::new(ClientRecoveryState::default()));
        let mut dispatch =
            ClientCommandDispatch::new(Arc::new(ApplicationStartupAuthority::ready()));
        for names in [
            &["update_crash_reporting"][..],
            &["report_mounted_xterm_count"][..],
        ] {
            let target = state.clone();
            dispatch.register_domain(
                names,
                Box::new(move |command| {
                    let target = target.clone();
                    Box::pin(async move {
                        let mut state = target.lock().unwrap();
                        Ok(match command {
                            wire::command_request::Command::UpdateCrashReporting(args) => {
                                let enabled = args.enabled.expect("enabled");
                                state.crash_reporting = enabled;
                                state.effects.push(format!("crash:{enabled}"));
                                wire::command_result::Command::UpdateCrashReporting(wire::Unit {})
                            }
                            wire::command_request::Command::ReportMountedXtermCount(args) => {
                                let count = args.count.expect("count");
                                state.mounted_xterms = count;
                                state.effects.push(format!("xterms:{count}"));
                                wire::command_result::Command::ReportMountedXtermCount(
                                    wire::Unit {},
                                )
                            }
                            _ => unreachable!(),
                        })
                    })
                }),
            );
        }
        let dispatch = Arc::new(dispatch);
        let mut urls = Vec::new();
        let mut servers = Vec::new();
        for _ in 0..2 {
            let deps = ClientApiDeps::new(
                dispatch.clone(),
                crate::adaptor::controller::daemon::client_priority_interceptor(),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            urls.push(format!("http://{}", listener.local_addr().unwrap()));
            servers.push(tokio::spawn(async move {
                axum::serve(
                    listener,
                    crate::adaptor::controller::api::client::router(
                        Some(deps),
                        crate::adaptor::controller::daemon::default_timeout(),
                    ),
                )
                .await
                .unwrap();
            }));
        }
        Self {
            urls,
            state,
            servers,
        }
    }
}

impl Drop for ClientRecoveryAcceptanceHost {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

pub async fn initialize_desktop_settings<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Manager;
    let settings = app
        .state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
        .connection()
        .unwrap()
        .settings;
    crate::desktop::apply_desktop_settings(app, settings);
}

pub fn desktop_window_preferences<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    use tauri::Manager;
    let settings = app
        .state::<crate::infrastructure::platform::window_lifecycle::WindowPreferencesState>()
        .read();
    settings.close_to_tray
}

pub fn spawn_desktop_successor() -> Result<(), String> {
    crate::infrastructure::platform::desktop_restart::spawn_successor()
}

pub fn wait_for_desktop_predecessor() -> Result<bool, String> {
    crate::infrastructure::platform::desktop_restart::wait_for_predecessor()
}

pub async fn terminate_daemon_for_acceptance(
    executable: std::path::PathBuf,
    data_dir: std::path::PathBuf,
) -> Result<std::time::Duration, String> {
    use crate::domain::daemon_supervision::DaemonProcessPort;
    let gateway = crate::adaptor::gateway::daemon_supervision::DaemonProcessGateway::new(
        executable,
        data_dir.clone(),
        Arc::new(crate::common::retry::RetryLimiter::new()),
    );
    gateway.spawn().await?;
    let ready = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !data_dir.join("ready").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    let duplicate_rejected = gateway.spawn().await.is_err();
    let start = std::time::Instant::now();
    gateway.terminate_and_wait().await?;
    ready.map_err(|e| e.to_string())?;
    if !duplicate_rejected {
        return Err("A second daemon was spawned before the previous one exited".into());
    }
    Ok(start.elapsed())
}

pub async fn desktop_client_endpoint<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> ClientEndpoint {
    let supervisor =
        app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>();
    let connection = supervisor.attach().await.unwrap();
    ClientEndpoint {
        url: connection.endpoint.url,
        token: connection.endpoint.token,
        launch_id: connection.launch_id,
    }
}

#[cfg(feature = "performance")]
pub fn probe_cli_installation() -> Result<String, String> {
    crate::infrastructure::platform::cli_install::install_cli()
}

pub type DesktopUpdateAction = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub async fn apply_desktop_update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: DesktopUpdateAction,
) -> Result<(), String> {
    struct Installer(DesktopUpdateAction);
    #[async_trait::async_trait]
    impl crate::usecase::desktop_update::DesktopUpdateGateway for Installer {
        async fn check(
            &self,
        ) -> Result<Option<crate::usecase::desktop_update::UpdateInfo>, String> {
            Ok(None)
        }
    }
    #[async_trait::async_trait]
    impl crate::domain::daemon_supervision::DesktopUpdateInstaller for Installer {
        async fn download(&self) -> Result<(), String> {
            (self.0)("download")
        }
        async fn install(&self) -> Result<(), String> {
            (self.0)("install")
        }
        fn restart(&self) -> Result<(), String> {
            (self.0)("restart")
        }
    }
    crate::usecase::desktop_update::DesktopUpdateUsecase::new(
        Arc::new(Installer(action)),
        app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
            .inner()
            .clone(),
    )
    .apply()
    .await
    .map_err(|e| e.to_string())
}

struct AcceptanceStateReads(Arc<RepositoryUsecase>);
#[async_trait::async_trait]
impl crate::usecase::state_subscription::StateSubscriptionRead for AcceptanceStateReads {
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<
        crate::usecase::state_subscription::StateValue,
        crate::usecase::state_subscription::StateReadError,
    > {
        use crate::usecase::state_subscription::{StateReadError, StateValue};
        let crate::usecase::state_subscription::SubscriptionTarget::CurrentBranch(path) = target
        else {
            return Err(StateReadError {
                source: crate::usecase::state_subscription::StateReadFailure::Workflow(Box::new(
                    crate::domain::workflow::WorkflowError::NotFound("unknown target".into()),
                )),
                message: "unknown target".into(),
            });
        };
        let path = path.clone();
        let repository = self.0.clone();
        tokio::task::spawn_blocking(move || {
            repository
                .get_current_branch(&path)
                .map(StateValue::CurrentBranch)
                .map_err(|error| StateReadError {
                    message: error.to_string(),
                    source: error.into(),
                })
        })
        .await
        .unwrap()
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}

pub async fn read_current_branch(
    client: &NativeClient,
    args: serde_json::Value,
) -> Result<serde_json::Value, connectrpc::ConnectError> {
    let path = args["repoPath"].as_str().unwrap_or_default();
    let target = crate::usecase::state_subscription::SubscriptionTarget::from_parts(
        "current-branch",
        &[path],
    )
    .map_err(crate::adaptor::presenter::connect::classified_error)?;
    read_state(client, &target.to_string()).await
}

pub async fn read_state(
    client: &NativeClient,
    target: &str,
) -> Result<serde_json::Value, connectrpc::ConnectError> {
    let target = crate::usecase::state_subscription::SubscriptionTarget::parse(target)
        .map_err(crate::adaptor::presenter::connect::classified_error)?;
    let (name, args) = target.parts();
    let client_id = uuid::Uuid::new_v4().to_string();
    let mut stream = client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: client_id.clone(),
            ..Default::default()
        })
        .await?;
    stream.message::<rpc::StateSubscriptionEvent>().await?;
    client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id: client_id.clone(),
            subscription_id: client_id.clone(),
            target: name.into(),
            args: args.clone(),
            version: None.into(),
            ..Default::default()
        })
        .await?;
    let item = stream
        .message::<rpc::StateSubscriptionEvent>()
        .await?
        .unwrap()
        .to_owned_message();
    // 毎回の読み取りを最新にするため、snapshot を受け取ったら購読を止めて worker を解放する。
    client
        .stop_state_subscription(rpc::StopStateSubscriptionRequest {
            subscription_id: client_id,
            ..Default::default()
        })
        .await?;
    let payload = match item.event {
        Some(rpc::state_subscription_event::Event::Snapshot(payload)) => payload,
        Some(rpc::state_subscription_event::Event::Failure(failure)) => {
            return Err(connectrpc::ConnectError::new(
                connectrpc::ErrorCode::from_grpc_code(failure.code as u32)
                    .expect("failure event must have an error code"),
                failure.message,
            ));
        }
        _ => panic!("snapshot or failure"),
    };
    use crate::adaptor::presenter::{client as wire, connect_wire::to_wire};
    let payload: wire::StatePayload = to_wire(payload.as_ref())?;
    let value = match payload.value.unwrap() {
        wire::state_payload::Value::CurrentBranch(value) => {
            wire::from_message("releash.client.v1.ResultString", &value)
        }
        wire::state_payload::Value::AgentSession(value) => {
            wire::from_message("releash.client.v1.NullableAgentSessionItemDto", &value)
        }
        wire::state_payload::Value::Providers(value) => {
            wire::from_message("releash.client.v1.ListAgentSessionProviderDto", &value)
        }
        wire::state_payload::Value::SessionHistory(value) => {
            wire::from_message("releash.client.v1.AgentSessionHistoryPageDto", &value)
        }
        wire::state_payload::Value::Workspaces(value) => {
            wire::from_message("releash.client.v1.WorkspaceListSnapshot", &value)
        }
        wire::state_payload::Value::ProviderAvailability(value) => wire::from_message(
            "releash.client.v1.ProviderAvailabilitySnapshotResponse",
            &value,
        ),
        wire::state_payload::Value::ProviderHookHealth(value) => {
            wire::from_message("releash.client.v1.ProviderHookHealthSnapshot", &value)
        }
        wire::state_payload::Value::DesktopSettings(value) => {
            wire::from_message("releash.client.v1.DesktopSettings", &value)
        }
        _ => panic!("Unsupported acceptance state"),
    };
    Ok(value.unwrap())
}
