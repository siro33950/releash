use crate::usecase::agent_session::AgentSessionQueryService;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::adaptor::gateway::agent_session::{
    LocalAgentSessionQueryService, LocalAgentSessionRepository,
};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::{
    LocalProviderHookHealthRepository, LocalProviderLifecycleCredentialGateway,
    LocalProviderLifecycleEventRepository,
};
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::adaptor::gateway::workflow::node_session_boundary::{
    NodeSessionInfo, WorkflowAgentSessionPort, WorkflowSessionLaunchConfig,
};
use crate::adaptor::gateway::workflow::workflow_host::{
    WorkflowRuntimeDependencies, WorkflowRuntimeHost,
};
use crate::adaptor::gateway::workflow::{
    ExecutionTreeArchiveFactRepository, WorkflowRuntimeCommandGateway,
};
use crate::adaptor::gateway::workspace_tree::{
    SqliteWorkspaceQueryService, SqliteWorkspaceTreeRepository,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderKind, ProviderLifecycleIngressResult, ProviderLifecycleScope,
    ProviderLifecycleSignal, ProviderLifecycleSlotId,
};
pub use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
pub use crate::domain::workflow::NodeExecutionStatus;
use crate::domain::workflow::{
    ExecutionOrigin, IsolatedWorktree, IsolatedWorktreeGateway, NodeFact, WorkflowDefinition,
    WorkflowError,
};
pub use crate::provider_lifecycle_acceptance::{AcceptanceIngressResult, AcceptanceProvider};
use crate::usecase::agent_session::AgentSessionUsecase;
use crate::usecase::provider_lifecycle::{
    ProviderExecutionTreeStopCommand, ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase,
    ProviderLifecycleUsecase,
};
use crate::usecase::workflow::command::{SubmitOutputArtifact, SubmitOutputCommand};
use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::usecase::workflow::WorkflowRuntimeUsecase;

pub struct AcceptanceLaunch(ArmedProviderLifecycle);

pub struct WorkflowDelegateAcceptanceHost {
    store: Arc<LocalEventStore>,
    dependencies: WorkflowRuntimeDependencies,
    host: WorkflowRuntimeHost,
    sessions: Arc<AcceptanceSessions>,
    control: WorkflowControlPlaneUsecase,
    lifecycle: Arc<ProviderLifecycleUsecase>,
    ingress: ProviderLifecycleIngressUsecase,
}

impl WorkflowDelegateAcceptanceHost {
    pub fn new(data_dir: &Path) -> Self {
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(data_dir.into())).unwrap();
        let archives = Arc::new(ExecutionTreeArchiveFactRepository::new(
            store.clone(),
            data_dir,
        ));
        let workspace_query = SqliteWorkspaceQueryService::with_repository(
            SqliteWorkspaceTreeRepository::new(store.clone()),
            archives.clone(),
        );
        let sessions = Arc::new(AcceptanceSessions::default());
        let host = WorkflowRuntimeHost::with_runtime_ports(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptanceWorktrees),
            workspace_query,
            sessions.clone(),
            Arc::new(AcceptanceWorktrees),
        );
        let dependencies = WorkflowRuntimeDependencies {
            processes: host.node_processes.clone(),
            store: Some(store.clone()),
            config: None,
            secrets: None,
            push: Arc::new(crate::infrastructure::push::PushSink::new()),
        };
        let host = crate::adaptor::controller::wiring::wire_delegate_continuation(
            dependencies.clone(),
            host,
        );
        let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            dependencies.clone(),
            Arc::new(host.clone()),
        ));
        let runtime = Arc::new(WorkflowRuntimeUsecase::new_with_worktree_operations(
            gateway.clone(),
            archives,
            Arc::new(crate::usecase::worktree_operation::WorktreeOperations::new(
                Arc::new(
                    crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks::new(data_dir),
                ),
            )),
        ));
        let repository = Arc::new(LocalAgentSessionRepository::new(store.clone()));
        let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(LocalProviderLifecycleEventRepository::new(
                store.clone(),
                store.installation_id().to_string(),
            )),
        ));
        let ingress = ProviderLifecycleIngressUsecase::new(
            lifecycle.clone(),
            Arc::new(AgentSessionUsecase::new(repository.clone())),
            Arc::new(ProviderHookHealthUsecase::new(Arc::new(
                LocalProviderHookHealthRepository::new(
                    store.clone(),
                    store.installation_id().to_string(),
                ),
            ))),
            repository,
            runtime,
            Arc::new(
                crate::adaptor::gateway::push::ClientAgentSessionChangeNotifier::new(
                    dependencies.push.clone(),
                ),
            ),
        );
        Self {
            store,
            dependencies,
            host,
            sessions,
            control: WorkflowControlPlaneUsecase::new(gateway),
            lifecycle,
            ingress,
        }
    }

    pub async fn start(&self, nodes: &str, worktree: &str) -> String {
        let definition = serde_saphyr::from_str(&format!(
            "name: delegate\ndescription: test\nnodes:\n{nodes}"
        ))
        .unwrap();
        self.host
            .start_resolved_workflow(
                &self.dependencies,
                definition,
                worktree.into(),
                None,
                ExecutionOrigin::Cli,
            )
            .await
            .unwrap()
    }

    pub async fn submit(&self, node_id: &str, value: serde_json::Value) {
        self.control
            .submit_output(SubmitOutputCommand {
                node_execution_id: node_id.into(),
                artifact: Some(SubmitOutputArtifact {
                    contract: "result".into(),
                    value,
                }),
            })
            .await
            .unwrap();
    }

    pub async fn stop_parent(&self, tree: &str, node: &RuntimeNodeExecution) {
        self.control
            .record_provider_stop(
                ProviderExecutionTreeStopCommand {
                    agent_session_id: node.session_id.clone().unwrap(),
                    tree_id: tree.into(),
                    node_execution_id: node.id.clone(),
                    binding_id: "binding".into(),
                },
                Vec::new(),
            )
            .await
            .unwrap();
    }

    pub async fn arm(&self, session_id: &str, provider: AcceptanceProvider) -> AcceptanceLaunch {
        AcceptanceLaunch(
            self.lifecycle
                .arm(
                    ProviderLifecycleSlotId::new(session_id).unwrap(),
                    match provider {
                        AcceptanceProvider::Claude => ProviderKind::Claude,
                        AcceptanceProvider::Codex => ProviderKind::Codex,
                    },
                    ProviderLifecycleScope::new(session_id).unwrap(),
                )
                .await
                .unwrap(),
        )
    }

    pub async fn session_started(
        &self,
        launch: &AcceptanceLaunch,
        provider_session_id: &str,
    ) -> AcceptanceIngressResult {
        let armed = &launch.0;
        self.receive(
            armed,
            ProviderLifecycleSignal::session_started(
                armed.binding_id(),
                armed.provider(),
                armed.scope().clone(),
                provider_session_id,
                None,
            )
            .unwrap(),
        )
        .await
    }

    pub async fn stop_observed(
        &self,
        launch: &AcceptanceLaunch,
        provider_session_id: &str,
    ) -> AcceptanceIngressResult {
        let armed = &launch.0;
        self.receive(
            armed,
            ProviderLifecycleSignal::stop_observed(
                armed.binding_id(),
                armed.provider(),
                armed.scope().clone(),
                provider_session_id,
                None,
            )
            .unwrap(),
        )
        .await
    }

    async fn receive(
        &self,
        armed: &ArmedProviderLifecycle,
        signal: ProviderLifecycleSignal,
    ) -> AcceptanceIngressResult {
        match self
            .ingress
            .receive(armed.slot_id(), armed.capability(), signal)
            .await
            .unwrap()
        {
            ProviderLifecycleIngressResult::Applied => AcceptanceIngressResult::Applied,
            ProviderLifecycleIngressResult::Duplicate => AcceptanceIngressResult::Duplicate,
            ProviderLifecycleIngressResult::Rejected(reason) => AcceptanceIngressResult::Rejected {
                reason: format!("{reason:?}"),
            },
        }
    }

    pub async fn session_context(&self, session_id: &str) -> (String, Option<String>) {
        let session = LocalAgentSessionQueryService::new(self.store.clone())
            .get(session_id)
            .await
            .unwrap()
            .unwrap();
        (session.worktree_path, session.provider_session_id)
    }

    pub async fn nodes(&self, tree: &str) -> Vec<RuntimeNodeExecution> {
        fact_log::fold_tree_from(&FactLogReadBackend::Live(self.store.clone()), tree)
            .await
            .unwrap()
            .unwrap()
            .aggregate
            .node_executions
            .clone()
    }

    pub async fn session_facts(&self, tree: &str, node_id: &str) -> (Vec<String>, usize) {
        let records = fact_log::read_tree_records(&self.store, tree)
            .await
            .unwrap();
        let mut provider_sessions = Vec::new();
        let mut stops = 0;
        for record in records
            .into_iter()
            .filter(|record| record.meta.node_execution_id == node_id)
        {
            match record.fact {
                NodeFact::SessionAttached(fact) => {
                    provider_sessions.extend(fact.provider_session_id)
                }
                NodeFact::StopReceived(_) => stops += 1,
                _ => {}
            }
        }
        (provider_sessions, stops)
    }

    pub fn launched_cwd(&self, node_id: &str) -> String {
        self.sessions
            .prepared
            .lock()
            .unwrap()
            .iter()
            .find(|(id, _)| id == node_id)
            .unwrap()
            .1
            .clone()
    }

    pub fn is_activated(&self, node_id: &str) -> bool {
        self.sessions
            .activated
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == node_id)
    }
}

struct UnusedWorkflowResolver;

#[async_trait::async_trait]
impl WorkflowDefinitionResolver for UnusedWorkflowResolver {
    async fn resolve(
        &self,
        _name: &str,
    ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
        unreachable!("workflow definition is supplied by the integration test")
    }
}

struct AcceptanceWorktrees;

#[async_trait::async_trait]
impl ManagedWorktreeResolver for AcceptanceWorktrees {
    async fn resolve(&self, path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(path)
    }
}

impl IsolatedWorktreeGateway for AcceptanceWorktrees {
    fn repository_root(&self, _path: &str) -> Result<String, WorkflowError> {
        Ok("/repo".into())
    }
    fn is_created(
        &self,
        _parent: &str,
        _worktree: &IsolatedWorktree,
    ) -> Result<bool, WorkflowError> {
        Ok(false)
    }
    fn create(&self, _parent: &str, _worktree: &IsolatedWorktree) -> Result<(), WorkflowError> {
        Ok(())
    }
}

#[derive(Default)]
struct AcceptanceSessions {
    prepared: Mutex<Vec<(String, String)>>,
    activated: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowAgentSessionPort for AcceptanceSessions {
    fn is_provider_available(&self, _provider: ProviderKind) -> bool {
        true
    }
    async fn prepare_workflow_agent_session(
        &self,
        _workspace: &str,
        cwd: &str,
        _config: WorkflowSessionLaunchConfig,
        _tree: &str,
        node: &str,
        _instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        self.prepared
            .lock()
            .unwrap()
            .push((node.into(), cwd.into()));
        Ok(NodeSessionInfo {
            id: format!("agent-{node}"),
        })
    }
    async fn activate_workflow_agent_session(
        &self,
        _session: &str,
        node: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.activated.lock().unwrap().push(node.into());
        Ok(())
    }
    async fn confirm_workflow_agent_session_attachment(
        &self,
        _session: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }
    async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
        &self,
        _session: &str,
        _node: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }
    async fn dispatch_continuation(
        &self,
        _session: &str,
        _child: &str,
        _instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        unreachable!("adjudication has not completed")
    }
    async fn has_recoverable_conversation(
        &self,
        _session: &str,
    ) -> Result<bool, WorkflowRuntimeError> {
        unreachable!("no session recovery in this scenario")
    }
    async fn recover_workflow_agent_session_provider(
        &self,
        _session: &str,
        _node: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        unreachable!("no session recovery in this scenario")
    }
    async fn rollback_workflow_agent_session(
        &self,
        _session: &str,
        _node: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        unreachable!("no session rollback in this scenario")
    }
}
