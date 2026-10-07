//! Workflow execution host gateway.
//!
//! The domain aggregate owns lifecycle transitions and decisions, while
//! `usecase::workflow::runtime_driver` owns their application procedure and
//! transaction ordering. This gateway reads aggregates from facts, delegates
//! decisions to them, and connects event storage, agent sessions, processes,
//! and notifications.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;

pub(crate) mod activation;
pub(crate) mod approval_runtime;
pub(crate) mod command_preparation;
pub(crate) mod delegate;
pub(crate) mod execution_state;
pub(crate) mod isolated_worktree;
mod lifecycle_commands;
pub(crate) mod node_settings;
pub(crate) mod node_startup;
pub(crate) mod output_limit;
pub(crate) mod prompt_rendering;
pub(crate) mod runtime_commit;
pub(crate) mod runtime_session;

use activation::{run_runtime_activation, RuntimeActivationGate};
use command_preparation::CommandExecutionInput;

use crate::adaptor::gateway::workflow::event_log_writer as workflow_event_log_writer;
use crate::adaptor::gateway::workflow::fact_log as workflow_fact_log;
use crate::adaptor::gateway::workflow::node_session_boundary::{
    ProviderWorkflowAgentSessionPort, WorkflowAgentSessionPort, WorkflowSessionLaunchConfig,
};
use crate::adaptor::gateway::workflow::secret_source;
use crate::domain::workflow::entities::workflow_execution::{
    AppliedAdvance, LeafKind, LeafStart, NodeStart, TransitionOutcome,
};
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::reference as workflow_reference;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::ExecutionOrigin;
use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::WorkflowFacetContents;
use crate::domain::workflow::{
    ContractValidationResult, FailureClassification, NodeExecutionFailureKind,
    SchemaDef as DomainSchemaDef,
};
use crate::domain::workflow::{NodeKindName, WorkflowDefinition};
use crate::infrastructure::process::command_runner::{
    self as workflow_command_runner, CommandRunOutput, CommandRunnerError,
};
use crate::usecase::agent_session::{
    AgentSessionInitialInstructionUsecase, AgentSessionLaunchUsecase, AgentSessionLifecycleUsecase,
};
use crate::usecase::workflow::runtime_driver::{
    self as workflow_runtime_driver, NodeOutcome, PreparedWorkflowTransaction,
    WorkflowRuntimeEffect, WorkflowTransactionCommitError,
};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, WorkflowDefinitionResolver,
};
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;
use crate::usecase::workflow::runtime_start_guard as workflow_runtime_start_guard;
use execution_state::DomainExecutionTree;
use node_settings::WorkflowDefaults;
use output_limit as workflow_output_limit;
use prompt_rendering as workflow_prompt;
use runtime_commit::RequiredEventCommit;
use runtime_session as workflow_runtime_session;

#[derive(Clone)]
pub struct WorkflowRuntimeDependencies {
    pub store: Option<Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    pub config: Option<Arc<dyn crate::domain::app_config::ConfigRepository>>,
    pub secrets: Option<Arc<dyn crate::domain::app_config::ConfigSecretRepository>>,
    pub state_changes: crate::usecase::state_subscription::StateSubscriptionUsecase,
}

pub fn current_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

/// 記録から取得した Workflow 集約と usecase の駆動手順を外界へ接続する gateway host。
#[derive(Clone)]
pub struct WorkflowRuntimeHost {
    pub queue: std::sync::Arc<crate::usecase::retry::Retrying>,
    workflow_start_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    commit_locks: Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>,
    /// execution_id → 解決済み facet 本文。workflow state / event には含めない runtime-local read model。
    execution_facet_contents: Arc<Mutex<HashMap<String, WorkflowFacetContents>>>,
    /// execution_id → runtime activation serialization lock.
    ///
    /// Weak references keep session/fanout startup and stop/abort mutually exclusive without
    /// retaining one lock for every historical execution.
    runtime_activation_locks: Arc<Mutex<HashMap<String, Weak<RuntimeActivationGate>>>>,
    startup_retries: Arc<Mutex<HashMap<String, node_startup::NodeStartupTask>>>,
    /// node_execution_id → active command process shutdown handle.
    pub node_processes: Arc<super::node_process::WorkflowNodeProcesses>,
    daemon: Arc<crate::adaptor::gateway::daemon::InMemoryDaemonRepository>,
    /// node_execution_id → owning workflow execution_id.
    active_command_executions: Arc<Mutex<HashMap<String, String>>>,
    /// node_execution_id → command completion observer task owned by this workflow runtime.
    command_completion_observers: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// node_execution_id → shutdown reason consumed by the completion observer.
    command_shutdown_intents: Arc<Mutex<HashMap<String, ActiveCommandShutdownIntent>>>,
    workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
    workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
    worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
    isolated_worktrees: Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway>,
    pub delegate_continuation:
        Option<Arc<crate::usecase::workflow::delegate::DelegateContinuationUsecase>>,
}

#[derive(Clone)]
pub struct ControlPlaneCommitCandidate<'a> {
    execution_id: &'a str,
    snapshot_before: DomainExecutionTree,
    candidate: DomainExecutionTree,
    transition_outcome: TransitionOutcome,
    events: &'a [WorkflowEvent],
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
}

#[derive(Clone)]
pub struct WorkflowExecutionInsert {
    execution_id: String,
    workflow: WorkflowDefinition,
    worktree_path: String,
    request: Option<String>,
    created_from: ExecutionOrigin,
    workflow_defaults: WorkflowDefaults,
    now: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveCommandShutdownIntent {
    GracefulShutdown,
}

pub struct CommandArtifact {
    value: serde_json::Value,
    event_contract: Option<String>,
    result_summary: String,
}

pub fn command_env(
    input: &CommandExecutionInput,
    mut definition_env: Vec<(String, String)>,
) -> Vec<(String, String)> {
    definition_env.extend([
        (
            "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
            input.execution_id.clone(),
        ),
        (
            "RELEASH_NODE_EXECUTION_ID".to_string(),
            input.node_execution_id.clone(),
        ),
        (
            "RELEASH_WORKTREE_PATH".to_string(),
            input.worktree_path.clone(),
        ),
    ]);
    if let Some(session_id) = input.session_id.as_ref() {
        definition_env.push(("RELEASH_SESSION_ID".to_string(), session_id.clone()));
    }
    definition_env
}

fn new_node_execution_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn with_append_context(error: WorkflowRuntimeError, context: &str) -> WorkflowRuntimeError {
    match error {
        WorkflowRuntimeError::SessionStore(reason) => {
            WorkflowRuntimeError::SessionStore(format!("{context}: {reason}"))
        }
        WorkflowRuntimeError::Store(failure) => {
            let message = format!("{context}: {failure}");
            WorkflowRuntimeError::Store(failure.with_message(message))
        }
        other => other,
    }
}

pub fn build_command_artifact(
    schemas: &BTreeMap<String, DomainSchemaDef>,
    contract: Option<&str>,
    output: CommandRunOutput,
    secrets: &[String],
) -> CommandArtifact {
    let stdout = workflow_output_limit::truncate_output(
        workflow_secret_masker::mask_sensitive_text(&output.stdout, secrets),
    );
    let stderr = workflow_output_limit::truncate_output(
        workflow_secret_masker::mask_sensitive_text(&output.stderr, secrets),
    );
    let mut object = serde_json::Map::new();
    object.insert(
        "exit_code".to_string(),
        serde_json::Value::Number(output.exit_code.into()),
    );
    object.insert(
        "stdout".to_string(),
        serde_json::Value::String(stdout.clone()),
    );
    object.insert("stderr".to_string(), serde_json::Value::String(stderr));
    object.insert(
        "duration".to_string(),
        serde_json::Value::Number(output.duration_ms.into()),
    );

    let mut validation_success = contract.is_none();
    let mut event_contract = None;
    if let Some(contract) = contract {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output.stdout) {
            let parsed = workflow_secret_masker::mask_sensitive_artifact(contract, parsed, secrets);
            if let ContractValidationResult::Valid { artifact, .. } =
                workflow_contract::validate_artifact_value(schemas, contract, parsed)
            {
                if let Some(fields) = artifact.as_object() {
                    for (field, value) in fields {
                        object.insert(field.clone(), value.clone());
                    }
                    validation_success = true;
                    event_contract = Some(contract.to_string());
                }
            }
        }
    }

    if let Some(contract) = contract {
        let value = serde_json::Value::Object(std::mem::take(&mut object));
        let masked = workflow_secret_masker::mask_sensitive_artifact(contract, value, secrets);
        if let serde_json::Value::Object(masked_object) = masked {
            object = masked_object;
        }
    }
    for value in object.values_mut() {
        workflow_secret_masker::mask_json_strings(value, secrets);
    }

    let ok = output.exit_code == 0 && validation_success;
    object.insert("ok".to_string(), serde_json::Value::Bool(ok));
    CommandArtifact {
        value: serde_json::Value::Object(object),
        event_contract,
        result_summary: format!("exit_code={}", output.exit_code),
    }
}

pub async fn retry_runtime_conflicts<T, F, Fut>(
    queue: &std::sync::Arc<crate::usecase::retry::Retrying>,
    target: &str,
    operation: F,
) -> Result<T, WorkflowRuntimeError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, WorkflowRuntimeError>>,
{
    crate::usecase::workflow::command::retry_control_plane_operation(queue, target, operation).await
}

impl WorkflowRuntimeHost {
    pub async fn load_control_plane_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<Option<DomainExecutionTree>, WorkflowRuntimeError> {
        Ok(Self::load_execution_revision(app, execution_id)
            .await?
            .map(|(execution, _)| execution))
    }

    pub async fn load_execution_revision(
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<Option<(DomainExecutionTree, i64)>, WorkflowRuntimeError> {
        use crate::adaptor::gateway::local_event_store::node_events;
        let store = app.store.as_ref().ok_or_else(|| {
            WorkflowRuntimeError::SessionStore(
                "workflow SQLite event authority is not managed".into(),
            )
        })?;
        let tree_id = execution_id.to_string();
        let rows = store
            .submit_query(move |connection| {
                node_events::read_tree(connection, &tree_id).map_err(|error| {
                    crate::adaptor::gateway::local_event_store::reader::storage_unavailable(&error)
                })
            })
            .await
            .map_err(|error| {
                let message = format!("tree read failed: {error:?}");
                WorkflowRuntimeError::storage(error, message)
            })?;
        let head = rows.last().map_or(0, |row| row.seq);
        let records = workflow_fact_log::records_from_tree_rows(&rows)
            .map_err(WorkflowRuntimeError::SessionStore)?;
        Ok(
            crate::domain::workflow::services::fact_replay::fold_execution_tree(
                execution_id,
                &records,
            )
            .map_err(WorkflowRuntimeError::SessionStore)?
            .map(|folded| (folded.aggregate, head)),
        )
    }

    pub async fn load_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<DomainExecutionTree, WorkflowRuntimeError> {
        self.load_control_plane_execution(app, execution_id)
            .await?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn load_executions(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<HashMap<String, DomainExecutionTree>, WorkflowRuntimeError> {
        Ok(self
            .load_control_plane_execution(app, execution_id)
            .await?
            .map(|execution| (execution_id.into(), execution))
            .into_iter()
            .collect())
    }

    pub async fn append_events_at_head(
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        head: i64,
        events: &[WorkflowEvent],
    ) -> Result<(), WorkflowRuntimeError> {
        use crate::domain::local_event::CommitBatchError;
        let store = app.store.as_ref().ok_or_else(|| {
            WorkflowRuntimeError::SessionStore(
                "workflow SQLite event authority is not managed".into(),
            )
        })?;
        let rows = workflow_fact_log::pending_rows_for_events(store, events)
            .await
            .map_err(|error| {
                let message = error.to_string();
                WorkflowRuntimeError::storage(error, message)
            })?;
        let result = store
            .append_node_events_at_head(
                rows.iter()
                    .map(|row| (row.row.clone(), Some(row.timestamp_ms)))
                    .collect(),
                Some((execution_id.into(), head)),
            )
            .await;
        match result {
            Err(CommitBatchError::AppendOutcomeUnknown) => {
                workflow_fact_log::resolve_unknown_append(store, rows, Some(head))
                    .await
                    .map_err(|error| {
                        let message = format!("control-plane commit readback failed: {error:?}");
                        WorkflowRuntimeError::storage(error, message)
                    })?
                    .map(|_| ())
            }
            result => result.map(|_| ()),
        }
        .map_err(|error| match error {
            CommitBatchError::TreeHeadConflict => WorkflowRuntimeError::Conflict(format!(
                "execution '{execution_id}' changed before commit"
            )),
            other => {
                let message = other.to_string();
                WorkflowRuntimeError::storage(other, message)
            }
        })
    }

    pub async fn register_started_execution_tree(
        &self,
        app: &WorkflowRuntimeDependencies,
        tree_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let execution = self
            .load_control_plane_execution(app, tree_id)
            .await?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(tree_id.into()))?;
        if !execution.is_active() {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution tree '{tree_id}' is not active"
            )));
        }
        Ok(())
    }

    pub(crate) async fn commit_workflow_control_plane(
        &self,
        app: &WorkflowRuntimeDependencies,
        commit: crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        let activation_gate = if commit
            .workflow_events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::NodeRetryRequested { .. }))
        {
            Some(self.runtime_activation_gate(&commit.execution_id).await)
        } else {
            None
        };
        let _activation_guard = match &activation_gate {
            Some(gate) => Some(gate.lock.lock().await),
            None => None,
        };
        self.commit_control_plane_candidate(
            app,
            ControlPlaneCommitCandidate {
                execution_id: &commit.execution_id,
                snapshot_before: commit.before,
                candidate: commit.after,
                transition_outcome: commit.transition_outcome,
                events: &commit.workflow_events,
                provider_events: commit.provider_events,
            },
        )
        .await
    }

    pub(crate) async fn finish_workflow_control_plane_commit(
        &self,
        app: &WorkflowRuntimeDependencies,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowRuntimeError> {
        self.finish_control_plane_commit(app, worktree_path, snapshot, outcome)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_canonical(
        queue: std::sync::Arc<crate::usecase::retry::Retrying>,
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        agent_session_launch: Arc<AgentSessionLaunchUsecase>,
        agent_session_initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
        agent_session_lifecycle: Arc<AgentSessionLifecycleUsecase>,
        provider_availability: Arc<dyn crate::domain::agent_session::ProviderAvailabilityReader>,
        isolated_worktrees: Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway>,
        daemon: Arc<crate::adaptor::gateway::daemon::InMemoryDaemonRepository>,
    ) -> Self {
        Self::with_runtime_ports(
            queue,
            workflow_resolver,
            worktree_resolver,
            workspace_query,
            Arc::new(ProviderWorkflowAgentSessionPort::new(
                agent_session_launch,
                agent_session_initial_instruction,
                agent_session_lifecycle,
                provider_availability,
            )),
            isolated_worktrees,
            daemon,
        )
    }

    pub fn with_runtime_ports(
        queue: std::sync::Arc<crate::usecase::retry::Retrying>,
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
        isolated_worktrees: Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway>,
        daemon: Arc<crate::adaptor::gateway::daemon::InMemoryDaemonRepository>,
    ) -> Self {
        Self {
            queue,
            workflow_start_locks: Arc::new(Mutex::new(HashMap::new())),
            commit_locks: Arc::new(Mutex::new(HashMap::new())),
            execution_facet_contents: Arc::new(Mutex::new(HashMap::new())),
            runtime_activation_locks: Arc::new(Mutex::new(HashMap::new())),
            startup_retries: Arc::new(Mutex::new(HashMap::new())),
            node_processes: Arc::new(Default::default()),
            daemon,
            active_command_executions: Arc::new(Mutex::new(HashMap::new())),
            command_completion_observers: Arc::new(Mutex::new(HashMap::new())),
            command_shutdown_intents: Arc::new(Mutex::new(HashMap::new())),
            workspace_query,
            workflow_resolver,
            worktree_resolver,
            workflow_agent_sessions,
            isolated_worktrees,
            delegate_continuation: None,
        }
    }

    pub async fn workflow_start_lock(&self, worktree_path: &str) -> Arc<Mutex<()>> {
        let identity = crate::domain::workspace_tree::WorkspaceIdentity::new(worktree_path);
        let mut locks = self.workflow_start_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(identity.as_str()).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(identity.as_str().to_string(), Arc::downgrade(&lock));
        lock
    }

    pub async fn commit_lock(&self, execution_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.commit_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(execution_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(execution_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    pub async fn runtime_activation_gate(&self, execution_id: &str) -> Arc<RuntimeActivationGate> {
        let mut locks = self.runtime_activation_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(execution_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(RuntimeActivationGate::new());
        locks.insert(execution_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    fn ensure_workflow_providers_available(
        &self,
        workflow: &WorkflowDefinition,
    ) -> Result<(), WorkflowRuntimeError> {
        for node in &workflow.nodes {
            let Some(session) = node.session() else {
                continue;
            };
            if !self
                .workflow_agent_sessions
                .is_provider_available(session.provider)
            {
                let provider = match session.provider {
                    crate::domain::provider_lifecycle::ProviderKind::Claude => "claude",
                    crate::domain::provider_lifecycle::ProviderKind::Codex => "codex",
                };
                return Err(WorkflowRuntimeError::AgentSession(format!(
                    "Provider '{provider}' configured for Session Node '{}' is unavailable",
                    node.name
                )));
            }
        }
        Ok(())
    }

    fn resolve_facet_contents_for_workflow(
        workflow: &WorkflowDefinition,
    ) -> Result<WorkflowFacetContents, WorkflowRuntimeError> {
        crate::adaptor::gateway::workflow::storage::resolve_and_validate_workflow_facets(
            workflow,
            &crate::adaptor::gateway::workflow::facet::facets_base_dir(),
        )
        .map_err(|e| WorkflowRuntimeError::InvalidWorkflow(e.to_string()))
    }

    async fn facet_contents_for_execution(
        &self,
        execution_id: &str,
        workflow: &WorkflowDefinition,
    ) -> Result<WorkflowFacetContents, WorkflowRuntimeError> {
        if let Some(contents) = self
            .execution_facet_contents
            .lock()
            .await
            .get(execution_id)
            .cloned()
        {
            return Ok(contents);
        }
        let contents = Self::resolve_facet_contents_for_workflow(workflow)?;
        self.execution_facet_contents
            .lock()
            .await
            .insert(execution_id.to_string(), contents.clone());
        Ok(contents)
    }

    pub async fn insert_workflow_execution(
        &self,
        input: WorkflowExecutionInsert,
    ) -> Result<(RuntimeCommitSnapshot, AppliedAdvance), WorkflowRuntimeError> {
        let WorkflowExecutionInsert {
            execution_id,
            workflow,
            worktree_path,
            request,
            created_from,
            workflow_defaults,
            now,
        } = input;
        let execution_id = crate::domain::workflow::WorkflowExecutionId::new(execution_id)
            .map_err(|error| WorkflowRuntimeError::ValidationError(error.to_string()))?
            .as_str()
            .to_string();
        let repository_root = Some(
            self.isolated_worktrees
                .repository_root(&worktree_path)
                .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?,
        );
        let mut execution = crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: execution_id.clone(),
            workflow: workflow.clone(),
            state: RuntimeExecutionState::Running,
            node_history: Vec::new(),
            workflow_defaults,
            created_from,
            error_reason: None,
            started_at: now,
            updated_at: now,
            current_session_id: None,
            scopes: Vec::new(),
            node_executions: Vec::new(),
            request,
            current_stall_observations: Vec::new(),
            workspace_identity: crate::domain::workspace_tree::WorkspaceIdentity::new(&worktree_path)
                .as_str()
                .to_string(),
            worktree_path: worktree_path.clone(),
            launched_as: crate::domain::workflow::ExecutionTreeLaunch::Workflow,
            repository_root,
        };

        // 実行木の起動カスケード: root（合成子なら実効 entry の leaf まで）を開始する。
        let mut new_id = new_node_execution_id;
        let applied = execution
            .start_root(&mut new_id, now)
            .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?;
        let snapshot = RuntimeCommitSnapshot::from_execution(&execution)?;
        Ok((snapshot, applied))
    }

    pub(crate) async fn reconcile_tree(
        &self,
        app: &WorkflowRuntimeDependencies,
        tree_id: &str,
        now: f64,
    ) -> Result<(), WorkflowRuntimeError> {
        let store = app.store.as_ref().ok_or_else(|| {
            WorkflowRuntimeError::SessionStore("canonical store is not configured".into())
        })?;
        let activation_gate = self.runtime_activation_gate(tree_id).await;
        let activation_guard = activation_gate.lock.lock().await;
        let mut new_id = new_node_execution_id;
        let Some(reconciliation) =
            workflow_fact_log::reconcile_tree_pass(store, tree_id, now, &mut new_id)
                .await
                .map_err(|error| match error {
                    crate::domain::workflow::WorkflowError::Conflict(reason) => {
                        WorkflowRuntimeError::Conflict(reason)
                    }
                    error => {
                        let message = error.to_string();
                        WorkflowRuntimeError::storage(error, message)
                    }
                })?
        else {
            return Ok(());
        };
        let folded = reconciliation.folded;
        if !folded.aggregate.is_active() {
            return Ok(());
        }
        let worktree_path = folded.aggregate.worktree_path.clone();
        drop(activation_guard);
        if !reconciliation.starts.is_empty() {
            self.start_nodes(app, tree_id, &worktree_path, reconciliation.starts)
                .await?;
        }
        Ok(())
    }
}

impl WorkflowRuntimeHost {
    /// ワークフローを開始する。
    /// ChatSessionは既に作成済みの前提で、最初のステップのプロンプトを送信する。
    ///
    /// 戻り値は新しく払い出された `execution_id`。
    /// `execution_id` を `execution_id` として「昇格」させた値であり、ここ以外で採番されることはない。
    /// state 変化の入口は resolved StartExecution port からこの private handler に合流する。
    /// 外部入口としては公開せず、usecase/gateway が解決済み workflow を渡す境界にする。
    async fn start_workflow(
        &self,
        app: &WorkflowRuntimeDependencies,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
    ) -> Result<String, WorkflowRuntimeError> {
        let worktree_path = crate::domain::workspace_tree::WorkspaceIdentity::new(worktree_path)
            .as_str()
            .to_string();
        // ===== Phase 1: 副作用なしの validation =====
        // parent ChatSession 作成・executions 登録・refs 登録の前で全 validation を実施する。
        // ここで弾けば、リトライ時に「孤立した parent session」「孤立した refs entry」
        // を残さない（Spec issues-1011: 起動順序のアトミック化）。
        //
        // 1) workflow 構造の事前検証（空 nodes などの実行不能形状の拒否）。
        workflow_runtime_start_guard::validate_workflow_shape(&workflow)?;
        self.ensure_workflow_providers_available(&workflow)?;
        let facet_contents = Self::resolve_facet_contents_for_workflow(&workflow)?;

        let start_lock = self.workflow_start_lock(&worktree_path).await;
        let start_guard = start_lock.lock().await;
        let candidates =
            crate::usecase::workspace_tree::WorkspaceQueryService::execution_summaries(
                self.workspace_query.as_ref(),
                Some(&crate::domain::workspace_tree::WorkspaceIdentity::new(
                    &worktree_path,
                )),
                Some(crate::domain::workflow::ExecutionStatusFilter::Active),
            )
            .await
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        workflow_runtime_start_guard::validate_start(&worktree_path, &candidates)?;
        let now = current_timestamp();
        let execution_id = uuid::Uuid::new_v4().to_string();
        self.execution_facet_contents
            .lock()
            .await
            .insert(execution_id.clone(), facet_contents);

        let workflow_defaults = WorkflowDefaults;
        let snapshot_result = self
            .insert_workflow_execution(WorkflowExecutionInsert {
                execution_id: execution_id.clone(),
                workflow: workflow.clone(),
                worktree_path: worktree_path.clone(),
                request: request.clone(),
                created_from,
                workflow_defaults,
                now,
            })
            .await;
        let (snapshot, applied) = match snapshot_result {
            Ok(s) => s,
            Err(e) => {
                self.release_execution_facet_contents(&execution_id).await;
                return Err(e);
            }
        };

        // [04] commit point: ExecutionStarted と起動カスケードの NodeStarted 群を
        // 同一の required batch で append する。
        let mut required_start_events = vec![WorkflowEvent::ExecutionStarted {
            repository_root: snapshot.repository_root.clone(),
            execution_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            worktree_path: worktree_path.clone(),
            created_from,
            request: request.clone().unwrap_or_default(),
            definition: workflow.clone(),
            timestamp: now,
        }];
        required_start_events.extend(applied.events);
        if let Err(e) = self
            .write_log_required_batch(app, &required_start_events)
            .await
        {
            self.release_execution_facet_contents(&execution_id).await;
            return Err({
                let message = format!("write initial workflow event batch failed: {e}");
                WorkflowRuntimeError::storage(e, message)
            });
        }

        drop(start_guard);
        // [04] post-commit: broadcast。ExecutionStarted は append 済みのため command は既に受理。
        workflow_runtime_session::broadcast_state(app, &worktree_path).await;

        // [04] post-commit: ExecutionStarted append 済みのため start primitive は既に受理。
        //    初回 runtime 起動失敗は事実として記録し、
        //    start primitive は Ok(execution_id) を返す（spec [04]『command 受理境界』Rule）。
        if let crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision::StartNodes(leaves) =
            applied.decision
        {
            if let Err(e) = self
                .start_nodes(app, &execution_id, &worktree_path, leaves)
                .await
            {
                if let Err(settle_error) = self
                    .settle_runtime_failure(app, &execution_id, &e)
                    .await
                {
                    log::error!(
                        "workflow {execution_id}: runtime start failed and NodeFailed settlement also failed: {settle_error}"
                    );
                }
                log::warn!("workflow {execution_id}: post-commit node runtime start failed: {e}");
            }
        }
        Ok(execution_id)
    }

    pub(crate) async fn resolve_start_execution_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowRuntimeError> {
        self.worktree_resolver
            .resolve(worktree_path)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn resolve_start_execution_workflow(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowRuntimeError> {
        crate::domain::workflow::validation::validate_name(workflow_name)
            .map_err(|e| WorkflowRuntimeError::ValidationError(format!("validation_error: {e}")))?;
        self.workflow_resolver
            .resolve(workflow_name)
            .await
            .map_err(Into::into)
    }

    pub async fn start_resolved_workflow(
        &self,
        app: &WorkflowRuntimeDependencies,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
    ) -> Result<String, WorkflowRuntimeError> {
        Box::pin(self.start_workflow(app, workflow, worktree_path, request, created_from)).await
    }

    pub async fn commit_control_plane_candidate(
        &self,
        app: &WorkflowRuntimeDependencies,
        commit: ControlPlaneCommitCandidate<'_>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        self.queue
            .stage(
                crate::usecase::failure::FailureKey::new("workflow_runtime", commit.execution_id),
                crate::common::retry::RetryBackoff::CONFLICT,
                |_| self.commit_control_plane_candidate_once(app, commit.clone()),
            )
            .await
    }

    async fn commit_control_plane_candidate_once(
        &self,
        app: &WorkflowRuntimeDependencies,
        commit: ControlPlaneCommitCandidate<'_>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        let commit_lock = self.commit_lock(commit.execution_id).await;
        let _commit_guard = commit_lock.lock().await;
        self.commit_control_plane_candidate_locked(app, commit)
            .await
    }

    async fn commit_admitted_command_candidate(
        &self,
        app: &WorkflowRuntimeDependencies,
        admission: &crate::adaptor::gateway::daemon::DaemonAdmissionGuard<'_>,
        commit: ControlPlaneCommitCandidate<'_>,
    ) -> Result<Option<RuntimeCommitSnapshot>, WorkflowRuntimeError> {
        self.queue
            .stage(
                crate::usecase::failure::FailureKey::new("workflow_runtime", commit.execution_id),
                crate::common::retry::RetryBackoff::CONFLICT,
                |_| async {
                    let commit_lock = self.commit_lock(commit.execution_id).await;
                    let _commit_guard = commit_lock.lock().await;
                    if !admission.admits(crate::domain::daemon::DaemonRequest::Operation) {
                        return Ok(None);
                    }
                    self.commit_control_plane_candidate_locked(app, commit.clone())
                        .await
                        .map(Some)
                },
            )
            .await
    }

    async fn commit_control_plane_candidate_locked(
        &self,
        app: &WorkflowRuntimeDependencies,
        commit: ControlPlaneCommitCandidate<'_>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        let ControlPlaneCommitCandidate {
            execution_id,
            snapshot_before,
            candidate,
            transition_outcome,
            events,
            provider_events,
        } = commit;
        let snapshot = RuntimeCommitSnapshot::from_execution(&candidate)?;
        let transaction = PreparedWorkflowTransaction::capture_with_outcome(
            snapshot_before,
            candidate,
            transition_outcome,
            events.to_vec(),
            vec![WorkflowRuntimeEffect::BroadcastState],
        )
        .map_err(|error| {
            WorkflowRuntimeError::InvalidState(format!(
                "invalid control-plane transaction preparation: {error:?}"
            ))
        })?;
        let (mut current, head) = Self::load_execution_revision(app, execution_id)
            .await?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))?;
        for event in events {
            if let WorkflowEvent::NodeRetryRequested {
                node_execution_id, ..
            } = event
            {
                let node = current.node_execution(node_execution_id).ok_or_else(|| {
                    WorkflowRuntimeError::Conflict("Node attempt no longer exists".into())
                })?;
                let presence = crate::domain::workflow::NodeProcessReader::presence(
                    self.node_processes.as_ref(),
                    &current.workspace_identity,
                    node_execution_id,
                    node.kind,
                    node.session_id.as_deref(),
                )
                .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?;
                if presence != crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent {
                    return Err(WorkflowRuntimeError::InvalidState(
                        "Node process must be absent before a new attempt can be committed".into(),
                    ));
                }
            }
        }
        let durable = transaction
            .persist(&mut current, |events| async move {
                Self::append_events_at_head(app, execution_id, head, &events).await
            })
            .await
            .map_err(|error| match error {
                WorkflowTransactionCommitError::StaleCandidate => WorkflowRuntimeError::Conflict(
                    format!("execution '{execution_id}' changed before control-plane commit"),
                ),
                WorkflowTransactionCommitError::Persistence(error) => error,
            })?;
        if !provider_events.is_empty() {
            if let Err(error) =
                workflow_event_log_writer::commit_provider_stop_for_app(app, provider_events).await
            {
                log::warn!("workflow provider Stop facts committed but provider lifecycle commit failed: {error}");
            }
        }
        self.spawn_committed_runtime_effects(durable.into_effects());
        Ok(snapshot)
    }

    /// durable commit 済みの runtime effect を detached task で実行する。
    ///
    /// provider Stop 受理経路は同一 AgentSession の operation lock と provider
    /// lifecycle slot lock を保持したまま commit するため、Session 停止 effect を
    /// 同じ call stack で実行すると lock を再取得して deadlock する。
    fn spawn_committed_runtime_effects(&self, effects: Vec<WorkflowRuntimeEffect>) {
        let stops: Vec<WorkflowRuntimeEffect> = effects
            .into_iter()
            .filter(|effect| {
                matches!(
                    effect,
                    WorkflowRuntimeEffect::StopWorkflowAgentSession { .. }
                )
            })
            .collect();
        if stops.is_empty() {
            return;
        }
        let sessions = self.workflow_agent_sessions.clone();
        tokio::spawn(async move {
            Self::run_committed_runtime_effects(sessions, stops).await;
        });
    }

    pub async fn run_committed_runtime_effects(
        sessions: Arc<dyn WorkflowAgentSessionPort>,
        effects: Vec<WorkflowRuntimeEffect>,
    ) {
        for effect in effects {
            let WorkflowRuntimeEffect::StopWorkflowAgentSession {
                node_execution_id,
                agent_session_id,
            } = effect
            else {
                continue;
            };
            if let Err(error) = sessions
                .stop_agent_session_for_terminal_node_preserving_checkpoint(
                    &agent_session_id,
                    &node_execution_id,
                )
                .await
            {
                log::warn!(
                    "workflow NodeExecution '{node_execution_id}': failed to stop AgentSession '{agent_session_id}' after durable terminal transition: {error}"
                );
            }
        }
    }

    async fn finish_control_plane_commit(
        &self,
        app: &WorkflowRuntimeDependencies,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowRuntimeError> {
        if let Some(outcome) = outcome {
            self.finalize_after_commit(app, snapshot, worktree_path)
                .await;
            self.dispatch_node_outcome_side_effects(app, worktree_path, outcome)
                .await
        } else {
            workflow_runtime_session::broadcast_state(app, worktree_path).await;
            Ok(())
        }
    }

    pub async fn release_deleted_execution_tree(
        &self,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.release_execution_facet_contents(execution_id).await;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn get_state_by_execution_id(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Option<RuntimeCommitSnapshot> {
        self.load_control_plane_execution(app, execution_id)
            .await
            .unwrap()
            .and_then(|execution| RuntimeCommitSnapshot::from_execution(&execution).ok())
    }

    #[cfg(feature = "test-support")]
    pub(crate) async fn acceptance_state_by_execution_id(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<Option<crate::domain::workflow::WorkflowRuntimeSnapshot>, WorkflowRuntimeError>
    {
        self.load_control_plane_execution(app, execution_id).await?
            .filter(DomainExecutionTree::is_active)
            .map(|execution| RuntimeCommitSnapshot::from_execution(&execution))
            .transpose()
            .map(|snapshot| snapshot.map(crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot))
    }

    async fn release_execution_facet_contents(&self, execution_id: &str) {
        self.execution_facet_contents
            .lock()
            .await
            .remove(execution_id);
    }

    async fn release_terminal_execution(&self, execution_id: &str) {
        self.release_execution_facet_contents(execution_id).await;
    }

    // ---- 内部メソッド ----

    /// advance が返した Node を準備して起動する。Session はまとめて prepare →
    /// SessionAttached を一括 commit → activate、Command は spawn する。
    pub async fn start_nodes(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        worktree_path: &str,
        starts: Vec<NodeStart>,
    ) -> Result<(), WorkflowRuntimeError> {
        let failed = self
            .start_nodes_once(app, execution_id, worktree_path, starts)
            .await?;
        self.schedule_startup_retries(app, execution_id, worktree_path, failed)
            .await;
        Ok(())
    }

    async fn start_nodes_once(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        worktree_path: &str,
        starts: Vec<NodeStart>,
    ) -> Result<Vec<crate::usecase::workflow::node_startup::FailedNodeStart>, WorkflowRuntimeError>
    {
        if starts.is_empty() {
            return Ok(Vec::new());
        }
        let (preparations, mut injections) = isolated_worktree::partition_actions(starts);
        let prepared = self
            .prepare_isolated_starts(app, execution_id, worktree_path, preparations)
            .await?;
        let mut failed = prepared.failed;
        injections.extend(prepared.injections);
        for injection in injections {
            self.inject_delegate_result(
                app,
                execution_id,
                &injection,
                delegate::DelegateInjectionOrigin::Automatic,
            )
            .await?;
        }
        let leaves = prepared.leaves;
        if leaves.is_empty() {
            return Ok(failed);
        }
        let (workflow, attempts_by_id) = {
            let loaded = self.load_execution(app, execution_id).await?;
            let exec = &loaded;
            let attempts_by_id: HashMap<String, u32> = exec
                .node_executions
                .iter()
                .map(|execution| (execution.id.clone(), execution.attempt))
                .collect();
            (
                exec.workflow_definition()
                    .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?
                    .clone(),
                attempts_by_id,
            )
        };
        let execution_id = execution_id.to_string();
        let activation_gate = self.runtime_activation_gate(&execution_id).await;
        let activation_guard = activation_gate.lock.lock().await;
        let facet_contents = self
            .facet_contents_for_execution(&execution_id, &workflow)
            .await?;

        let mut command_inputs = Vec::new();
        let mut session_plans = Vec::new();
        for leaf in leaves {
            let node = workflow
                .node_by_name(&leaf.node_name)
                .cloned()
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidWorkflow(format!(
                        "node '{}' is undefined",
                        leaf.node_name
                    ))
                })?;
            let execution_worktree_path = {
                let Some(loaded) = self
                    .load_control_plane_execution(app, &execution_id)
                    .await?
                else {
                    continue;
                };
                let execution = &loaded;
                if !execution
                    .node_execution(&leaf.node_execution_id)
                    .is_some_and(|node| node.can_start_process())
                {
                    continue;
                }
                execution
                    .execution_worktree_path(&leaf.node_execution_id)
                    .map(str::to_string)
                    .ok_or_else(|| {
                        WorkflowRuntimeError::InvalidState(
                            "execution worktree is unavailable".to_string(),
                        )
                    })?
            };
            match leaf.kind {
                LeafKind::Command => {
                    let command = node.command_spec().ok_or_else(|| {
                        WorkflowRuntimeError::InvalidState(format!(
                            "node '{}' is not a command",
                            leaf.node_name
                        ))
                    })?;
                    let rendered = workflow_prompt::render_parameter_references(
                        &command.command,
                        &leaf.bindings,
                    );
                    match workflow_reference::resolve_command_environment(
                        &command.env,
                        &leaf.bindings,
                    ) {
                        Ok(definition_env) => command_inputs.push(Ok(CommandExecutionInput {
                            execution_id: execution_id.clone(),
                            node_execution_id: leaf.node_execution_id.clone(),
                            node_name: leaf.node_name.clone(),
                            attempt: attempts_by_id
                                .get(&leaf.node_execution_id)
                                .copied()
                                .unwrap_or(1),
                            worktree_path: execution_worktree_path,
                            raw_command: Some(rendered),
                            definition_env,
                            contract: node.artifact.clone(),
                            schemas: workflow.schemas.clone(),
                            session_id: None,
                        })),
                        Err(error) => {
                            command_inputs.push(Err((leaf.node_execution_id.clone(), error)))
                        }
                    }
                }
                LeafKind::Session => {
                    let (system_prompt, user_message) = workflow_prompt::build_leaf_prompt(
                        &node,
                        facet_contents.for_node(&node.name),
                        &leaf.node_execution_id,
                        &leaf.bindings,
                        &workflow.schemas,
                    )?;
                    let initial_instruction =
                        crate::domain::workflow::services::prompt_composition::provider_tui_initial_instruction(
                            system_prompt.as_deref(),
                            &user_message,
                        );
                    let launch_config = node
                        .session()
                        .map(WorkflowSessionLaunchConfig::from_session_spec)
                        .ok_or_else(|| {
                            WorkflowRuntimeError::InvalidWorkflow(format!(
                                "Node '{}' is not a Session Node",
                                node.name
                            ))
                        })?;
                    session_plans.push((
                        leaf.node_execution_id.clone(),
                        execution_worktree_path,
                        launch_config,
                        initial_instruction,
                    ));
                }
            }
        }

        // Session を先に全 prepare し、SessionAttached を一括 commit してから activate する。
        let mut session_setups: Vec<(String, String)> = Vec::with_capacity(session_plans.len());
        let mut session_failures = Vec::new();
        for (node_execution_id, execution_worktree_path, launch_config, initial_instruction) in
            session_plans
        {
            let prepared = self
                .workflow_agent_sessions
                .prepare_workflow_agent_session(
                    worktree_path,
                    &execution_worktree_path,
                    launch_config,
                    &execution_id,
                    &node_execution_id,
                    &initial_instruction,
                )
                .await;
            match prepared {
                Ok(session) => {
                    session_setups.push((node_execution_id, session.id));
                }
                Err(launch_error) => {
                    session_failures.push((node_execution_id, launch_error));
                }
            }
        }
        if !session_setups.is_empty() {
            let timestamp = current_timestamp();
            let commit_result: Result<RuntimeCommitSnapshot, WorkflowRuntimeError> = retry_runtime_conflicts(&self.queue, &execution_id, || async {
                    let snapshot_before = self.load_execution(app, &execution_id).await?;
                    let mut candidate = snapshot_before.clone();
                    let mut events = Vec::new();
                    for (node_execution_id, session_id) in &session_setups {
                        if !candidate
                            .node_execution(node_execution_id)
                            .is_some_and(|node| node.can_start_process())
                            || candidate.attach_node_session(
                                node_execution_id,
                                session_id.clone(),
                                timestamp,
                            ) != TransitionOutcome::Applied
                        {
                            return Err(WorkflowRuntimeError::InvalidState(format!(
                                "NodeExecution '{node_execution_id}' does not admit AgentSession attachment"
                            )));
                        }
                        events.push(WorkflowEvent::SessionAttached {
                            execution_id: execution_id.clone(),
                            node_execution_id: node_execution_id.clone(),
                            session_id: session_id.clone(),
                            timestamp,
                        });
                    }
                    self
                        .commit_required_events(
                            app,
                            RequiredEventCommit {
                                execution_id: &execution_id,
                                snapshot_before,
                                candidate,
                                required_events: events,
                                append_error_context: "session attachment event append failed",
                            },
                        )
                        .await
            })
            .await;
            match commit_result {
                Ok(_) => (),
                Err(error) => {
                    return match self.rollback_prepared_sessions(&session_setups).await {
                        Some(rollback_error) => Err(WorkflowRuntimeError::AgentSession(format!(
                            "{error}; rollback failed: {rollback_error}"
                        ))),
                        None => Err(error),
                    };
                }
            };
            workflow_runtime_session::broadcast_state(app, worktree_path).await;
        }
        let mut activated_sessions = Vec::with_capacity(session_setups.len());
        for (node_execution_id, session_id) in &session_setups {
            match run_runtime_activation(
                &activation_gate,
                &execution_id,
                "session",
                self.workflow_agent_sessions
                    .activate_workflow_agent_session(session_id, node_execution_id),
            )
            .await
            {
                Ok(()) => activated_sessions.push((node_execution_id.clone(), session_id.clone())),
                Err(error) => {
                    log::warn!(
                        "workflow {execution_id}: NodeExecution '{node_execution_id}' failed to activate: {error}"
                    );
                    session_failures.push((node_execution_id.clone(), error));
                }
            }
        }
        for (_, session_id) in &activated_sessions {
            if let Err(error) = self
                .workflow_agent_sessions
                .confirm_workflow_agent_session_attachment(session_id)
                .await
            {
                log::warn!(
                    "workflow {execution_id}: failed to release attached AgentSession '{session_id}' launch state: {error}"
                );
            }
        }
        drop(activation_guard);
        drop(activation_gate);
        for (node_execution_id, _) in activated_sessions {
            let injection = self
                .load_control_plane_execution(app, &execution_id)
                .await?
                .and_then(|execution| execution.pending_delegate_injection(&node_execution_id));
            if let Some(injection) = injection {
                self.inject_delegate_result(
                    app,
                    &execution_id,
                    &injection,
                    delegate::DelegateInjectionOrigin::Automatic,
                )
                .await?;
            }
        }
        for (node_execution_id, error) in session_failures {
            self.record_node_start_failure(
                app,
                &execution_id,
                &node_execution_id,
                &error,
                &mut failed,
            )
            .await?;
        }
        for input in command_inputs {
            let (node_execution_id, result) = match input {
                Ok(input) => {
                    let node_execution_id = input.node_execution_id.clone();
                    let result = self.spawn_command_execution(app, input).await;
                    (node_execution_id, result)
                }
                Err((node_execution_id, error)) => (
                    node_execution_id,
                    Err(WorkflowRuntimeError::SessionStore(format!(
                        "failed to prepare command environment: {error}"
                    ))),
                ),
            };
            if let Err(error) = result {
                self.record_node_start_failure(
                    app,
                    &execution_id,
                    &node_execution_id,
                    &error,
                    &mut failed,
                )
                .await?;
                log::warn!(
                    "workflow {execution_id}: Command NodeExecution '{node_execution_id}' failed to activate: {error}"
                );
            }
        }
        Ok(failed)
    }

    pub async fn record_node_start_failure(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        error: &WorkflowRuntimeError,
        failed: &mut Vec<crate::usecase::workflow::node_startup::FailedNodeStart>,
    ) -> Result<(), WorkflowRuntimeError> {
        let kind = crate::usecase::failure::Failure::from(error);
        if crate::usecase::failure::next_attempt(kind).is_some() {
            failed.push(crate::usecase::workflow::node_startup::FailedNodeStart {
                id: node_execution_id.into(),
                kind,
            });
        }
        if error.version_conflict().is_none() {
            Box::pin(self.settle_runtime_failure_for_node(
                app,
                execution_id,
                node_execution_id,
                error,
            ))
            .await?;
        }
        Ok(())
    }

    async fn rollback_prepared_sessions(
        &self,
        session_setups: &[(String, String)],
    ) -> Option<WorkflowRuntimeError> {
        let mut rollback_failure = None;
        for (node_execution_id, session_id) in session_setups {
            if let Err(error) = self
                .workflow_agent_sessions
                .rollback_workflow_agent_session(session_id, node_execution_id)
                .await
            {
                rollback_failure.get_or_insert(error);
            }
        }
        rollback_failure
    }

    pub fn spawn_command_execution<'a>(
        &'a self,
        app: &'a WorkflowRuntimeDependencies,
        mut input: CommandExecutionInput,
    ) -> futures_util::future::BoxFuture<'a, Result<(), WorkflowRuntimeError>> {
        Box::pin(async move {
            let command_admission = self.daemon.admission().await;
            if !command_admission.admits(crate::domain::daemon::DaemonRequest::Operation) {
                log::warn!(
                    "workflow {}: command {} start was not applied: application is shutting down",
                    input.execution_id,
                    input.node_execution_id
                );
                return Ok(());
            }
            let raw_command = input.raw_command.take().ok_or_else(|| {
                WorkflowRuntimeError::InvalidState(format!(
                    "raw command for node execution '{}' is unavailable",
                    input.node_execution_id
                ))
            })?;
            let definition_env = std::mem::take(&mut input.definition_env);
            let display_command = {
                let secrets = secret_source::collect_configured_secret_values(app);
                workflow_secret_masker::mask_sensitive_text(&raw_command, &secrets)
            };
            // Keep the execution lock from the final current-node check through process registration.
            // A concurrent stop therefore has only two observable orders: it wins first and no process
            // is spawned, or the process is registered first and stop can always find and kill it.
            let spawn_result = {
                let commit_lock = self.commit_lock(&input.execution_id).await;
                let _commit_guard = commit_lock.lock().await;
                let Some(execution) = self.load_current_command(app, &input).await? else {
                    return Ok(());
                };
                if !execution
                    .node_execution(&input.node_execution_id)
                    .is_some_and(|node| node.can_start_process())
                    || self
                        .node_processes
                        .active_commands
                        .lock()
                        .expect("command process registry poisoned")
                        .contains_key(&input.node_execution_id)
                {
                    log::warn!(
                        "workflow {}: command {} start was not applied: node was already started",
                        input.execution_id,
                        input.node_execution_id
                    );
                    return Ok(());
                }

                let Some(spawn_result) = command_admission.if_admitted(
                    crate::domain::daemon::DaemonRequest::Operation,
                    || {
                        workflow_command_runner::spawn_shell_command(
                            &input.worktree_path,
                            &raw_command,
                            command_env(&input, definition_env),
                            "workflow command",
                            workflow_command_runner::OutputLimit {
                                max_bytes: workflow_output_limit::MAX_OUTPUT_SIZE,
                                truncation_marker: workflow_output_limit::TRUNCATION_MARKER,
                            },
                        )
                    },
                ) else {
                    return Ok(());
                };
                if let Ok(running) = &spawn_result {
                    self.node_processes
                        .active_commands
                        .lock()
                        .expect("command process registry poisoned")
                        .insert(input.node_execution_id.clone(), running.handle());
                    self.active_command_executions
                        .lock()
                        .await
                        .insert(input.node_execution_id.clone(), input.execution_id.clone());
                }
                spawn_result
            };
            drop(raw_command);

            let running = match spawn_result {
                Ok(running) => running,
                Err(CommandRunnerError::Spawn(error)) => {
                    // The caller converts runtime activation failures into a crash checkpoint after
                    // releasing any activation lock. Interrupting here would recurse into that lock
                    // for fanout command children.
                    return Err(WorkflowRuntimeError::SessionStore(format!(
                        "failed to spawn command: {error}"
                    )));
                }
                Err(error) => {
                    return Err(WorkflowRuntimeError::SessionStore(format!(
                        "failed to prepare command: {error}"
                    )));
                }
            };
            match self
                .commit_command_spawned(app, &input, display_command)
                .await
            {
                Ok(true) => {}
                Ok(false) => {
                    running.handle().request_shutdown();
                    self.node_processes
                        .active_commands
                        .lock()
                        .expect("command process registry poisoned")
                        .remove(&input.node_execution_id);
                    self.active_command_executions
                        .lock()
                        .await
                        .remove(&input.node_execution_id);
                    return Ok(());
                }
                Err(error) => {
                    running.handle().request_shutdown();
                    self.node_processes
                        .active_commands
                        .lock()
                        .expect("command process registry poisoned")
                        .remove(&input.node_execution_id);
                    self.active_command_executions
                        .lock()
                        .await
                        .remove(&input.node_execution_id);
                    return Err(error);
                }
            }
            let driver = self.clone();
            let observer_app = app.clone();
            let node_execution_id = input.node_execution_id.clone();
            let still_current = self.command_execution_still_current(app, &input).await;
            let observer_node_execution_id = node_execution_id.clone();
            let mut observers = self.command_completion_observers.lock().await;
            let observer = tokio::spawn(async move {
                driver
                    .observe_command_completion(&observer_app, input, running)
                    .await;
                driver
                    .command_completion_observers
                    .lock()
                    .await
                    .remove(&observer_node_execution_id);
            });
            observers.insert(node_execution_id.clone(), observer);
            drop(observers);
            drop(command_admission);
            if !still_current {
                self.shutdown_active_command_execution(&node_execution_id)
                    .await;
            }
            Ok(())
        })
    }

    async fn observe_command_completion(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: CommandExecutionInput,
        running: workflow_command_runner::RunningCommand,
    ) {
        let output = running.wait().await;
        self.finish_command_execution(app, input, output).await;
    }

    pub async fn finish_command_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: CommandExecutionInput,
        output: Result<CommandRunOutput, CommandRunnerError>,
    ) {
        self.node_processes
            .active_commands
            .lock()
            .expect("command process registry poisoned")
            .remove(&input.node_execution_id);
        self.active_command_executions
            .lock()
            .await
            .remove(&input.node_execution_id);

        match output {
            Ok(output) => {
                let failure_input = input.clone();
                if let Err(error) = self.commit_command_output(app, input, output).await {
                    if matches!(error, WorkflowRuntimeError::Conflict(_)) {
                        log::warn!(
                            "workflow {}: command {} result was not applied: {error}",
                            failure_input.execution_id,
                            failure_input.node_execution_id
                        );
                        return;
                    }
                    let reason = format!("command completion failed: {error}");
                    log::warn!("{reason}");
                    if let Err(settle_error) = self
                        .fail_current_command_node(app, &failure_input, reason.clone())
                        .await
                    {
                        log::error!(
                            "workflow {}: command completion failed and NodeFailed settlement also failed: {settle_error}",
                            failure_input.execution_id
                        );
                    }
                }
            }
            Err(CommandRunnerError::Cancelled) => {
                let intent = self
                    .command_shutdown_intents
                    .lock()
                    .await
                    .remove(&input.node_execution_id);
                if matches!(intent, Some(ActiveCommandShutdownIntent::GracefulShutdown)) {
                    log::debug!(
                        "workflow {}: command {} cancellation was not applied: graceful shutdown",
                        input.execution_id,
                        input.node_execution_id
                    );
                } else {
                    log::warn!("workflow {}: command {} cancellation was not applied: process was cancelled", input.execution_id, input.node_execution_id);
                }
            }
            Err(error) => {
                let reason = format!("command runtime failed: {error}");
                if let Err(settle_error) = self
                    .fail_current_command_node(app, &input, reason.clone())
                    .await
                {
                    log::error!(
                        "workflow {}: command runtime failed and NodeFailed settlement also failed: {settle_error}",
                        input.execution_id
                    );
                }
                log::warn!("{reason}");
            }
        }
    }

    pub async fn command_execution_still_current(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
    ) -> bool {
        self.load_current_command(app, input)
            .await
            .is_ok_and(|execution| execution.is_some())
    }

    pub async fn commit_command_output(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: CommandExecutionInput,
        output: CommandRunOutput,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.daemon.admission().await;
        if !command_admission.admits(crate::domain::daemon::DaemonRequest::Operation) {
            log::warn!(
                "workflow {}: command {} result was not applied: application is shutting down",
                input.execution_id,
                input.node_execution_id
            );
            return Ok(());
        }
        let secrets = secret_source::collect_configured_secret_values(app);
        let artifact =
            build_command_artifact(&input.schemas, input.contract.as_deref(), output, &secrets);
        let artifact_value = artifact.value.clone();
        let artifact_event_contract = artifact.event_contract.clone();
        let result_summary = artifact.result_summary.clone();
        let timestamp = current_timestamp();

        let committed = retry_runtime_conflicts(&self.queue, &input.node_execution_id, || async {
            let (outcome, snapshot_before, candidate, worktree_path, required_events) = {
                let Some(mut loaded) = self.load_current_command(app, &input).await? else {
                    return Ok(None);
                };
                if !command_admission.admits(crate::domain::daemon::DaemonRequest::Operation) {
                    return Ok(None);
                }
                let exec = &mut loaded;
                let snapshot_before = exec.clone();
                let requires_approval = exec
                    .node_definition(&input.node_name)
                    .map(workflow_transition::decide_completion_disposition)
                    == Some(workflow_transition::CompletionDisposition::RequestApproval);
                let _ = exec.record_pending_result(
                    &input.node_execution_id,
                    Some(result_summary.clone()),
                    Some(artifact_value.clone()),
                    artifact_event_contract.clone(),
                    None,
                    timestamp,
                );
                let mut required_events = vec![WorkflowEvent::ArtifactProduced {
                    execution_id: input.execution_id.clone(),
                    node_execution_id: input.node_execution_id.clone(),
                    node_name: input.node_name.clone(),
                    contract: artifact_event_contract.clone(),
                    value: artifact_value.clone(),
                    request_id: None,
                    submitted_at: None,
                    timestamp,
                }];
                let outcome = if requires_approval {
                    // completion.require: approval — exit code での既定完了後、human の承認まで完了しない。
                    if exec.mark_node_waiting_approval(&input.node_execution_id, timestamp)
                        != TransitionOutcome::Applied
                    {
                        *exec = snapshot_before;
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "command NodeExecution '{}' cannot wait for approval",
                            input.node_execution_id
                        )));
                    }
                    required_events.push(WorkflowEvent::ApprovalRequested {
                        execution_id: input.execution_id.clone(),
                        node_execution_id: input.node_execution_id.clone(),
                        node_name: input.node_name.clone(),
                        result_summary: Some(result_summary.clone()),
                        timestamp,
                    });
                    None
                } else {
                    let mut new_id = new_node_execution_id;
                    let applied = match exec.complete_leaf_and_advance(
                        &input.node_execution_id,
                        &mut new_id,
                        timestamp,
                    ) {
                        Ok(applied) => applied,
                        Err(error) => {
                            *exec = snapshot_before;
                            return Err(WorkflowRuntimeError::InvalidState(error.to_string()));
                        }
                    };
                    required_events.extend(applied.events);
                    match workflow_runtime_driver::node_outcome_from_advance(exec, applied.decision)
                    {
                        Ok(outcome) => Some(outcome),
                        Err(error) => {
                            *exec = snapshot_before;
                            return Err(error);
                        }
                    }
                };
                (
                    outcome,
                    snapshot_before,
                    exec.clone(),
                    exec.worktree_path.clone(),
                    required_events,
                )
            };

            let Some(snapshot_for_commit) = self
                .commit_admitted_command_candidate(
                    app,
                    &command_admission,
                    ControlPlaneCommitCandidate {
                        execution_id: &input.execution_id,

                        snapshot_before,
                        candidate,
                        transition_outcome: TransitionOutcome::Applied,
                        events: &required_events,
                        provider_events: Vec::new(),
                    },
                )
                .await
                .map_err(|error| {
                    with_append_context(error, "command completion event append failed")
                })?
            else {
                return Ok(None);
            };
            Ok(Some((outcome, snapshot_for_commit, worktree_path)))
        })
        .await?;
        let Some((outcome, snapshot_for_commit, worktree_path)) = committed else {
            return Ok(());
        };
        drop(command_admission);
        self.finalize_after_commit(app, &snapshot_for_commit, &worktree_path)
            .await;
        if let Some(outcome) = outcome {
            Box::pin(self.dispatch_node_outcome_side_effects(app, &worktree_path, outcome)).await?;
        }
        Ok(())
    }

    pub async fn fail_current_command_node(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
        reason: String,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.daemon.admission().await;
        if !command_admission.admits(crate::domain::daemon::DaemonRequest::Operation) {
            log::warn!(
                "workflow {}: command {} failure was not applied: application is shutting down",
                input.execution_id,
                input.node_execution_id
            );
            return Ok(());
        }
        let events = [WorkflowEvent::NodeFailed {
            execution_id: input.execution_id.clone(),
            node_execution_id: input.node_execution_id.clone(),
            node_name: input.node_name.clone(),
            attempt: input.attempt,
            reason,
            failure_kind: NodeExecutionFailureKind::InfrastructureCrash,
            retry_count: None,
            timestamp: current_timestamp(),
        }];
        let snapshot = retry_runtime_conflicts(&self.queue, &input.node_execution_id, || async {
            let Some(before) = self.load_current_command(app, input).await? else {
                return Ok(None);
            };
            if !command_admission.admits(crate::domain::daemon::DaemonRequest::Operation) {
                return Ok(None);
            }
            self.commit_admitted_command_candidate(
                app,
                &command_admission,
                ControlPlaneCommitCandidate {
                    execution_id: &input.execution_id,
                    snapshot_before: before.clone(),
                    candidate: before,
                    transition_outcome: TransitionOutcome::AlreadyApplied,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await
        })
        .await
        .inspect_err(|error| {
            log::warn!(
                "workflow {}: command {} failure was not applied: {error}",
                input.execution_id,
                input.node_execution_id
            );
        })?;
        let Some(snapshot) = snapshot else {
            return Ok(());
        };
        drop(command_admission);
        self.finish_control_plane_commit(app, &snapshot.worktree_path, &snapshot, None)
            .await?;
        Ok(())
    }

    async fn shutdown_active_command_execution(&self, node_execution_id: &str) {
        if let Some(handle) = self
            .node_processes
            .active_commands
            .lock()
            .expect("command process registry poisoned")
            .remove(node_execution_id)
        {
            handle.request_shutdown();
        }
        let observer = self
            .command_completion_observers
            .lock()
            .await
            .remove(node_execution_id);
        if let Some(observer) = observer {
            if let Err(error) = observer.await {
                log::warn!(
                    "node execution {node_execution_id}: command completion observer failed: {error}"
                );
            }
        }
        self.command_shutdown_intents
            .lock()
            .await
            .remove(node_execution_id);
        self.active_command_executions
            .lock()
            .await
            .remove(node_execution_id);
    }

    pub(crate) async fn shutdown_active_commands_for_execution(&self, execution_id: &str) -> bool {
        let node_execution_ids = self
            .active_command_executions
            .lock()
            .await
            .iter()
            .filter_map(|(node_execution_id, owner_execution_id)| {
                (owner_execution_id == execution_id).then_some(node_execution_id.clone())
            })
            .collect::<Vec<_>>();
        let observed_owned_command = !node_execution_ids.is_empty();
        for node_execution_id in node_execution_ids {
            self.shutdown_active_command_execution(&node_execution_id)
                .await;
        }
        observed_owned_command
    }

    pub async fn shutdown_all_active_commands(&self) {
        self.daemon.drain_commands().await;
        self.shutdown_startup_retries().await;
        let commands = {
            let active_commands = self
                .node_processes
                .active_commands
                .lock()
                .expect("command process registry poisoned");
            active_commands
                .iter()
                .map(|(node_execution_id, handle)| (node_execution_id.clone(), handle.clone()))
                .collect::<Vec<_>>()
        };
        if commands.is_empty() {
            return;
        }
        {
            let mut intents = self.command_shutdown_intents.lock().await;
            for (node_execution_id, _) in &commands {
                intents.insert(
                    node_execution_id.clone(),
                    ActiveCommandShutdownIntent::GracefulShutdown,
                );
            }
        }
        for (_, handle) in &commands {
            handle.request_shutdown();
        }
        let observers = {
            let mut observers = self.command_completion_observers.lock().await;
            commands
                .iter()
                .filter_map(|(node_execution_id, _)| observers.remove(node_execution_id))
                .collect::<Vec<_>>()
        };
        for observer in observers {
            if let Err(error) = observer.await {
                log::warn!("workflow command completion observer failed during shutdown: {error}");
            }
        }
        let node_execution_ids = commands
            .into_iter()
            .map(|(node_execution_id, _)| node_execution_id)
            .collect::<Vec<_>>();
        {
            let mut active_commands = self
                .node_processes
                .active_commands
                .lock()
                .expect("command process registry poisoned");
            for node_execution_id in &node_execution_ids {
                active_commands.remove(node_execution_id);
            }
        }
        {
            let mut intents = self.command_shutdown_intents.lock().await;
            for node_execution_id in &node_execution_ids {
                intents.remove(node_execution_id);
            }
        }
        let mut executions = self.active_command_executions.lock().await;
        for node_execution_id in &node_execution_ids {
            executions.remove(node_execution_id);
        }
    }

    /// 記録の最新状態との競合を検査して必須事実を追記する。
    async fn commit_required_events(
        &self,
        app: &WorkflowRuntimeDependencies,
        commit: RequiredEventCommit<'_>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        let RequiredEventCommit {
            execution_id,
            snapshot_before,
            candidate,
            required_events,
            append_error_context,
        } = commit;
        self.commit_control_plane_candidate(
            app,
            ControlPlaneCommitCandidate {
                execution_id,
                snapshot_before,
                candidate,
                transition_outcome: TransitionOutcome::Applied,
                events: &required_events,
                provider_events: Vec::new(),
            },
        )
        .await
        .map_err(|error| with_append_context(error, append_error_context))
    }

    /// [04] post-commit phase: broadcast and runtime release. Every required
    /// transition/terminal event is already in the canonical commit; this
    /// phase contains only derived notifications and in-memory cleanup.
    async fn finalize_after_commit(
        &self,
        app: &WorkflowRuntimeDependencies,
        snapshot: &RuntimeCommitSnapshot,
        worktree_path: &str,
    ) {
        let execution_id = snapshot.execution_id.clone();
        let is_finished = matches!(
            snapshot.state,
            RuntimeExecutionState::Completed | RuntimeExecutionState::Aborted
        );
        workflow_runtime_session::broadcast_state(app, worktree_path).await;
        if is_finished {
            self.release_terminal_execution(&execution_id).await;
        }
    }

    async fn settle_runtime_failure(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        error: &WorkflowRuntimeError,
    ) -> Result<(), WorkflowRuntimeError> {
        let node_execution_id = {
            let loaded = self.load_execution(app, execution_id).await?;
            let execution = &loaded;
            if !execution.is_active() {
                return Ok(());
            }
            execution
                .node_executions
                .iter()
                .rev()
                .find(|node| node.status.is_active() && !node.kind.is_composite_kind())
                .map(|node| node.id.clone())
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "workflow '{execution_id}' has no active node attempt to fail"
                    ))
                })?
        };
        self.settle_runtime_failure_for_node(app, execution_id, &node_execution_id, error)
            .await
    }

    pub async fn settle_runtime_failure_for_node(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        error: &WorkflowRuntimeError,
    ) -> Result<(), WorkflowRuntimeError> {
        if let Some(reason) = error.version_conflict() {
            return Err(WorkflowRuntimeError::Conflict(reason.into()));
        }
        let failure_kind = error.workflow_failure_kind();
        let reason = format!("workflow runtime activation failed: {error}");
        crate::usecase::workflow::command::retry_control_plane_operation(
            &self.queue,
            node_execution_id,
            || {
                self.settle_node_failure_for_node(
                    app,
                    execution_id,
                    node_execution_id,
                    reason.clone(),
                    failure_kind,
                )
            },
        )
        .await
    }

    pub async fn settle_node_failure_for_node(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.daemon.admission().await;
        let timestamp = current_timestamp();
        let (snapshot_before, candidate, node_name, attempt, is_command) = {
            let loaded = self.load_execution(app, execution_id).await?;
            let execution = &loaded;
            if !execution.is_active() {
                return Ok(());
            }
            let node = execution
                .node_executions
                .iter()
                .find(|node| node.id == node_execution_id && node.status.is_active())
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "workflow '{execution_id}' has no active NodeExecution '{node_execution_id}' to fail"
                    ))
                })?;
            if node.kind == NodeKindName::Command
                && !command_admission.admits(crate::domain::daemon::DaemonRequest::Operation)
            {
                return Ok(());
            }
            (
                execution.clone(),
                execution.clone(),
                node.node_name.clone(),
                node.attempt,
                node.kind == NodeKindName::Command,
            )
        };
        let events = vec![WorkflowEvent::NodeFailed {
            execution_id: execution_id.to_string(),
            node_execution_id: node_execution_id.to_string(),
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count: None,
            timestamp,
        }];
        let commit = ControlPlaneCommitCandidate {
            execution_id,
            snapshot_before,
            candidate,
            transition_outcome: TransitionOutcome::AlreadyApplied,
            events: &events,
            provider_events: Vec::new(),
        };
        let snapshot = if is_command {
            let Some(snapshot) = self
                .commit_admitted_command_candidate(app, &command_admission, commit)
                .await?
            else {
                return Ok(());
            };
            snapshot
        } else {
            self.commit_control_plane_candidate(app, commit).await?
        };
        drop(command_admission);
        self.finish_control_plane_commit(app, &snapshot.worktree_path, &snapshot, None)
            .await?;
        Ok(())
    }

    /// [04] post-commit variant work（共通 side-effect helper）。
    ///
    /// snapshot は既に persist 済みである前提で、outcome variant に応じた残りの副作用
    /// （NodeStarted 書き込み・start_node_session・reduce + 派生 mutation の再帰・
    /// start_fanout_children）のみを担当する。`execute_outcome`
    /// （non-command 経路）と `handle_approval` などの 4 command handler の双方から
    /// 呼ばれ、副作用ロジックの単一 source of truth として機能する。失敗は warn 化して
    /// command 結果に伝播させない設計に揃える（spec [04] post-commit 境界）。
    async fn dispatch_node_outcome_side_effects(
        &self,
        app: &WorkflowRuntimeDependencies,
        worktree_path: &str,
        outcome: NodeOutcome,
    ) -> Result<(), WorkflowRuntimeError> {
        match outcome {
            NodeOutcome::Persist => Ok(()),
            NodeOutcome::StartNodes(snapshot, leaves) => {
                if let Err(e) =
                    Box::pin(self.start_nodes(app, &snapshot.execution_id, worktree_path, leaves))
                        .await
                {
                    if let Err(settle_error) =
                        Box::pin(self.settle_runtime_failure(app, &snapshot.execution_id, &e)).await
                    {
                        if matches!(settle_error, WorkflowRuntimeError::Conflict(_)) {
                            log::warn!(
                                "workflow {}: post-commit node start was not applied: {e}",
                                snapshot.execution_id
                            );
                            return Ok(());
                        }
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "{e}; NodeFailed settlement failed: {settle_error}"
                        )));
                    }
                    return Ok(());
                }
                Ok(())
            }
        }
    }

    /// 複数の必須 event を事実ログへ一括追記する。
    ///
    /// [04] spec『event 列と domain state の整合』Rule: 同一 command 受理サイクル内で
    /// 複数 required event を発行する場合は本 helper を使う。永続形は純粋事実の
    /// 行 append であり、導出表 mutation は存在しない。
    pub async fn write_log_required_batch(
        &self,
        app: &WorkflowRuntimeDependencies,
        events: &[WorkflowEvent],
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        workflow_event_log_writer::append_required_events_for_app(app, events).await
    }
}

#[cfg(test)]
#[path = "workflow_host_test.rs"]
mod workflow_host_tests;

#[cfg(feature = "test-support")]
impl CommandArtifact {
    pub fn test_value(&self) -> &serde_json::Value {
        &self.value
    }
}

#[cfg(feature = "test-support")]
impl WorkflowRuntimeHost {
    pub fn test_active_command_executions(&self) -> &Arc<Mutex<HashMap<String, String>>> {
        &self.active_command_executions
    }
    pub fn test_command_completion_observers(
        &self,
    ) -> &Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> {
        &self.command_completion_observers
    }
    pub fn test_commit_locks(&self) -> &RuntimeLockMap {
        &self.commit_locks
    }
    pub fn test_daemon(&self) -> &Arc<crate::adaptor::gateway::daemon::InMemoryDaemonRepository> {
        &self.daemon
    }
    pub fn test_execution_facet_contents(
        &self,
    ) -> &Arc<Mutex<HashMap<String, WorkflowFacetContents>>> {
        &self.execution_facet_contents
    }
    pub fn test_isolated_worktrees(
        &self,
    ) -> &Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway> {
        &self.isolated_worktrees
    }
    pub fn test_isolated_worktrees_mut(
        &mut self,
    ) -> &mut Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway> {
        &mut self.isolated_worktrees
    }
    pub fn test_runtime_activation_locks(
        &self,
    ) -> &Arc<Mutex<HashMap<String, Weak<RuntimeActivationGate>>>> {
        &self.runtime_activation_locks
    }
    pub fn test_startup_retries(
        &self,
    ) -> &Arc<Mutex<HashMap<String, node_startup::NodeStartupTask>>> {
        &self.startup_retries
    }
    pub fn test_workflow_start_locks(&self) -> &RuntimeLockMap {
        &self.workflow_start_locks
    }
    pub fn test_workspace_query(
        &self,
    ) -> &Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService> {
        &self.workspace_query
    }
    pub fn test_worktree_resolver(&self) -> &Arc<dyn ManagedWorktreeResolver> {
        &self.worktree_resolver
    }
    pub fn test_worktree_resolver_mut(&mut self) -> &mut Arc<dyn ManagedWorktreeResolver> {
        &mut self.worktree_resolver
    }
}

#[cfg(feature = "test-support")]
impl<'a> ControlPlaneCommitCandidate<'a> {
    pub fn test_new(
        execution_id: &'a str,
        snapshot_before: DomainExecutionTree,
        candidate: DomainExecutionTree,
        transition_outcome: TransitionOutcome,
        events: &'a [WorkflowEvent],
        provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
    ) -> Self {
        Self {
            execution_id,
            snapshot_before,
            candidate,
            transition_outcome,
            events,
            provider_events,
        }
    }
}

#[cfg(feature = "test-support")]
impl WorkflowExecutionInsert {
    pub fn test_new(
        execution_id: String,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        workflow_defaults: WorkflowDefaults,
        now: f64,
    ) -> Self {
        Self {
            execution_id,
            workflow,
            worktree_path,
            request,
            created_from,
            workflow_defaults,
            now,
        }
    }
}

#[cfg(feature = "test-support")]
type RuntimeLockMap = Arc<Mutex<HashMap<String, Weak<Mutex<()>>>>>;
