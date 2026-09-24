use std::path::Path;
use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::controller::api::{ClientApiDeps, TerminalApiDeps};
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::command::CommandRouter;
use crate::adaptor::controller::terminal_surface_runtime::TerminalSurfaceRuntime;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::push::ClientPushGateway;
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::infrastructure::push::PushSink;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub use crate::adaptor::gateway::push::BackendPush;
pub use crate::adaptor::gateway::repository::branch::BranchGateway;
pub use crate::adaptor::gateway::repository::watch::{FileChangeEvent, GitStatusChangedEvent};
pub use crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;
pub use crate::adaptor::protocol::workflow::*;
pub use crate::domain::repository::{Branch, BranchRepository, RepositoryError};
pub use crate::infrastructure::comment::watcher::spawn_review_comments_watcher;

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
    let supervisor = crate::usecase::daemon_supervision::DaemonSupervisionUsecase::start(Arc::new(
        crate::adaptor::gateway::daemon_supervision::DaemonProcessGateway::new(
            executable.into(),
            data_dir.into(),
        ),
    ));
    builder
        .manage(Arc::new(ApplicationStartupAuthority::ready()))
        .manage(crate::usecase::client_connection::ClientConnectionUsecase(
            Box::new(supervisor.clone()),
        ))
        .manage(supervisor)
        .invoke_handler(move |invoke| router.handle(invoke))
        .build(crate::application_context())
        .unwrap()
}

pub fn desktop_supervision_status<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> serde_json::Value {
    serde_json::to_value(
        app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>()
            .status(),
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
            crate::usecase::repository_query_service::RepositoryQueryService::new(
                Arc::new(repository::branch_card::BranchCardGateway),
                operations.clone(),
            ),
        );
        let authority = Arc::new(ApplicationStartupAuthority::ready());
        let repository = Arc::new(repository);
        let state = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            vec![],
            Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
        )
        .with_reads(
            Arc::new(AcceptanceStateReads(repository.clone())),
            None,
            vec![],
        );
        let mut dispatch = ClientCommandDispatch::new(authority.clone());
        dispatch.register_domain(
            &["get_releash_base"],
            Box::new(move |command| {
                let repository = repository.clone();
                Box::pin(async move {
                    let crate::adaptor::protocol::client::command_request::Command::GetReleashBase(
                        args,
                    ) = command
                    else {
                        unreachable!()
                    };
                    let path =
                        crate::adaptor::controller::client::required(args.repo_path, "repoPath")?;
                    let result =
                        crate::adaptor::controller::client::repository::run_blocking(move || {
                            repository.get_current_branch(&path)
                        })
                        .await;
                    crate::adaptor::controller::client::outcome(result.map(Some)).map(
                        crate::adaptor::protocol::client::command_result::Command::GetReleashBase,
                    )
                })
            }),
        );
        let dispatch = Arc::new(dispatch);
        let router: CommandRouter<Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>> =
            CommandRouter::new(Box::new(|_| false));
        let sink = Arc::new(PushSink::new());
        let binding = LocalApiServerBinding::bind(data_dir.to_path_buf()).unwrap();
        let master_subprotocol = format!(
            "{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}{}",
            binding.bearer_token()
        );
        let app = builder
            .manage(sink.clone())
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
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .unwrap();
        let workflow = crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
            data_dir, None,
        )
        .unwrap();
        let runtime = WorkflowRuntimeUsecase::new_with_worktree_operations(
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
        let terminal = TerminalSurfaceRuntime::new(data_dir.to_path_buf());
        let router = crate::adaptor::controller::api::build_router(
            Arc::new(workflow),
            Arc::new(runtime),
            binding.bearer_token(),
            binding.client_bearer_token(),
            Some(TerminalApiDeps::new(terminal.application())),
            Some(
                ClientApiDeps::new(
                    dispatch,
                    ClientPushGateway::new(sink),
                    crate::client_api_acceptance::watcher(),
                )
                .with_state_subscriptions(state),
            ),
            None,
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

    pub fn review_comment_notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let sink = crate::desktop_test_support::push_sink(self.app.handle());
        Arc::new(move || BackendPush::ReviewCommentsChanged("*").emit(&sink))
    }

    pub fn emit(&self, push: BackendPush<'_>) {
        push.emit(&crate::desktop_test_support::push_sink(self.app.handle()));
    }

    pub fn push_subscription_count(&self) -> usize {
        self.app.state::<Arc<PushSink>>().subscriber_count()
    }

    pub fn subscribe_push(&self) -> tokio::sync::broadcast::Receiver<Arc<[u8]>> {
        self.app.state::<Arc<PushSink>>().subscribe()
    }
}

impl<R: tauri::Runtime> Drop for ClientApiAcceptanceHost<R> {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

pub use crate::adaptor::protocol::connect::rpc;
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
    use crate::adaptor::protocol::client as wire;
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

pub fn decode_rpc_push(push: rpc::Push) -> (&'static str, serde_json::Value) {
    decode_client_push(crate::adaptor::controller::api::protocol::connect::to_wire(&push).unwrap())
}

pub fn decode_client_value(
    value: impl crate::adaptor::controller::api::protocol::client::ClientValue,
) -> serde_json::Value {
    crate::adaptor::controller::api::protocol::client::from_value(value).unwrap()
}

pub fn decode_client_push(
    push: crate::adaptor::controller::api::protocol::client::Push,
) -> (&'static str, serde_json::Value) {
    push.into_value().unwrap()
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
        use crate::adaptor::controller::api::protocol::client as wire;
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
                ClientPushGateway::new(Arc::new(PushSink::new())),
                crate::client_api_acceptance::watcher(),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            urls.push(format!("http://{}", listener.local_addr().unwrap()));
            servers.push(tokio::spawn(async move {
                axum::serve(
                    listener,
                    crate::adaptor::controller::api::client::router(Some(deps)),
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
        .state::<crate::usecase::client_connection::ClientConnectionUsecase>()
        .desktop_settings()
        .await
        .unwrap();
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
    id: String,
) -> ClientEndpoint {
    let supervisor =
        app.state::<Arc<crate::usecase::daemon_supervision::DaemonSupervisionUsecase>>();
    let connection = supervisor.attach(id).await.unwrap();
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

pub(crate) fn watcher() -> Arc<crate::usecase::watcher::WatcherUsecase> {
    Arc::new(crate::usecase::watcher::WatcherUsecase::new(
        None,
        Arc::new(
            crate::adaptor::gateway::repository::file_watcher::FileWatcherGateway::new(
                Arc::new(crate::infrastructure::file_watcher::FileWatcherManager::default()),
                Arc::new(PushSink::new()),
            ),
        ),
    ))
}

struct AcceptanceStateReads(Arc<RepositoryUsecase>);
#[async_trait::async_trait]
impl crate::usecase::state_subscription::StateSubscriptionRead for AcceptanceStateReads {
    async fn read(
        &self,
        target: &crate::domain::state_subscription::SubscriptionTarget,
    ) -> Result<
        crate::usecase::state_subscription::StateValue,
        crate::usecase::state_subscription::StateReadError,
    > {
        use crate::domain::failure::{ClassifiedFailure, FailureKind};
        use crate::usecase::state_subscription::{StateReadError, StateValue};
        let crate::domain::state_subscription::SubscriptionTarget::CurrentBranch(path) = target
        else {
            return Err(StateReadError {
                kind: FailureKind::Missing,
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
                    kind: error.failure_kind(),
                    message: error.to_string(),
                })
        })
        .await
        .unwrap()
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

pub async fn read_current_branch(
    client: &NativeClient,
    args: serde_json::Value,
) -> Result<serde_json::Value, connectrpc::ConnectError> {
    let path = args["repoPath"].as_str().unwrap_or_default();
    let target = crate::domain::state_subscription::SubscriptionTarget::from_parts(
        "current-branch",
        &[path],
    )
    .map_err(crate::adaptor::protocol::connect::classified_error)?;
    read_state(client, &target.to_string()).await
}

pub async fn read_state(
    client: &NativeClient,
    target: &str,
) -> Result<serde_json::Value, connectrpc::ConnectError> {
    let target = crate::domain::state_subscription::SubscriptionTarget::parse(target)
        .map_err(crate::adaptor::protocol::connect::classified_error)?;
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
            client_id,
            target: name.into(),
            args,
            version: None.into(),
            ..Default::default()
        })
        .await?;
    let item = stream
        .message::<rpc::StateSubscriptionEvent>()
        .await?
        .unwrap()
        .to_owned_message();
    let Some(rpc::state_subscription_event::Event::Snapshot(payload)) = item.event else {
        panic!("snapshot")
    };
    use crate::adaptor::protocol::{client as wire, connect::to_wire};
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
            wire::from_message("releash.client.v1.WorkspaceListSnapshotDto", &value)
        }
        _ => panic!("Unsupported acceptance state"),
    };
    Ok(value.unwrap())
}
