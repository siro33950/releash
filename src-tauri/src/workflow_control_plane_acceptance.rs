use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::adaptor::controller::agent_session_wiring::{
    compose_agent_sessions, AgentSessionCompositionInput,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
use crate::adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGateway;
use crate::domain::agent_session::aggregates::AgentSessionLifecycle;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleScope};
use crate::domain::workflow::{
    ChildEntry, FacetRefs, FanoutSpec, NodeCompletion, NodeCompletionSignalState, NodeDefinition,
    NodeExecutionStatus, NodeKind, NodeKindName, RepositoryWorktreeInventory,
    RuntimeExecutionState, SchemaDef, SequenceSpec, SessionSpec, WorkflowDefinition, WorkflowError,
    WorkflowRuntimeSnapshot, WorktreeInventoryGateway,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::domain::workspace_tree::{WorkspaceNodeStatusClassification, WorkspaceTreeRepository};
use crate::infrastructure::local_api::{LocalApiServer, LocalApiServerBinding};
use crate::terminal_surface::TerminalSurfaceRuntime;
use crate::usecase::agent_session::{
    AgentSessionLaunchRequest, AgentSessionLaunchUsecase, AgentSessionLifecycleUsecase,
    AgentSessionUsecase,
};
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::usecase::workflow::{
    WorkflowRuntimeUsecase, WorkspaceNodeActionResolver, WorkspaceNodeApprovalTarget,
    WorkspaceNodeCommandUsecase, WorkspaceNodeRetryTarget, WorkspaceSessionNodeRenameTarget,
};
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
const DEFAULT_CAP_FANOUT_CLAUDE_WORKFLOW: &str = "acceptance-default-cap-fanout-claude";
const DEFAULT_CAP_FANOUT_CHILDREN: usize = 33;
const ARTIFACT_CLAUDE_WORKFLOW: &str = "acceptance-artifact-claude";
const ARTIFACT_CODEX_WORKFLOW: &str = "acceptance-artifact-codex";
const APPROVAL_ARTIFACT_CLAUDE_WORKFLOW: &str = "acceptance-approval-artifact-claude";
const APPROVAL_ARTIFACT_CODEX_WORKFLOW: &str = "acceptance-approval-artifact-codex";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceNodeKind {
    Command,
    Session,
    Fanout,
    Sequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceWorkspaceNodeStatus {
    Active,
    Attention,
    Failure,
    Idle,
    Unbound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceNodeExecution {
    pub id: String,
    pub node_name: String,
    pub kind: AcceptanceNodeKind,
    pub attempt: u32,
    pub status: AcceptanceNodeExecutionStatus,
    pub agent_session_id: Option<String>,
    pub submit_received: bool,
    pub stop_received: bool,
    pub can_approve: bool,
    pub can_retry: bool,
    pub has_artifact: bool,
    pub artifact: Option<serde_json::Value>,
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
#[serde(rename_all = "snake_case")]
enum NodeKindResponse {
    Command,
    Session,
    Fanout,
    Sequence,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeExecutionResponse {
    id: String,
    node_name: String,
    kind: NodeKindResponse,
    attempt: u32,
    status: NodeExecutionStatusResponse,
    session_id: Option<String>,
    submit_received: bool,
    stop_received: bool,
    can_approve: bool,
    can_retry: bool,
    has_artifact: bool,
    artifact: Option<ArtifactResponse>,
    failure: Option<NodeExecutionFailureResponse>,
}

#[derive(Deserialize)]
struct ArtifactResponse {
    value: serde_json::Value,
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
    completion: NodeCompletion,
) -> NodeDefinition {
    NodeDefinition {
        name: name.to_string(),
        kind: NodeKind::Session(SessionSpec {
            provider,
            model: None,
            permission: None,
            facets: FacetRefs {
                instruction: Some("policy-confirmation".to_string()),
                ..FacetRefs::default()
            },
        }),
        artifact: None,
        input: Vec::new(),
        completion,
        worktree: None,
    }
}

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for AcceptanceWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
        if workflow_name == DEFAULT_CAP_FANOUT_CLAUDE_WORKFLOW {
            let child_names = (0..DEFAULT_CAP_FANOUT_CHILDREN)
                .map(|index| format!("review-{index:02}"))
                .collect::<Vec<_>>();
            let mut nodes = vec![NodeDefinition {
                name: "fanout".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: child_names.iter().map(ChildEntry::reference).collect(),
                    items: None,
                }),
                artifact: None,
                input: Vec::new(),
                completion: NodeCompletion::Auto,
                worktree: None,
            }];
            nodes.extend(child_names.iter().map(|name| {
                acceptance_session_node(name, ProviderKind::Claude, NodeCompletion::Approval)
            }));
            return Ok(WorkflowDefinition {
                name: workflow_name.to_string(),
                description: "Workflow control-plane product acceptance".to_string(),
                builtin: false,
                schemas: Default::default(),
                nodes,
                entry: "fanout".to_string(),
            });
        }
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
                            children: vec![
                                ChildEntry::reference("review-a"),
                                ChildEntry::reference("review-b"),
                            ],
                            items: None,
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: NodeCompletion::Auto,
                        worktree: None,
                    },
                    acceptance_session_node("review-a", provider, NodeCompletion::Approval),
                    acceptance_session_node("review-b", provider, NodeCompletion::Approval),
                ],
                entry: "fanout".to_string(),
            });
        }
        let artifact_provider = match workflow_name {
            ARTIFACT_CLAUDE_WORKFLOW | APPROVAL_ARTIFACT_CLAUDE_WORKFLOW => {
                Some(ProviderKind::Claude)
            }
            ARTIFACT_CODEX_WORKFLOW | APPROVAL_ARTIFACT_CODEX_WORKFLOW => Some(ProviderKind::Codex),
            _ => None,
        };
        if let Some(provider) = artifact_provider {
            let completion = if matches!(
                workflow_name,
                APPROVAL_ARTIFACT_CLAUDE_WORKFLOW | APPROVAL_ARTIFACT_CODEX_WORKFLOW
            ) {
                NodeCompletion::Approval
            } else {
                NodeCompletion::Auto
            };
            let mut node = acceptance_session_node("agent", provider, completion);
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
                entry: "agent".to_string(),
            });
        }
        let (provider, completion, chained) = match workflow_name {
            AUTO_CLAUDE_WORKFLOW => (ProviderKind::Claude, NodeCompletion::Auto, false),
            AUTO_CODEX_WORKFLOW => (ProviderKind::Codex, NodeCompletion::Auto, false),
            AUTO_CHAIN_CLAUDE_WORKFLOW => (ProviderKind::Claude, NodeCompletion::Auto, true),
            AUTO_CHAIN_CODEX_WORKFLOW => (ProviderKind::Codex, NodeCompletion::Auto, true),
            APPROVAL_CLAUDE_WORKFLOW => (ProviderKind::Claude, NodeCompletion::Approval, false),
            APPROVAL_CODEX_WORKFLOW => (ProviderKind::Codex, NodeCompletion::Approval, false),
            _ => {
                return Err(WorkflowDefinitionResolverError::InvalidWorkflow(format!(
                    "unknown acceptance workflow '{workflow_name}'"
                )))
            }
        };
        // 直列は root sequence の隣接辺（rules 無し = リストの次へ）で表現する。
        let nodes = if chained {
            vec![
                NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Sequence(SequenceSpec {
                        entry: None,
                        children: vec![
                            ChildEntry::reference("agent-first"),
                            ChildEntry::reference("agent-second"),
                        ],
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::Auto,
                    worktree: None,
                },
                acceptance_session_node("agent-first", provider, completion),
                acceptance_session_node("agent-second", provider, completion),
            ]
        } else {
            vec![acceptance_session_node("agent", provider, completion)]
        };
        let entry = nodes[0].name.clone();
        Ok(WorkflowDefinition {
            name: workflow_name.to_string(),
            description: "Workflow control-plane product acceptance".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes,
            entry,
        })
    }
}

struct AcceptanceManagedWorktreeResolver;

struct AcceptanceWorktreeInventory;

struct AcceptanceWorkspaceNodeActionResolver;

impl WorkspaceNodeActionResolver for AcceptanceWorkspaceNodeActionResolver {
    fn resolve_approval_target(
        &self,
        _worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalTarget, WorkflowError> {
        Ok(WorkspaceNodeApprovalTarget {
            execution_id: node_id.to_string(),
            node_name: "session".to_string(),
            node_execution_id: node_id.to_string(),
        })
    }

    fn resolve_retry_target(
        &self,
        _worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeRetryTarget, WorkflowError> {
        Ok(WorkspaceNodeRetryTarget {
            execution_id: node_id.to_string(),
            node_execution_id: node_id.to_string(),
        })
    }

    fn resolve_session_rename_target(
        &self,
        _worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError> {
        Ok(WorkspaceSessionNodeRenameTarget {
            agent_session_id: node_id.to_string(),
        })
    }
}

impl WorktreeInventoryGateway for AcceptanceWorktreeInventory {
    fn snapshot(&self) -> Result<Vec<RepositoryWorktreeInventory>, WorkflowError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AcceptanceManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}

pub struct WorkflowControlPlaneAcceptanceHost<R: tauri::Runtime> {
    _app: tauri::App<R>,
    writer_lock_path: std::path::PathBuf,
    terminal: TerminalSurfaceRuntime,
    exit_observer: tauri::async_runtime::JoinHandle<()>,
    exit_observer_cancellation:
        Arc<dyn crate::domain::terminal_surface::gateway::TerminalSurfaceEventCancellation>,
    provider_sessions: Arc<AgentSessionUsecase>,
    provider_launch: Arc<AgentSessionLaunchUsecase>,
    provider_launch_bindings: Arc<crate::usecase::provider_lifecycle::ProviderLifecycleUsecase>,
    provider_lifecycle: Arc<AgentSessionLifecycleUsecase>,
    runtime_driver: Arc<WorkflowRuntimeHost>,
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
        let composition = compose_agent_sessions(AgentSessionCompositionInput {
			store: store.clone(),
			data_dir: config.data_dir.clone(),
				provider_executable_config: Arc::new(
					crate::adaptor::gateway::agent_session::InMemoryProviderExecutableConfigRepository::new(
						config.claude_executable.as_ref().map(|path| path.to_string_lossy().into_owned()),
						config.codex_executable.as_ref().map(|path| path.to_string_lossy().into_owned()),
					)
					.map_err(|error| format!("Provider executable Config初期化失敗: {error:?}"))?,
				),
				provider_executable_probe: Arc::new(
					crate::adaptor::gateway::agent_session::LocalProviderExecutableProbeGateway::with_search_path(
						config.provider_search_path,
					),
				),
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

        let workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService> =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::with_repository(
                crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(
                    store.clone(),
                ),
                Arc::new(
                    crate::adaptor::gateway::workflow::WorkflowExecutionArchiveFileRepository::new(
                        config.data_dir.clone(),
                    ),
                ),
            );

        let driver = Arc::new(WorkflowRuntimeHost::new_canonical(
            Arc::new(AcceptanceWorkflowDefinitionResolver),
            Arc::new(AcceptanceManagedWorktreeResolver),
            Some(config.data_dir.clone()),
            workspace_query,
            composition.launch.clone(),
            composition.initial_instruction.clone(),
            composition.interrupt.clone(),
            composition.lifecycle.clone(),
            composition.availability_reader.clone(),
            Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            ),
            Arc::new(AcceptanceWorktreeInventory),
        ));
        let gateway = Arc::new(TauriWorkflowRuntimeCommandGateway::new_with_driver(
            app.handle().clone(),
            driver.clone(),
            repository,
            installation_id,
        ));
        let runtime = Arc::new(WorkflowRuntimeUsecase::new(gateway));
        let workspace_node_commands = Arc::new(WorkspaceNodeCommandUsecase::new(
            Arc::new(AcceptanceWorkspaceNodeActionResolver),
            runtime.clone(),
            composition.rename.clone(),
        ));
        app.manage(workspace_node_commands.clone());
        composition.execution_tree_stops.bind(runtime.clone());
        composition
            .execution_tree_registrations
            .bind(runtime.clone());

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
            Some(composition.lifecycle_ingress.clone()),
        );
        let local_api = binding.start(router, &tokio::runtime::Handle::current());

        let terminal_events = terminal.application().subscribe_events();
        let exit_observer_cancellation = terminal_events.cancellation.clone();
        let exit_observer = tauri::async_runtime::spawn(
			crate::adaptor::controller::agent_session_exit_observer::run_agent_session_exit_observer(
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
            provider_launch: composition.launch,
            provider_launch_bindings: composition.provider_lifecycle,
            provider_lifecycle: composition.lifecycle,
            runtime_driver: driver,
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

    #[doc(hidden)]
    pub async fn start_default_capacity_fanout_workflow(
        &self,
        worktree_path: &str,
    ) -> Result<String, String> {
        self.start_named_workflow(worktree_path, DEFAULT_CAP_FANOUT_CLAUDE_WORKFLOW)
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

    pub async fn start_approval_artifact_workflow(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
    ) -> Result<String, String> {
        let workflow_name = match provider {
            AcceptanceProvider::Claude => APPROVAL_ARTIFACT_CLAUDE_WORKFLOW,
            AcceptanceProvider::Codex => APPROVAL_ARTIFACT_CODEX_WORKFLOW,
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
            .recover_startup()
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn workflow_log(&self, execution_id: &str) -> Result<Vec<serde_json::Value>, String> {
        self.get(&format!("/v1/workflow/executions/{execution_id}/log"))
            .await
    }

    pub async fn execution_direct(
        &self,
        execution_id: &str,
    ) -> Result<Option<AcceptanceWorkflowExecution>, String> {
        Ok(self
            .runtime_driver
            .acceptance_state_by_execution_id(execution_id)
            .await
            .map(acceptance_execution_from_runtime))
    }

    pub async fn submit(&self, node_execution_id: &str) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/node-executions/{node_execution_id}/submit"),
                &serde_json::json!({}),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Submit response was not successful".to_string())
    }

    pub async fn submit_artifact(
        &self,
        node_execution_id: &str,
        contract: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/node-executions/{node_execution_id}/submit"),
                &serde_json::json!({
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

    pub async fn retry_workspace_node_from_tauri(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<(), String> {
        crate::adaptor::controller::command::workspace_tree::retry_workspace_node(
            self._app.state::<Arc<WorkspaceNodeCommandUsecase>>(),
            worktree_path.to_string(),
            node_id.to_string(),
        )
        .await
    }

    pub async fn abort(&self, execution_id: &str) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/abort"),
                &serde_json::json!({}),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Abort response was not successful".to_string())
    }

    pub async fn stop(&self, execution_id: &str) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/stop"),
                &serde_json::json!({}),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Stop response was not successful".to_string())
    }

    pub async fn resume(&self, execution_id: &str) -> Result<(), String> {
        let response: MutationResponse = self
            .post(
                &format!("/v1/workflow/executions/{execution_id}/resume"),
                &serde_json::json!({}),
            )
            .await?;
        response
            .ok
            .then_some(())
            .ok_or_else(|| "Resume response was not successful".to_string())
    }

    pub async fn launch_manual_agent_session(
        &self,
        worktree_path: &str,
        provider: AcceptanceProvider,
        caller_request_id: &str,
    ) -> Result<String, String> {
        self.provider_launch
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new(worktree_path),
                worktree_path: worktree_path.to_string(),
                provider: match provider {
                    AcceptanceProvider::Claude => ProviderKind::Claude,
                    AcceptanceProvider::Codex => ProviderKind::Codex,
                },
                rows: 24,
                cols: 80,
                caller_request_id: caller_request_id.to_string(),
            })
            .await
            .map(|session| session.session().id().to_string())
            .map_err(|error| format!("{error:?}"))
    }

    pub async fn resume_agent_session(&self, agent_session_id: &str) -> Result<(), String> {
        self.provider_lifecycle
            .resume(
                agent_session_id,
                24,
                80,
                &format!("acceptance-resume-{agent_session_id}"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }

    pub async fn archive_agent_session(&self, agent_session_id: &str) -> Result<(), String> {
        self.provider_lifecycle
            .archive(
                agent_session_id,
                &format!("acceptance-archive-{agent_session_id}"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }

    pub async fn restore_agent_session(&self, agent_session_id: &str) -> Result<(), String> {
        self.provider_lifecycle
            .restore(
                agent_session_id,
                24,
                80,
                &format!("acceptance-restore-{agent_session_id}"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }

    pub fn workspace_node_status(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<AcceptanceWorkspaceNodeStatus>, String> {
        let store = self
            ._app
            .try_state::<Arc<LocalEventStore>>()
            .map(|store| store.inner().clone())
            .ok_or_else(|| "LocalEventStore is not managed".to_string())?;
        let repository =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(store);
        repository
            .load_node_by_node_execution_id(node_execution_id)
            .map_err(|error| error.to_string())
            .map(|node| {
                node.map(|node| match node.status_classification {
                    WorkspaceNodeStatusClassification::Active => {
                        AcceptanceWorkspaceNodeStatus::Active
                    }
                    WorkspaceNodeStatusClassification::Attention => {
                        AcceptanceWorkspaceNodeStatus::Attention
                    }
                    WorkspaceNodeStatusClassification::Failure => {
                        AcceptanceWorkspaceNodeStatus::Failure
                    }
                    WorkspaceNodeStatusClassification::Idle => AcceptanceWorkspaceNodeStatus::Idle,
                    WorkspaceNodeStatusClassification::Unbound => {
                        AcceptanceWorkspaceNodeStatus::Unbound
                    }
                })
            })
    }

    pub fn workspace_node_detail_status(
        &self,
        worktree_path: &str,
        node_execution_id: &str,
    ) -> Result<Option<String>, String> {
        let store = self
            ._app
            .try_state::<Arc<LocalEventStore>>()
            .map(|store| store.inner().clone())
            .ok_or_else(|| "LocalEventStore is not managed".to_string())?;
        let repository =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(store);
        let Some(node) = repository
            .load_node_by_node_execution_id(node_execution_id)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let data_dir = self
            .writer_lock_path
            .parent()
            .ok_or_else(|| "Local Event Store data directory is unavailable".to_string())?;
        let query =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::with_repository(
                repository,
                Arc::new(
                    crate::adaptor::gateway::workflow::WorkflowExecutionArchiveFileRepository::new(
                        data_dir.to_path_buf(),
                    ),
                ),
            );
        query
            .node_detail(&WorkspaceIdentity::new(worktree_path), &node.id)
            .map(|detail| detail.map(|detail| detail.status_classification))
            .map_err(|error| error.to_string())
    }

    pub fn execution_fact_event_types(&self, tree_id: &str) -> Result<Vec<String>, String> {
        let store = self
            ._app
            .try_state::<Arc<LocalEventStore>>()
            .map(|store| store.inner().clone())
            .ok_or_else(|| "LocalEventStore is not managed".to_string())?;
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&store, tree_id)
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| record.fact.event_type().to_string())
                    .collect()
            })
            .map_err(|error| error.to_string())
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

    pub async fn agent_session_has_active_launch_binding(
        &self,
        agent_session_id: &str,
    ) -> Result<bool, String> {
        let session = self
            .provider_sessions
            .find(agent_session_id)
            .await
            .map_err(|error| format!("{error:?}"))?
            .ok_or_else(|| format!("AgentSession '{agent_session_id}' not found"))?;
        let scope =
            ProviderLifecycleScope::new(agent_session_id).map_err(|error| error.to_string())?;
        self.provider_launch_bindings
            .active_launch_id(session.session().provider(), &scope)
            .await
            .map(|slot| slot.is_some())
            .map_err(|error| format!("{error:?}"))
    }

    pub fn active_provider_process_count(&self) -> usize {
        self.terminal
            .application()
            .summaries()
            .into_iter()
            .filter(|surface| {
                matches!(
                    surface.owner,
                    crate::domain::terminal_surface::TerminalSurfaceOwner::Session { .. }
                ) && !surface.process_state.is_exited()
            })
            .count()
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
            provider_launch,
            provider_launch_bindings,
            provider_lifecycle,
            runtime_driver,
            _runtime,
            local_api,
            local_api_base_url: _,
            local_api_token: _,
        } = self;
        exit_observer_cancellation.cancel();
        exit_observer
            .await
            .map_err(|error| format!("join AgentSession exit observer: {error}"))?;
        terminal.shutdown()?;
        local_api
            .shutdown_and_wait()
            .await
            .map_err(|error| format!("join local API server: {error}"))?;
        _app.unmanage::<Arc<WorkspaceNodeCommandUsecase>>();
        _app.unmanage::<Arc<LocalEventStore>>();
        drop((
            local_api,
            _runtime,
            provider_sessions,
            provider_launch,
            provider_launch_bindings,
            provider_lifecycle,
            runtime_driver,
            terminal,
            _app,
        ));
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

fn acceptance_execution_from_runtime(
    snapshot: WorkflowRuntimeSnapshot,
) -> AcceptanceWorkflowExecution {
    AcceptanceWorkflowExecution {
        id: snapshot.execution_id,
        status: match snapshot.state {
            RuntimeExecutionState::Running => AcceptanceWorkflowExecutionStatus::Running,
            #[cfg(test)]
            RuntimeExecutionState::WaitingApproval => {
                AcceptanceWorkflowExecutionStatus::WaitingApproval
            }
            RuntimeExecutionState::Completed => AcceptanceWorkflowExecutionStatus::Completed,
            RuntimeExecutionState::Aborted => AcceptanceWorkflowExecutionStatus::Aborted,
            #[cfg(test)]
            RuntimeExecutionState::Interrupted => AcceptanceWorkflowExecutionStatus::Interrupted,
        },
        node_executions: snapshot
            .node_executions
            .into_iter()
            .map(|node| {
                let can_retry = node.can_retry();
                AcceptanceNodeExecution {
                    id: node.id,
                    node_name: node.node_name,
                    kind: match node.kind {
                        NodeKindName::Command => AcceptanceNodeKind::Command,
                        NodeKindName::Session => AcceptanceNodeKind::Session,
                        NodeKindName::Fanout => AcceptanceNodeKind::Fanout,
                        NodeKindName::Sequence => AcceptanceNodeKind::Sequence,
                    },
                    attempt: node.attempt,
                    status: match node.status {
                        NodeExecutionStatus::Running => AcceptanceNodeExecutionStatus::Running,
                        NodeExecutionStatus::Paused => AcceptanceNodeExecutionStatus::Paused,
                        NodeExecutionStatus::WaitingApproval => {
                            AcceptanceNodeExecutionStatus::WaitingApproval
                        }
                        NodeExecutionStatus::Succeeded => AcceptanceNodeExecutionStatus::Succeeded,
                        NodeExecutionStatus::Failed => AcceptanceNodeExecutionStatus::Failed,
                        NodeExecutionStatus::Aborted => AcceptanceNodeExecutionStatus::Aborted,
                    },
                    agent_session_id: node.session_id,
                    submit_received: matches!(
                        node.completion_signals,
                        NodeCompletionSignalState::SubmitReceived
                            | NodeCompletionSignalState::Ready
                    ),
                    stop_received: matches!(
                        node.completion_signals,
                        NodeCompletionSignalState::StopReceived | NodeCompletionSignalState::Ready
                    ),
                    can_approve: node.status == NodeExecutionStatus::WaitingApproval,
                    can_retry,
                    has_artifact: node.artifact.is_some(),
                    artifact: node.artifact.map(|artifact| artifact.value),
                    failure_reason: node.failure.map(|failure| failure.reason),
                }
            })
            .collect(),
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
            kind: match value.kind {
                NodeKindResponse::Command => AcceptanceNodeKind::Command,
                NodeKindResponse::Session => AcceptanceNodeKind::Session,
                NodeKindResponse::Fanout => AcceptanceNodeKind::Fanout,
                NodeKindResponse::Sequence => AcceptanceNodeKind::Sequence,
            },
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
            artifact: value.artifact.map(|artifact| artifact.value),
            failure_reason: value.failure.map(|failure| failure.reason),
        }
    }
}
