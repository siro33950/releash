use crate::adaptor::gateway::workflow::fact_codec;
use std::sync::Arc;

use serde::Deserialize;

use crate::adaptor::controller::agent_session_wiring::{
    compose_agent_sessions, AgentSessionCompositionInput,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost;
use crate::adaptor::gateway::workflow::WorkflowRuntimeCommandGateway;
use crate::domain::agent_session::aggregates::AgentSessionLifecycle;
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleScope};
use crate::domain::workflow::{
    ChildEntry, FacetRefs, FanoutSpec, NodeCompletion, NodeCompletionSignalState, NodeDefinition,
    NodeExecutionStatus, NodeKind, NodeKindName, RuntimeExecutionState, SchemaDef, SequenceSpec,
    SessionSpec, WorkflowDefinition, WorkflowError, WorkflowRuntimeSnapshot,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::domain::workspace_tree::{WorkspaceNodeStatusClassification, WorkspaceTreeRepository};
use crate::infrastructure::local_api::LocalApiServer;
use crate::terminal_subscription_acceptance::TerminalSubscriptionHarness as TerminalSurfaceRuntime;
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
    Completed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceNodeExecutionStatus {
    Running,
    WaitingApproval,
    Succeeded,
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
    Idle,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceWorkflowExecution {
    pub id: String,
    pub status: AcceptanceWorkflowExecutionStatus,
    pub node_executions: Vec<AcceptanceNodeExecution>,
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
    Completed,
    Aborted,
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
}

#[derive(Deserialize)]
struct ArtifactResponse {
    value: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum NodeExecutionStatusResponse {
    Running,
    WaitingApproval,
    Succeeded,
    Aborted,
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
                completion: NodeCompletion::default(),
                worktree: None,
            }];
            nodes.extend(child_names.iter().map(|name| {
                acceptance_session_node(
                    name,
                    ProviderKind::Claude,
                    NodeCompletion::require_approval(),
                )
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
                        completion: NodeCompletion::default(),
                        worktree: None,
                    },
                    acceptance_session_node(
                        "review-a",
                        provider,
                        NodeCompletion::require_approval(),
                    ),
                    acceptance_session_node(
                        "review-b",
                        provider,
                        NodeCompletion::require_approval(),
                    ),
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
                NodeCompletion::require_approval()
            } else {
                NodeCompletion::default()
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
            AUTO_CLAUDE_WORKFLOW => (ProviderKind::Claude, NodeCompletion::default(), false),
            AUTO_CODEX_WORKFLOW => (ProviderKind::Codex, NodeCompletion::default(), false),
            AUTO_CHAIN_CLAUDE_WORKFLOW => (ProviderKind::Claude, NodeCompletion::default(), true),
            AUTO_CHAIN_CODEX_WORKFLOW => (ProviderKind::Codex, NodeCompletion::default(), true),
            APPROVAL_CLAUDE_WORKFLOW => (
                ProviderKind::Claude,
                NodeCompletion::require_approval(),
                false,
            ),
            APPROVAL_CODEX_WORKFLOW => (
                ProviderKind::Codex,
                NodeCompletion::require_approval(),
                false,
            ),
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
                    completion: NodeCompletion::default(),
                    worktree: None,
                },
                acceptance_session_node("agent-first", provider, completion.clone()),
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

struct AcceptanceWorkspaceNodeActionResolver(Arc<LocalEventStore>);

#[async_trait::async_trait]
impl WorkspaceNodeActionResolver for AcceptanceWorkspaceNodeActionResolver {
    async fn resolve_approval_target(
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

    async fn resolve_retry_target(
        &self,
        _worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeRetryTarget, WorkflowError> {
        Ok(WorkspaceNodeRetryTarget {
            execution_id: crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(
                self.0.clone(),
            )
            .tree_id_for_node(node_id)
            .await
            .map_err(WorkflowError::from)?
            .ok_or_else(|| WorkflowError::NotFound(node_id.to_string()))?,
            node_execution_id: node_id.to_string(),
        })
    }

    async fn resolve_session_resume_target(
        &self,
        _worktree_path: &str,
        node_id: &str,
    ) -> Result<crate::usecase::workflow::command::ResumeSessionNodeCommand, WorkflowError> {
        Ok(
            crate::usecase::workflow::command::ResumeSessionNodeCommand {
                execution_id:
                    crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(
                        self.0.clone(),
                    )
                    .tree_id_for_node(node_id)
                    .await
                    .map_err(WorkflowError::from)?
                    .ok_or_else(|| WorkflowError::NotFound(node_id.to_string()))?,
                node_execution_id: node_id.to_string(),
            },
        )
    }

    async fn resolve_session_rename_target(
        &self,
        _worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceSessionNodeRenameTarget, WorkflowError> {
        Ok(WorkspaceSessionNodeRenameTarget {
            agent_session_id: node_id.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AcceptanceManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}

pub struct WorkflowControlPlaneAcceptanceHost {
    retrying: Arc<crate::usecase::retry::Retrying>,
    startup: Option<Arc<crate::usecase::workflow::startup::WorkflowStartupUsecase>>,
    store: Arc<LocalEventStore>,
    workspace_node_commands: Arc<WorkspaceNodeCommandUsecase>,
    writer_lock_path: std::path::PathBuf,
    terminal: TerminalSurfaceRuntime,
    exit_observer: tokio::task::JoinHandle<()>,
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
    workflow_read: Arc<crate::usecase::workflow::WorkflowReadUsecase>,
}

impl WorkflowControlPlaneAcceptanceHost {
    pub fn start(config: AgentSessionTuiAcceptanceConfig) -> Result<Self, String> {
        let work = crate::terminal_surface::initialize_background_work_for_acceptance();
        std::fs::create_dir_all(&config.data_dir).map_err(|error| error.to_string())?;
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            config.data_dir.clone(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ))
        .map_err(|error| error.to_string())?;
        let terminal = TerminalSurfaceRuntime::new(work.clone(), config.data_dir.clone());
        let composition = compose_agent_sessions(AgentSessionCompositionInput {
 hook_token: std::sync::Arc::<str>::from("hook-token"),
launch_retention: crate::adaptor::controller::agent_session_launch_retention::run(crate::infrastructure::timer::delays(crate::adaptor::controller::agent_session_launch_retention::RETENTION)),
            retrying: work.retrying.clone(),
            state_publisher: None,
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
			subscriptions: crate::acceptance_test_support::state_subscriptions(),
		})
		.map_err(|error| format!("Provider availability初期化失敗: {error:?}"))?;

        let workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService> =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::with_repository(
                crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(
                    store.clone(),
                ),
            );

        let mut driver = WorkflowRuntimeHost::new_canonical(
            work.retrying.clone(),
            Arc::new(AcceptanceWorkflowDefinitionResolver),
            Arc::new(AcceptanceManagedWorktreeResolver),
            workspace_query,
            composition.launch.clone(),
            composition.initial_instruction.clone(),
            composition.lifecycle.clone(),
            composition.availability_reader.clone(),
            Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway),
            crate::adaptor::gateway::daemon::serving(),
        );
        let node_processes = Arc::new(
            crate::adaptor::gateway::workflow::node_process::WorkflowNodeProcesses::new(
                terminal.application(),
            ),
        );
        driver.node_processes = node_processes.clone();
        let dependencies =
            crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies {
                store: Some(store.clone()),
                config: None,
                secrets: None,
                state_changes: crate::acceptance_test_support::state_subscriptions(),
            };
        let driver = Arc::new(driver);
        let startup = crate::adaptor::controller::wiring::wire_workflow_startup(
            dependencies.clone(),
            driver.clone(),
        );
        let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            dependencies,
            driver.clone(),
        ));
        let runtime = Arc::new(
            WorkflowRuntimeUsecase::new_with_worktree_operations(work.retrying.clone(),
                gateway,
                Arc::new(
                    crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::new(
                        store.clone(),
                        config.data_dir.clone(),
                    ),
                ),
                Arc::new(crate::usecase::worktree_operation::WorktreeOperations::new(Arc::new(
                    crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks::new(&config.data_dir),
                ))),
            ),
        );
        let workspace_node_commands = Arc::new(WorkspaceNodeCommandUsecase::new(
            Arc::new(AcceptanceWorkspaceNodeActionResolver(store.clone())),
            runtime.clone(),
            composition.rename.clone(),
        ));
        composition.execution_tree_stops.bind(runtime.clone());
        composition
            .execution_tree_registrations
            .bind(runtime.clone());

        let workflows_dir = config.data_dir.join("acceptance-workflows");
        std::fs::create_dir_all(&workflows_dir).map_err(|error| error.to_string())?;
        let workflow_read = Arc::new(crate::acceptance_test_support::workflow_read(
            store.clone(),
            config.data_dir.clone(),
            Some(workflows_dir),
        ));
        let (binding, daemon) =
            crate::acceptance_test_support::client_binding(config.data_dir.clone())
                .map_err(|error| error.to_string())?;
        let port = binding.port();
        let token = binding.client_bearer_token();
        let mut dependencies =
            crate::acceptance_test_support::build_client_dependencies(config.data_dir.clone());
        dependencies.daemon = daemon;
        dependencies.workflow_runtime_usecase = Some(runtime.clone());
        dependencies.workspace_node_command_usecase = Some(workspace_node_commands.clone());
        let mut dispatch = crate::adaptor::controller::client::ClientCommandDispatch::new(
            dependencies.daemon.clone(),
        );
        dispatch.register_dependencies(&dependencies);
        let presenter = terminal.presenter();
        let state = terminal.subscriptions().with_reads(
            Arc::new(AcceptanceWorkflowStateReads(workflow_read.clone())),
            None,
            vec![],
            String::new(),
        );
        let subscriptions = terminal.terminal_subscriptions();
        let client = crate::adaptor::controller::api::ClientApiDeps::new(
            Arc::new(dispatch),
            crate::adaptor::controller::daemon::client_priority_interceptor(),
        )
        .with_provider_lifecycle(composition.lifecycle_ingress.clone())
        .with_state_subscriptions(
            crate::adaptor::controller::api::StateSubscriptionDeps::new(
                state,
                presenter,
                subscriptions,
            ),
        );
        let router = crate::adaptor::controller::api::build_router(
            crate::adaptor::controller::api::auth::ClientTokens {
                operator: token.clone(),
                hook: binding.hook_bearer_token(),
            },
            Some(client),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        let local_api = binding
            .start(router, &tokio::runtime::Handle::current())
            .inspect(|server| {
                server.publish_discovery().unwrap();
            })
            .unwrap();

        let terminal_events = terminal.application().subscribe_events();
        let exit_observer_cancellation = terminal_events.cancellation.clone();
        let exit_observer = tokio::spawn(
			crate::adaptor::controller::agent_session_exit_observer::run_agent_session_exit_observer(
				terminal_events,
				composition.exit.clone(),
			),
		);

        Ok(Self {
            retrying: work.retrying.clone(),
            startup,
            store,
            workspace_node_commands,
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
            local_api_token: token.token().to_string(),
            workflow_read,
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
        let value = self.call("start_workflow", serde_json::json!({"workflowName":workflow_name,"worktreePath":worktree_path,"request":"acceptance initial instruction","createdFrom":"api"})).await?;
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    pub async fn execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<AcceptanceWorkflowExecution>, String> {
        let target = crate::usecase::state_subscription::SubscriptionTarget::WorkflowExecution(
            execution_id.to_string(),
        );
        let value = crate::client_api_acceptance::read_state(&self.client(), &target.to_string())
            .await
            .map_err(|error| error.to_string())?;
        let value = value["value"].clone();
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value::<ExecutionResponse>(value)
            .map(|response| Some(response.into()))
            .map_err(|error| error.to_string())
    }

    pub async fn recover_startup(&self) -> Result<(), String> {
        let Some(startup) = &self.startup else {
            return Ok(());
        };
        crate::adaptor::controller::workflow_startup::recover(&self.retrying, startup)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn workflow_log(&self, execution_id: &str) -> Result<Vec<serde_json::Value>, String> {
        use crate::usecase::workflow::ports::WorkflowEventRepository;
        let execution_id = crate::domain::workflow::ExecutionTreeId::new(execution_id.to_string())
            .map_err(|error| error.to_string())?;
        Ok(
            crate::adaptor::gateway::workflow::WorkflowEventLogRepository::with_store(
                self.store.clone(),
            )
            .read(&execution_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|event| {
                let mut object = match event.payload {
                    serde_json::Value::Object(object) => object,
                    other => serde_json::Map::from_iter([("payload".to_string(), other)]),
                };
                object.insert("event".to_string(), event.event_kind.into());
                serde_json::Value::Object(object)
            })
            .collect(),
        )
    }

    pub async fn execution_direct(
        &self,
        execution_id: &str,
    ) -> Result<Option<AcceptanceWorkflowExecution>, String> {
        self.runtime_driver
            .acceptance_state_by_execution_id(
                &crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies {
                    store: Some(self.store.clone()),
                    config: None,
                    secrets: None,
                    state_changes: crate::acceptance_test_support::state_subscriptions(),
                },
                execution_id,
            )
            .await
            .map(|snapshot| snapshot.map(acceptance_execution_from_runtime))
            .map_err(|error| error.to_string())
    }

    pub async fn submit(&self, node_execution_id: &str) -> Result<(), String> {
        self.call(
            "workflow_submit_output",
            serde_json::json!({"nodeExecutionId":node_execution_id,"artifact":null}),
        )
        .await
        .map(|_| ())
    }

    pub async fn submit_artifact(
        &self,
        node_execution_id: &str,
        contract: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        self.call("workflow_submit_output",serde_json::json!({"nodeExecutionId":node_execution_id,"artifact":{"contract":contract,"value":value}})).await.map(|_| ())
    }

    pub async fn approve(
        &self,
        execution_id: &str,
        node_name: &str,
        node_execution_id: &str,
    ) -> Result<(), String> {
        self.call("approve_workflow_node",serde_json::json!({"args":{"executionId":execution_id,"nodeName":node_name,"nodeExecutionId":node_execution_id,"comment":null}})).await.map(|_| ())
    }

    pub async fn retry(&self, execution_id: &str, node_execution_id: &str) -> Result<(), String> {
        let execution = self
            .workflow_read
            .get_execution_state(execution_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "execution not found".to_string())?;
        self.call(
            "retry_workspace_node",
            serde_json::json!({"worktreePath":execution.worktree_path,"nodeId":node_execution_id}),
        )
        .await
        .map(|_| ())
    }

    pub async fn retry_workspace_node_from_tauri(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<(), String> {
        crate::adaptor::controller::client::workspace_tree::retry_workspace_node_shared(
            &self.workspace_node_commands,
            worktree_path.to_string(),
            node_id.to_string(),
        )
        .await
        .map_err(|error| error.to_string())
    }

    pub async fn abort(&self, execution_id: &str) -> Result<(), String> {
        self.call(
            "abort_workflow",
            serde_json::json!({"executionId":execution_id}),
        )
        .await
        .map(|_| ())
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

    pub async fn resume_session_node(&self, node_execution_id: &str) -> Result<(), String> {
        let execution_id = crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(
            self.store.clone(),
        )
        .tree_id_for_node(node_execution_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "node execution not found".to_string())?;
        let execution = self
            .workflow_read
            .get_execution_state(&execution_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "execution not found".to_string())?;
        self.call(
            "resume_workspace_session_node",
            serde_json::json!({"worktreePath":execution.worktree_path,"nodeId":node_execution_id}),
        )
        .await
        .map(|_| ())
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

    pub async fn workspace_node_status(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<AcceptanceWorkspaceNodeStatus>, String> {
        let repository =
            crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(
                self.store.clone(),
            );
        let backend = crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(
            self.store.clone(),
        );
        let Some(tree_id) = backend
            .tree_id_for_node(node_execution_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let Some(tree) =
            crate::adaptor::gateway::workflow::fact_log::fold_tree_from(&backend, &tree_id)
                .await
                .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let workspace =
            crate::domain::workspace_tree::WorkspaceIdentity::new(&tree.root.workspace_identity);
        let tree = repository
            .load_trees(&[workspace])
            .await
            .remove(0)
            .map_err(|error| error.to_string())?;
        Ok(tree
            .nodes()
            .iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id))
            .map(|node| match node.status_classification {
                WorkspaceNodeStatusClassification::Active => AcceptanceWorkspaceNodeStatus::Active,
                WorkspaceNodeStatusClassification::Attention => {
                    AcceptanceWorkspaceNodeStatus::Attention
                }
                WorkspaceNodeStatusClassification::Idle => AcceptanceWorkspaceNodeStatus::Idle,
            }))
    }

    pub async fn execution_fact_event_types(&self, tree_id: &str) -> Result<Vec<String>, String> {
        crate::adaptor::gateway::workflow::fact_log::read_tree_records(&self.store, tree_id)
            .await
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| fact_codec::event_type(&record.fact).to_string())
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

    fn client(&self) -> crate::client_api_acceptance::NativeClient {
        crate::client_api_acceptance::connect_client(
            &crate::client_api_acceptance::ClientEndpoint {
                url: self.local_api_base_url.clone(),
                token: self.local_api_token.clone(),
            },
        )
    }

    async fn call(&self, name: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        crate::client_api_acceptance::request_client(&self.client(), name, args)
            .await
            .map_err(|error| error.to_string())
    }

    #[allow(deprecated)]
    pub async fn shutdown(self) -> Result<(), String> {
        let Self {
            retrying,
            startup,
            store,
            workspace_node_commands,
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
            workflow_read,
        } = self;
        _runtime.shutdown_active_commands().await;
        exit_observer_cancellation.cancel();
        exit_observer
            .await
            .map_err(|error| format!("join AgentSession exit observer: {error}"))?;
        terminal.shutdown()?;
        local_api
            .shutdown_and_wait()
            .await
            .map_err(|error| format!("join local API server: {error}"))?;
        drop((
            local_api,
            _runtime,
            startup,
            retrying,
            provider_sessions,
            provider_launch,
            provider_launch_bindings,
            provider_lifecycle,
            runtime_driver,
            terminal,
            store,
            workspace_node_commands,
            workflow_read,
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
            RuntimeExecutionState::Completed => AcceptanceWorkflowExecutionStatus::Completed,
            RuntimeExecutionState::Aborted => AcceptanceWorkflowExecutionStatus::Aborted,
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
                        NodeExecutionStatus::WaitingApproval => {
                            AcceptanceNodeExecutionStatus::WaitingApproval
                        }
                        NodeExecutionStatus::Succeeded => AcceptanceNodeExecutionStatus::Succeeded,
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
                ExecutionStatusResponse::Completed => AcceptanceWorkflowExecutionStatus::Completed,
                ExecutionStatusResponse::Aborted => AcceptanceWorkflowExecutionStatus::Aborted,
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
                NodeExecutionStatusResponse::WaitingApproval => {
                    AcceptanceNodeExecutionStatus::WaitingApproval
                }
                NodeExecutionStatusResponse::Succeeded => AcceptanceNodeExecutionStatus::Succeeded,
                NodeExecutionStatusResponse::Aborted => AcceptanceNodeExecutionStatus::Aborted,
            },
            agent_session_id: value.session_id,
            submit_received: value.submit_received,
            stop_received: value.stop_received,
            can_approve: value.can_approve,
            can_retry: value.can_retry,
            has_artifact: value.has_artifact,
            artifact: value.artifact.map(|artifact| artifact.value),
        }
    }
}

#[cfg(test)]
#[path = "workflow_control_plane_acceptance_test.rs"]
mod workflow_control_plane_acceptance_tests;

struct AcceptanceWorkflowStateReads(Arc<crate::usecase::workflow::WorkflowReadUsecase>);
#[async_trait::async_trait]
impl crate::usecase::state_subscription::StateSubscriptionRead for AcceptanceWorkflowStateReads {
    async fn read(
        &self,
        target: &crate::usecase::state_subscription::SubscriptionTarget,
    ) -> Result<
        crate::usecase::state_subscription::StateValue,
        crate::usecase::state_subscription::StateReadError,
    > {
        use crate::usecase::state_subscription::{StateReadError, StateValue, SubscriptionTarget};
        let result = match target {
            SubscriptionTarget::WorkflowExecution(id) => self
                .0
                .get_execution_state(id)
                .await
                .map(StateValue::WorkflowExecution),
            _ => Err(WorkflowError::NotFound("unknown target".into())),
        };
        result.map_err(|error| StateReadError {
            message: error.to_string(),
            source: error.into(),
        })
    }
    fn repositories(&self) -> Vec<String> {
        vec![]
    }
}
