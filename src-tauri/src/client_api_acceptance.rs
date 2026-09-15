use std::path::Path;
use std::sync::Arc;

use tauri::Manager;

use crate::adaptor::controller::api::{ClientApiDeps, TerminalApiDeps};
use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::adaptor::controller::command::client::client_endpoint;
use crate::adaptor::controller::command::CommandRouter;
use crate::adaptor::controller::state::TerminalStreamEndpoint;
use crate::adaptor::controller::terminal_surface_runtime::TerminalSurfaceRuntime;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::push::ClientPushGateway;
use crate::adaptor::protocol::client::ClientEndpoint;
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::infrastructure::push::PushSink;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub use crate::adaptor::gateway::push::{AgentSessionChangedPayload, BackendPush};
pub use crate::adaptor::gateway::repository::branch::BranchGateway;
pub use crate::adaptor::gateway::repository::watch::{FileChangeEvent, GitStatusChangedEvent};
pub use crate::adaptor::protocol::terminal::TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX;
pub use crate::adaptor::protocol::workflow::*;
pub use crate::domain::repository::{Branch, BranchRepository, RepositoryError};
use crate::domain::workflow::{ExecutionOrigin, RuntimeExecutionState, WorkflowRuntimeSnapshot};
pub use crate::infrastructure::comment::watcher::spawn_review_comments_watcher;
pub use crate::usecase::repository_state::snapshot::RepositorySnapshotChangedEvent;

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
        let repository = RepositoryUsecase::new(
            branch,
            Arc::new(repository::log::LogGateway),
            Arc::new(repository::status::StatusGateway),
            Arc::new(repository::worktree::WorktreeGateway),
            Arc::new(repository::git_config::GitConfigGateway),
            Arc::new(repository::util::RepoLocatorGateway),
            Arc::new(repository::worktree_terminal::NoopWorktreeTerminalGateway),
            crate::usecase::repository_query_service::RepositoryQueryService::new(Arc::new(
                repository::branch_card::BranchCardGateway,
            )),
        );
        let authority = Arc::new(ApplicationStartupAuthority::ready());
        let dispatch = Arc::new(ClientCommandDispatch::new(
            Arc::new(repository),
            authority.clone(),
        ));
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
            .manage(TerminalStreamEndpoint {
                port: binding.port(),
                token: binding.terminal_bearer_token(),
            })
            .invoke_handler(move |invoke| router.handle(invoke))
            .build(crate::application_context())
            .unwrap();
        drop(
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.to_path_buf()))
                .unwrap(),
        );
        let workflow = crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
            data_dir, None,
        )
        .unwrap();
        let runtime = WorkflowRuntimeUsecase::new(Arc::new(
            crate::provider_lifecycle_acceptance::AcceptanceWorkflowRuntimeGateway::default(),
        ));
        let terminal =
            TerminalSurfaceRuntime::new_with_data_dir(app.handle().clone(), data_dir.to_path_buf());
        let router = crate::adaptor::controller::api::build_router(
            Arc::new(workflow),
            Arc::new(runtime),
            binding.bearer_token(),
            binding.terminal_bearer_token(),
            Some(TerminalApiDeps::new(terminal.application())),
            Some(ClientApiDeps::new(dispatch, ClientPushGateway::new(sink))),
            None,
        );
        Self {
            app,
            server: binding.start(router, &tokio::runtime::Handle::current()),
            master_subprotocol,
        }
    }

    pub fn endpoint(&self) -> ClientEndpoint {
        client_endpoint(self.app.handle()).unwrap()
    }

    pub fn subscribe_push(&self) -> tokio::sync::broadcast::Receiver<Arc<[u8]>> {
        self.app.state::<Arc<PushSink>>().subscribe()
    }

    pub async fn emit_completed_workflow(
        &self,
        execution_id: &str,
        worktree_path: &str,
        updated_at: f64,
    ) {
        let state = WorkflowRuntimeSnapshot {
            execution_id: execution_id.into(),
            workflow_name: "review".into(),
            worktree_path: worktree_path.into(),
            created_from: ExecutionOrigin::Cli,
            request: "review".into(),
            error_reason: None,
            state: RuntimeExecutionState::Completed,
            current_node_name: None,
            current_session_id: None,
            node_history: vec![],
            workflow_definition: Default::default(),
            total_token_usage: Default::default(),
            artifacts: Default::default(),
            node_executions: vec![],
            started_at: 1.0,
            updated_at,
        };
        crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
            self.app.handle(),
            worktree_path,
            state,
        )
        .await;
    }
}

impl<R: tauri::Runtime> Drop for ClientApiAcceptanceHost<R> {
    fn drop(&mut self) {
        self.server.shutdown();
    }
}

pub use crate::adaptor::controller::api::protocol::client::{
    command_request, command_response, envelope, Ack, CommandRequest, Envelope, RequestAck,
};

pub fn encode_client_request(id: &str, name: &str, args: serde_json::Value) -> Vec<u8> {
    use prost::Message;
    let mut request = CommandRequest::from_value(name, args).expect("command arguments");
    request.request_id = id.into();
    Envelope {
        body: Some(envelope::Body::Request(Box::new(request))),
    }
    .encode_to_vec()
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
        let mut dispatch = ClientCommandDispatch::new(
            Arc::new(crate::adaptor::controller::wiring::build_repository_usecase_with_worktree_terminals(
                Arc::new(crate::adaptor::gateway::repository::worktree_terminal::NoopWorktreeTerminalGateway),
            )),
            Arc::new(ApplicationStartupAuthority::ready()),
        );
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
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            urls.push(format!("ws://{}/v1/client", listener.local_addr().unwrap()));
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
