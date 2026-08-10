use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::adaptor::controller::provider_agent_session_wiring::{
    compose_provider_agent_sessions, ProviderAgentSessionCompositionInput,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
use crate::adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGateway;
use crate::domain::agent_session::aggregates::AgentSessionLifecycle;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{
    FacetRefs, FanoutSpec, NodeDefinition, NodeKind, Rule, SchemaDef, SessionGate, SessionSpec,
    WorkflowDefinition,
};
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::terminal_surface::TerminalSurfaceRuntime;
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::context::InstructionSourcePort;
use crate::usecase::agent_session::runtime::ports::{
    AgentSessionEventNotifier, AgentSessionStateChangedPayload, AgentStallObservedPayload,
    AgentStreamingDeltaPayload,
};
use crate::usecase::agent_session::session::{
    ChatMessage, ChatSession, ContextCarryState, GetSessionResponse, ModelInfo, SessionStore,
    TokenUsage,
};
use crate::usecase::agent_session::status::{
    AgentStatusCenter, AgentStatusChanges, AgentStatusNotifier, SessionNotice,
};
use crate::usecase::agent_session::ProviderAgentSessionUsecase;
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;
use crate::usecase::workspace_tree::WorkspaceQueryService;

pub use crate::agent_session_tui_acceptance::{
    AcceptanceAgentSessionLifecycle, AcceptanceProvider, AgentSessionTuiAcceptanceConfig,
};

const AUTO_CLAUDE_WORKFLOW: &str = "acceptance-auto-claude";
const AUTO_CODEX_WORKFLOW: &str = "acceptance-auto-codex";
const AUTO_CHAIN_CLAUDE_WORKFLOW: &str = "acceptance-auto-chain-claude";
const AUTO_CHAIN_CODEX_WORKFLOW: &str = "acceptance-auto-chain-codex";
const APPROVAL_CLAUDE_WORKFLOW: &str = "acceptance-approval-claude";
const APPROVAL_CODEX_WORKFLOW: &str = "acceptance-approval-codex";
const APPROVAL_FANOUT_CLAUDE_WORKFLOW: &str = "acceptance-approval-fanout-claude";
const APPROVAL_FANOUT_CODEX_WORKFLOW: &str = "acceptance-approval-fanout-codex";
const ARTIFACT_CLAUDE_WORKFLOW: &str = "acceptance-artifact-claude";
const ARTIFACT_CODEX_WORKFLOW: &str = "acceptance-artifact-codex";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceWorkflowExecutionStatus {
    Running,
    WaitingApproval,
    Completed,
    Aborted,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceNodeExecutionStatus {
    Running,
    Paused,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceNodeExecution {
    pub id: String,
    pub node_name: String,
    pub attempt: u32,
    pub status: AcceptanceNodeExecutionStatus,
    pub agent_session_id: Option<String>,
    pub submit_received: bool,
    pub stop_received: bool,
    pub can_approve: bool,
    pub can_retry: bool,
    pub has_artifact: bool,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceWorkflowExecution {
    pub id: String,
    pub status: AcceptanceWorkflowExecutionStatus,
    pub node_executions: Vec<AcceptanceNodeExecution>,
}

#[derive(Deserialize)]
struct StartExecutionResponse {
    execution_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionResponse {
    id: String,
    status: ExecutionStatusResponse,
    node_executions: Vec<NodeExecutionResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ExecutionStatusResponse {
    Running,
    WaitingApproval,
    Completed,
    Aborted,
    Interrupted,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeExecutionResponse {
    id: String,
    node_name: String,
    attempt: u32,
    status: NodeExecutionStatusResponse,
    session_id: Option<String>,
    submit_received: bool,
    stop_received: bool,
    can_approve: bool,
    can_retry: bool,
    has_artifact: bool,
    failure: Option<NodeExecutionFailureResponse>,
}

#[derive(Deserialize)]
struct NodeExecutionFailureResponse {
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NodeExecutionStatusResponse {
    Running,
    Paused,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

#[derive(Deserialize)]
struct MutationResponse {
    ok: bool,
}

struct AcceptanceWorkflowDefinitionResolver;

fn acceptance_session_node(
    name: &str,
    provider: ProviderKind,
    gate: SessionGate,
    rules: Vec<Rule>,
) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Session(SessionSpec {
            provider,
            gate,
            facets: FacetRefs {
                instruction: Some("policy-confirmation".to_string()),
                ..FacetRefs::default()
            },
        }),
        artifact: None,
        input: None,
        inputs: Vec::new(),
        rules,
    }
}

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for AcceptanceWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
        let fanout_provider = match workflow_name {
            APPROVAL_FANOUT_CLAUDE_WORKFLOW => Some(ProviderKind::Claude),
            APPROVAL_FANOUT_CODEX_WORKFLOW => Some(ProviderKind::Codex),
            _ => None,
        };
        if let Some(provider) = fanout_provider {
            return Ok(WorkflowDefinition {
                name: workflow_name.to_string(),
                description: "Workflow control-plane product acceptance".to_string(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![
                    NodeDefinition {
                        name: "fanout".to_string(),
                        kind: NodeKind::Fanout(FanoutSpec {
                            child: vec!["review-a".to_string(), "review-b".to_string()],
                            items: None,
                        }),
                        artifact: None,
                        input: None,
                        inputs: Vec::new(),
                        rules: Vec::new(),
                    },
                    acceptance_session_node(
                        "review-a",
                        provider,
                        SessionGate::Approval,
                        Vec::new(),
                    ),
                    acceptance_session_node(
                        "review-b",
                        provider,
                        SessionGate::Approval,
                        Vec::new(),
                    ),
                ],
            });
        }
        let artifact_provider = match workflow_name {
            ARTIFACT_CLAUDE_WORKFLOW => Some(ProviderKind::Claude),
            ARTIFACT_CODEX_WORKFLOW => Some(ProviderKind::Codex),
            _ => None,
        };
        if let Some(provider) = artifact_provider {
            let mut node =
                acceptance_session_node("agent", provider, SessionGate::Auto, Vec::new());
            node.artifact = Some("acceptance-result".to_string());
            return Ok(WorkflowDefinition {
                name: workflow_name.to_string(),
                description: "Workflow control-plane product acceptance".to_string(),
                builtin: false,
                schemas: [(
                    "acceptance-result".to_string(),
                    SchemaDef::Object {
                        properties: [("result".to_string(), SchemaDef::String { r#enum: None })]
                            .into_iter()
                            .collect(),
                        required: ["result".to_string()].into_iter().collect(),
                    },
                )]
                .into_iter()
                .collect(),
                nodes: vec![node],
            });
        }
        let (provider, gate, chained) = match workflow_name {
            AUTO_CLAUDE_WORKFLOW => (ProviderKind::Claude, SessionGate::Auto, false),
            AUTO_CODEX_WORKFLOW => (ProviderKind::Codex, SessionGate::Auto, false),
            AUTO_CHAIN_CLAUDE_WORKFLOW => (ProviderKind::Claude, SessionGate::Auto, true),
            AUTO_CHAIN_CODEX_WORKFLOW => (ProviderKind::Codex, SessionGate::Auto, true),
            APPROVAL_CLAUDE_WORKFLOW => (ProviderKind::Claude, SessionGate::Approval, false),
            APPROVAL_CODEX_WORKFLOW => (ProviderKind::Codex, SessionGate::Approval, false),
            _ => {
                return Err(WorkflowDefinitionResolverError::InvalidWorkflow(format!(
                    "unknown acceptance workflow '{workflow_name}'"
                )))
            }
        };
        let nodes = if chained {
            vec![
                acceptance_session_node(
                    "agent-first",
                    provider,
                    gate,
                    vec![Rule::Next("agent-second".to_string())],
                ),
                acceptance_session_node("agent-second", provider, gate, Vec::new()),
            ]
        } else {
            vec![acceptance_session_node("agent", provider, gate, Vec::new())]
        };
        Ok(WorkflowDefinition {
            name: workflow_name.to_string(),
            description: "Workflow control-plane product acceptance".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes,
        })
    }
}

struct AcceptanceManagedWorktreeResolver;

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AcceptanceManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}

struct AcceptanceInstructionSource;

impl InstructionSourcePort for AcceptanceInstructionSource {
    fn read_instruction_file(
        &self,
        _path: &std::path::Path,
        _worktree_root: &std::path::Path,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
}

struct AcceptanceAgentStatusNotifier;

impl AgentStatusNotifier for AcceptanceAgentStatusNotifier {
    fn status_changed(&self, _changes: AgentStatusChanges) {}
}

struct AcceptanceAgentSessionEventNotifier;

impl AgentSessionEventNotifier for AcceptanceAgentSessionEventNotifier {
    fn persist_notice(&self, _notice: SessionNotice) {}

    fn display_window_updated(&self, _response: &GetSessionResponse) -> bool {
        true
    }

    fn session_state_changed(&self, _payload: AgentSessionStateChangedPayload) {}

    fn stall_observed(&self, _payload: AgentStallObservedPayload) {}

    fn stall_cleared(&self, _session_id: &str) {}

    fn streaming_delta(&self, _payload: AgentStreamingDeltaPayload) -> bool {
        true
    }

    fn supported_commands_updated(
        &self,
        _session_id: &str,
        _commands: Vec<crate::domain::agent_session::value_objects::SlashCommand>,
    ) {
    }

    fn token_usage_updated(&self, _session_id: &str, _token_usage: TokenUsage) {}

    fn permission_mode_changed(&self, _session_id: &str, _permission_mode: &str) {}

    fn models_updated(
        &self,
        _session_id: &str,
        _available_models: Vec<ModelInfo>,
        _selected_model: String,
    ) {
    }

    fn context_carry_updated(
        &self,
        _session_id: &str,
        _agent_session_id: Option<String>,
        _context_carry: Option<ContextCarryState>,
        _updated_at: f64,
    ) {
    }

    fn pending_message_consumed(
        &self,
        _session_id: &str,
        _queued_turn_id: Option<String>,
        _human_message: Option<ChatMessage>,
        _agent_message: ChatMessage,
    ) {
    }

    fn turn_prepared(
        &self,
        _session: &ChatSession,
        _human_message: &ChatMessage,
        _agent_message: &ChatMessage,
    ) {
    }
}

pub struct WorkflowControlPlaneAcceptanceHost<R: tauri::Runtime> {
    _app: tauri::App<R>,
    writer_lock_path: std::path::PathBuf,
    terminal: TerminalSurfaceRuntime,
    exit_observer: tauri::async_runtime::JoinHandle<()>,
    exit_observer_cancellation:
        Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceEventCancellation>,
    provider_sessions: Arc<ProviderAgentSessionUsecase>,
    _runtime: Arc<WorkflowRuntimeUsecase>,
    local_api: Arc<LocalApiServer>,
    local_api_base_url: String,
    local_api_token: String,
}

impl<R: tauri::Runtime> WorkflowControlPlaneAcceptanceHost<R> {
    pub fn start(
        config: AgentSessionTuiAcceptanceConfig,
        app: tauri::App<R>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(&config.data_dir).map_err(|error| error.to_string())?;
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(config.data_dir.clone()))
                .map_err(|error| error.to_string())?;
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let installation_id = store.installation_id().to_string();
        app.manage(store.clone());
        app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            config.data_dir.clone(),
        ));

        let terminal = TerminalSurfaceRuntime::new_with_data_dir(
            app.handle().clone(),
            config.data_dir.clone(),
        );
        let composition = compose_provider_agent_sessions(ProviderAgentSessionCompositionInput {
			repository: repository.clone(),
			installation_id: installation_id.clone(),
			data_dir: config.data_dir.clone(),
			claude_executable: config.claude_executable.to_string_lossy().into_owned(),
			codex_executable: config.codex_executable.to_string_lossy().into_owned(),
			claude_config_dir: config.claude_config_dir,
			codex_home: config.codex_home,
			cli_binary: "releash-dev".to_string(),
			terminal: terminal.application(),
			change_notifier: Arc::new(
				crate::adaptor::presenter::provider_agent_session_changed::TauriProviderAgentSessionChangeNotifier::new(
					app.handle().clone(),
				),
			),
		});
        terminal.bind_agent_session_activity(composition.activity.clone());

        let session_store = Arc::new(SessionStore::new_canonical(
			repository.clone(),
			installation_id.clone(),
			Arc::new(
				crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
			),
		));
        let archives = Arc::new(
            crate::adaptor::gateway::workflow::WorkflowExecutionArchiveFileRepository::new(
                config.data_dir.clone(),
            ),
        );
        let workspace_query: Arc<dyn WorkspaceQueryService> =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::new(
                store.clone(),
                archives,
            );
        let agent_status_center = Arc::new(AgentStatusCenter::new());
        app.manage(agent_status_center.clone());
        let agent_runtime = crate::compose_agent_session_runtime(
            session_store.clone(),
            Arc::new(AgentBackendRegistry::new()),
            agent_status_center,
            Arc::new(AcceptanceAgentStatusNotifier),
            Arc::new(AcceptanceAgentSessionEventNotifier),
            Arc::new(crate::adaptor::gateway::agent_session::TokioAgentTaskSpawner),
            None,
            Arc::new(AcceptanceInstructionSource),
            config.data_dir.clone(),
            workspace_query,
        );
        let driver = Arc::new(WorkflowRuntimeHost::new_canonical(
            Arc::new(AcceptanceWorkflowDefinitionResolver),
            Arc::new(AcceptanceManagedWorktreeResolver),
            Some(config.data_dir.clone()),
            repository.clone(),
            installation_id.clone(),
            composition.launch.clone(),
            composition.initial_instruction.clone(),
            composition.availability_gateway.clone(),
        ));
        let gateway = Arc::new(TauriWorkflowRuntimeCommandGateway::new_with_driver(
            app.handle().clone(),
            driver,
            session_store,
            agent_runtime,
            repository,
            installation_id,
        ));
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(gateway));
        composition.workflow_stops.bind(runtime.clone());

        let workflows_dir = config.data_dir.join("acceptance-workflows");
        std::fs::create_dir_all(&workflows_dir).map_err(|error| error.to_string())?;
        let workflow_read = Arc::new(
            crate::adaptor::controller::wiring::build_canonical_workflow_read_usecase(
                config.data_dir.clone(),
                Some(workflows_dir),
            )
            .map_err(|error| error.to_string())?,
        );
        let binding = LocalApiServerBinding::bind(config.data_dir.clone())
            .map_err(|error| error.to_string())?;
        let port = binding.port();
        let token = binding.bearer_token();
        let router = crate::adaptor::controller::api::build_router(
            workflow_read,
            runtime.clone(),
            token.clone(),
            binding.terminal_bearer_token(),
            None,
            None,
            Some(composition.lifecycle_ingress.clone()),
        );
        let local_api = binding.start(router, &tokio::runtime::Handle::current());

        let terminal_events = terminal.application().subscribe_events();
        let exit_observer_cancellation = terminal_events.cancellation.clone();
        let exit_observer = tauri::async_runtime::spawn(
			crate::adaptor::controller::provider_agent_session_exit_observer::run_provider_agent_session_exit_observer(
				terminal_events,
				composition.exit.clone(),
			),
		);

        Ok(Self {
            _app: app,
            writer_lock_path: config.data_dir.join("local-event-store.lock"),
            terminal,
            exit_observer,
            exit_observer_cancellation,
            provider_sessions: composition.sessions,
            _runtime: runtime,
            local_api,
            local_api_base_url: format!("http://127.0.0.1:{port}"),
            local_api_token: token.to_string(),
        })
    }

    pub fn terminal(&self) -> &TerminalSurfaceRuntime {
        &self.terminal
    }

    pub async fn start_auto_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
    ) -> Result<String, String> {
        let workflow_name = match provider {
            AcceptanceProvider::Claude => AUTO_CLAUDE_WORKFLOW,
            AcceptanceProvider::Codex => AUTO_CODEX_WORKFLOW,
        };
        self.start_named_workflow(worktree_path, workflow_name)
            .await
    }

    pub async fn start_auto_chain_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
    ) -> Result<String, String> {
        let workflow_name = match provider {
            AcceptanceProvider::Claude => AUTO_CHAIN_CLAUDE_WORKFLOW,
            AcceptanceProvider::Codex => AUTO_CHAIN_CODEX_WORKFLOW,
        };
        self.start_named_workflow(worktree_path, workflow_name)
            .await
    }

    pub async fn start_approval_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
    ) -> Result<String, String> {
        let workflow_name = match provider {
            AcceptanceProvider::Claude => APPROVAL_CLAUDE_WORKFLOW,
            AcceptanceProvider::Codex => APPROVAL_CODEX_WORKFLOW,
        };
        self.start_named_workflow(worktree_path, workflow_name)
            .await
    }

    pub async fn start_approval_fanout_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
    ) -> Result<String, String> {
        let workflow_name = match provider {
            AcceptanceProvider::Claude => APPROVAL_FANOUT_CLAUDE_WORKFLOW,
            AcceptanceProvider::Codex => APPROVAL_FANOUT_CODEX_WORKFLOW,
        };
        self.start_named_workflow(worktree_path, workflow_name)
            .await
    }

    pub async fn start_artifact_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
    ) -> Result<String, String> {
        let workflow_name = match provider {
            AcceptanceProvider::Claude => ARTIFACT_CLAUDE_WORKFLOW,
            AcceptanceProvider::Codex => ARTIFACT_CODEX_WORKFLOW,
        };
        self.start_named_workflow(worktree_path, workflow_name)
            .await
    }

    async fn start_named_workflow(
        &self,
        worktree_path: &str,
        workflow_name: &str,
    ) -> Result<String, String> {
        let response: StartExecutionResponse = self
            .post(
                "/v1/workflow/executions",
                &serde_json::json!({
                    "workflow_name": workflow_name,
                    "worktree_path": worktree_path,
                    "request": "acceptance initial instruction",
                    "permission_mode": "ask",
                    "created_from": "api"
                }),
            )
            .await?;
        Ok(response.execution_id)
    }

    pub async fn execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<AcceptanceWorkflowExecution>, String> {
        let response = self
            .get::<ExecutionResponse>(&format!("/v1/workflow/executions/{execution_id}"))
            .await;
        match response {
            Ok(response) => Ok(Some(response.into())),
            Err(error) if error.starts_with("HTTP 404:") => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn recover_startup(&self) -> Result<(), String> {
        self._runtime
            .recover_startup_excluding(&std::collections::BTreeSet::new())
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn submit(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: &str,
    ) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/submit"),
                &serde_json::json!({
                    "node": node_name,
                    "node_execution_id": node_execution_id,
                }),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Submit response was not successful".to_string())
    }

    pub async fn submit_artifact(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: &str,
        contract: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/submit"),
                &serde_json::json!({
                    "node": node_name,
                    "node_execution_id": node_execution_id,
                    "artifact": {
                        "contract": contract,
                        "value": value,
                    },
                }),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Artifact Submit response was not successful".to_string())
    }

    pub async fn approve(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: &str,
    ) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/approve"),
                &serde_json::json!({
                    "node": node_name,
                    "node_execution_id": node_execution_id,
                    "comment": null,
                }),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Approval response was not successful".to_string())
    }

    pub async fn retry(&self, execution_id: &str, node_execution_id: &str) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/retry"),
                &serde_json::json!({
                    "node_execution_id": node_execution_id,
                }),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Retry response was not successful".to_string())
    }

    pub async fn agent_session_lifecycle(
        &self,
        agent_session_id: &str,
    ) -> Result<Option<AcceptanceAgentSessionLifecycle>, String> {
        let session = self
            .provider_sessions
            .find(agent_session_id)
            .await
            .map_err(|error| format!("{error:?}"))?;
        Ok(session.map(|session| match session.session().lifecycle() {
            AgentSessionLifecycle::Open => AcceptanceAgentSessionLifecycle::Open,
            AgentSessionLifecycle::Paused => AcceptanceAgentSessionLifecycle::Paused,
            AgentSessionLifecycle::Archived => AcceptanceAgentSessionLifecycle::Archived,
        }))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        self.request(reqwest::Method::GET, path, None).await
    }

    async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, String> {
        let body = serde_json::to_value(body).map_err(|error| error.to_string())?;
        self.request(reqwest::Method::POST, path, Some(body)).await
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(|error| error.to_string())?;
        let mut request = client
            .request(method, format!("{}{path}", self.local_api_base_url))
            .bearer_auth(&self.local_api_token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| error.to_string())?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&bytes)
            ));
        }
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    }

    #[allow(deprecated)]
    pub async fn shutdown(self) -> Result<(), String> {
        let Self {
            _app,
            writer_lock_path,
            terminal,
            exit_observer,
            exit_observer_cancellation,
            provider_sessions,
            _runtime,
            local_api,
            local_api_base_url: _,
            local_api_token: _,
        } = self;
        exit_observer_cancellation.cancel();
        exit_observer
            .await
            .map_err(|error| format!("join Provider AgentSession exit observer: {error}"))?;
        terminal.shutdown()?;
        local_api.shutdown();
        _app.unmanage::<Arc<LocalEventStore>>();
        _app.unmanage::<Arc<AgentStatusCenter>>();
        drop((local_api, _runtime, provider_sessions, terminal, _app));
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let writer_lock = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&writer_lock_path)
                    .map_err(|error| error.to_string())?;
                if fs2::FileExt::try_lock_exclusive(&writer_lock).is_ok() {
                    fs2::FileExt::unlock(&writer_lock).map_err(|error| error.to_string())?;
                    return Ok::<(), String>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .map_err(|_| "timed out waiting for Local Event Store shutdown".to_string())??;
        Ok(())
    }
}

impl From<ExecutionResponse> for AcceptanceWorkflowExecution {
    fn from(value: ExecutionResponse) -> Self {
        Self {
            id: value.id,
            status: match value.status {
                ExecutionStatusResponse::Running => AcceptanceWorkflowExecutionStatus::Running,
                ExecutionStatusResponse::WaitingApproval => {
                    AcceptanceWorkflowExecutionStatus::WaitingApproval
                }
                ExecutionStatusResponse::Completed => AcceptanceWorkflowExecutionStatus::Completed,
                ExecutionStatusResponse::Aborted => AcceptanceWorkflowExecutionStatus::Aborted,
                ExecutionStatusResponse::Interrupted => {
                    AcceptanceWorkflowExecutionStatus::Interrupted
                }
            },
            node_executions: value
                .node_executions
                .into_iter()
                .map(AcceptanceNodeExecution::from)
                .collect(),
        }
    }
}

impl From<NodeExecutionResponse> for AcceptanceNodeExecution {
    fn from(value: NodeExecutionResponse) -> Self {
        Self {
            id: value.id,
            node_name: value.node_name,
            attempt: value.attempt,
            status: match value.status {
                NodeExecutionStatusResponse::Running => AcceptanceNodeExecutionStatus::Running,
                NodeExecutionStatusResponse::Paused => AcceptanceNodeExecutionStatus::Paused,
                NodeExecutionStatusResponse::WaitingApproval => {
                    AcceptanceNodeExecutionStatus::WaitingApproval
                }
                NodeExecutionStatusResponse::Succeeded => AcceptanceNodeExecutionStatus::Succeeded,
                NodeExecutionStatusResponse::Failed => AcceptanceNodeExecutionStatus::Failed,
                NodeExecutionStatusResponse::Aborted => AcceptanceNodeExecutionStatus::Aborted,
            },
            agent_session_id: value.session_id,
            submit_received: value.submit_received,
            stop_received: value.stop_received,
            can_approve: value.can_approve,
            can_retry: value.can_retry,
            has_artifact: value.has_artifact,
            failure_reason: value.failure.map(|failure| failure.reason),
        }
    }
}
