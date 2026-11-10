use crate::adaptor::presenter::client::{CommandRequestEncode, CommandResultDecode};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::adaptor::controller::api::ClientApiDeps;
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::terminal_surface_runtime::TerminalSurfaceRuntime;
use crate::infrastructure::local_api::LocalApiServer;

pub use crate::adaptor::gateway::repository::git::GitGateway;
pub use crate::adaptor::presenter::workflow_wire::{
    DiagnosticReport, DiagnosticSpan, DiagnosticStage, Severity,
};
pub use crate::domain::git::{Branch, GitError, GitService};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientEndpoint {
    pub url: String,
    pub token: String,
}

pub struct ClientApiAcceptanceHost {
    data_dir: PathBuf,
    server: Arc<LocalApiServer>,
}

impl ClientApiAcceptanceHost {
    pub fn start(data_dir: &Path, branch: Arc<dyn GitService>) -> Self {
        let work = crate::terminal_surface::initialize_background_work_for_acceptance();
        let config_path = data_dir.join("releash.toml");
        let config =
            crate::adaptor::gateway::app_config::load_or_create_config(&config_path).unwrap();
        let app_config = Arc::new(crate::adaptor::gateway::app_config::AppConfig::new(
            config,
            config_path,
        ));
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
            Arc::new(AcceptanceStateReads(branch.clone())),
            None,
            vec![],
            String::new(),
        );
        let mut dispatch = ClientCommandDispatch::new(crate::usecase::daemon::DaemonUsecase::new(
            crate::adaptor::gateway::daemon::serving(),
        ));
        dispatch.register_domain(
            &["add_repo_path"],
            Box::new(move |command| {
                let branch = branch.clone();
                Box::pin(async move {
                    let crate::adaptor::presenter::client::command_request::Command::AddRepoPath(
                        args,
                    ) = command
                    else {
                        unreachable!()
                    };
                    let path = crate::adaptor::controller::client::required(args.path, "path")?;
                    let result =
                        crate::adaptor::controller::client::repository::run_blocking(move || {
                            branch
                                .current(&path)
                                .map(|_| true)
                                .map_err(crate::usecase::repository_error::UsecaseError::from)
                        })
                        .await;
                    crate::adaptor::controller::client::outcome(result).map(
                        crate::adaptor::presenter::client::command_result::Command::AddRepoPath,
                    )
                })
            }),
        );
        let dispatch = Arc::new(dispatch);
        let binding =
            crate::infrastructure::local_api::test_binding(data_dir.to_path_buf()).unwrap();
        let terminal =
            TerminalSurfaceRuntime::new(work.clone(), data_dir.to_path_buf(), app_config);
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
        let router = crate::adaptor::controller::api::build_router(
            crate::adaptor::controller::api::auth::ClientTokens {
                operator: binding.client_bearer_token(),
                hook: binding.hook_bearer_token(),
            },
            Some(
                ClientApiDeps::new(dispatch, priority).with_state_subscriptions(
                    crate::adaptor::controller::api::StateSubscriptionDeps::new(
                        state,
                        state_presenter,
                        terminal_subscriptions,
                    ),
                ),
            ),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        Self {
            data_dir: data_dir.to_path_buf(),
            server: binding
                .start(router, &tokio::runtime::Handle::current())
                .inspect(|server| {
                    server.publish_discovery().unwrap();
                })
                .unwrap(),
        }
    }

    pub fn endpoint(&self) -> ClientEndpoint {
        let discovery: serde_json::Value =
            serde_json::from_slice(&std::fs::read(self.data_dir.join("client-api.json")).unwrap())
                .unwrap();
        ClientEndpoint {
            url: format!("http://127.0.0.1:{}", discovery["port"]),
            token: discovery["token"].as_str().unwrap().to_owned(),
        }
    }
}

impl Drop for ClientApiAcceptanceHost {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

pub use crate::adaptor::presenter::connect_wire::rpc;
pub type NativeClient = rpc::ClientServiceClient<connectrpc::client::HttpClient>;

pub fn connect_client(endpoint: &ClientEndpoint) -> NativeClient {
    use crate::adaptor::presenter::client::descriptor;
    use connectrpc::client::{ClientConfig, HttpClient};
    let options = descriptor::pool()
        .get_service_by_name("releash.client.v1.ClientService")
        .unwrap()
        .options();
    let timeout = descriptor::option(&options, "default_timeout_ms")
        .as_u32()
        .unwrap();
    rpc::ClientServiceClient::new(
        HttpClient::plaintext_http2_only(),
        ClientConfig::new(endpoint.url.parse().unwrap())
            .with_default_header("authorization", format!("Bearer {}", endpoint.token))
            .with_default_timeout(std::time::Duration::from_millis(timeout.into())),
    )
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
    let result = call(client, command).await?;
    Ok(wire::CommandResult {
        command: Some(result),
    }
    .into_value()
    .unwrap()
    .1)
}

struct AcceptanceStateReads(Arc<dyn GitService>);
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
        let crate::usecase::state_subscription::SubscriptionTarget::ReleashBase(path) = target
        else {
            return Err(StateReadError {
                source: crate::usecase::state_subscription::StateReadFailure::Workflow(Box::new(
                    crate::domain::workflow::WorkflowError::NotFound("unknown target".into()),
                )),
                message: "unknown target".into(),
            });
        };
        let path = path.clone();
        let branch = self.0.clone();
        tokio::task::spawn_blocking(move || {
            branch
                .current(&path)
                .map(|name| StateValue::ReleashBase(Some(name)))
                .map_err(|error| {
                    let error = crate::usecase::repository_error::UsecaseError::from(error);
                    StateReadError {
                        message: error.to_string(),
                        source: error.into(),
                    }
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
    let target =
        crate::usecase::state_subscription::SubscriptionTarget::from_parts("releash-base", &[path])
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
        wire::state_payload::Value::WorkflowExecution(value) => {
            wire::from_message("releash.client.v1.NullableWorkflowExecutionView", &value)
        }
        wire::state_payload::Value::ReleashBase(value) => {
            wire::from_message("releash.client.v1.Nullablestring", &value)
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
        wire::state_payload::Value::SessionTab(value) => {
            wire::from_message("releash.client.v1.SessionTabSnapshot", &value)
        }
        _ => panic!("Unsupported acceptance state"),
    };
    Ok(value.unwrap())
}

use crate::adaptor::presenter::client_calls::call;
