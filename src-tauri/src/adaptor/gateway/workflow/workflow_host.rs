//! Workflow execution host gateway.
//!
//! The domain aggregate owns lifecycle transitions and decisions, while
//! `usecase::workflow::runtime_driver` owns their application procedure and
//! transaction ordering. This gateway reads aggregates from facts, delegates
//! decisions to them, and connects event storage, agent sessions, processes,
//! and notifications.

#[cfg(test)]
use crate::adaptor::gateway::workflow::fact_codec;
use crate::domain::failure::ClassifiedFailure;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

use tokio::sync::{Mutex, RwLock};

mod activation;
pub(crate) mod approval_runtime;
mod command_preparation;
pub(crate) mod delegate;
pub(crate) mod execution_state;
mod isolated_worktree;
mod lifecycle_commands;
pub(crate) mod node_settings;
mod node_startup;
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
#[cfg(test)]
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus as NodeExecutionStatus;
use crate::domain::workflow::entities::workflow_execution::{
    AppliedAdvance, LeafKind, LeafStart, NodeStart, TransitionOutcome,
};
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::reference as workflow_reference;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::ExecutionOrigin;
#[cfg(test)]
use crate::domain::workflow::ExecutionStatus;
#[cfg(test)]
use crate::domain::workflow::ExecutionTreeLaunch;
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
pub(crate) struct WorkflowRuntimeDependencies {
    pub(crate) store: Option<Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>>,
    pub(crate) config: Option<Arc<dyn crate::domain::app_config::ConfigRepository>>,
    pub(crate) secrets: Option<Arc<dyn crate::domain::app_config::ConfigSecretRepository>>,
    pub(crate) state_changes: crate::usecase::state_subscription::StateSubscriptionPublisher,
}

fn current_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

/// 記録から取得した Workflow 集約と usecase の駆動手順を外界へ接続する gateway host。
#[derive(Clone)]
pub struct WorkflowRuntimeHost {
    pub(crate) queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
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
    pub(crate) node_processes: Arc<super::node_process::WorkflowNodeProcesses>,
    command_admission: Arc<RwLock<crate::domain::application_lifecycle::CommandAdmission>>,
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
    pub(crate) delegate_continuation:
        Option<Arc<crate::usecase::workflow::delegate::DelegateContinuationUsecase>>,
}

#[derive(Clone)]
struct ControlPlaneCommitCandidate<'a> {
    execution_id: &'a str,
    snapshot_before: DomainExecutionTree,
    candidate: DomainExecutionTree,
    transition_outcome: TransitionOutcome,
    events: &'a [WorkflowEvent],
    provider_events: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
}

#[derive(Clone)]
struct WorkflowExecutionInsert {
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

struct CommandArtifact {
    value: serde_json::Value,
    event_contract: Option<String>,
    result_summary: String,
}

fn command_env(
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

fn build_command_artifact(
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

async fn retry_runtime_conflicts<T, F, Fut>(
    queue: &std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
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
    pub(crate) async fn load_control_plane_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<Option<DomainExecutionTree>, WorkflowRuntimeError> {
        Ok(Self::load_execution_revision(app, execution_id)
            .await?
            .map(|(execution, _)| execution))
    }

    async fn load_execution_revision(
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
            .map_err(|error| WorkflowRuntimeError::StorageFailure {
                kind: error.failure_kind(),
                message: format!("tree read failed: {error:?}"),
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

    async fn load_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<DomainExecutionTree, WorkflowRuntimeError> {
        self.load_control_plane_execution(app, execution_id)
            .await?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))
    }

    #[cfg(test)]
    async fn load_executions(
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

    async fn append_events_at_head(
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
            .map_err(|error| WorkflowRuntimeError::StorageFailure {
                kind: error.failure_kind(),
                message: error.to_string(),
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
                    .map_err(|error| WorkflowRuntimeError::StorageFailure {
                        kind: error.failure_kind(),
                        message: format!("control-plane commit readback failed: {error:?}"),
                    })?
                    .map(|_| ())
            }
            result => result.map(|_| ()),
        }
        .map_err(|error| match error {
            CommitBatchError::TreeHeadConflict => WorkflowRuntimeError::Conflict(format!(
                "execution '{execution_id}' changed before commit"
            )),
            other => WorkflowRuntimeError::StorageFailure {
                kind: other.failure_kind(),
                message: other.to_string(),
            },
        })
    }

    pub(crate) async fn register_started_execution_tree(
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
        queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        agent_session_launch: Arc<AgentSessionLaunchUsecase>,
        agent_session_initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
        agent_session_lifecycle: Arc<AgentSessionLifecycleUsecase>,
        provider_availability: Arc<dyn crate::domain::agent_session::ProviderAvailabilityReader>,
        isolated_worktrees: Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway>,
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
        )
    }

    pub(crate) fn with_runtime_ports(
        queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
        isolated_worktrees: Arc<dyn crate::domain::workflow::IsolatedWorktreeGateway>,
    ) -> Self {
        Self {
            queue,
            workflow_start_locks: Arc::new(Mutex::new(HashMap::new())),
            commit_locks: Arc::new(Mutex::new(HashMap::new())),
            execution_facet_contents: Arc::new(Mutex::new(HashMap::new())),
            runtime_activation_locks: Arc::new(Mutex::new(HashMap::new())),
            startup_retries: Arc::new(Mutex::new(HashMap::new())),
            node_processes: Arc::new(Default::default()),
            command_admission: Arc::new(RwLock::new(Default::default())),
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

    async fn workflow_start_lock(&self, worktree_path: &str) -> Arc<Mutex<()>> {
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

    async fn commit_lock(&self, execution_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.commit_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(execution_id).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(execution_id.to_string(), Arc::downgrade(&lock));
        lock
    }

    async fn runtime_activation_gate(&self, execution_id: &str) -> Arc<RuntimeActivationGate> {
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

    async fn insert_workflow_execution(
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
                    error => WorkflowRuntimeError::StorageFailure {
                        kind: error.failure_kind(),
                        message: error.to_string(),
                    },
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
                None,
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
            return Err(WorkflowRuntimeError::StorageFailure {
                kind: e.failure_kind(),
                message: format!("write initial workflow event batch failed: {e}"),
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

    pub(crate) async fn start_resolved_workflow(
        &self,
        app: &WorkflowRuntimeDependencies,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
    ) -> Result<String, WorkflowRuntimeError> {
        self.start_workflow(app, workflow, worktree_path, request, created_from)
            .await
    }

    async fn commit_control_plane_candidate(
        &self,
        app: &WorkflowRuntimeDependencies,
        commit: ControlPlaneCommitCandidate<'_>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        crate::adaptor::gateway::work_queue::retry(
            &self.queue,
            crate::usecase::work_queue::WorkKey::new("workflow_runtime", commit.execution_id),
            crate::common::retry::RetryBackoff::CONFLICT,
            || self.commit_control_plane_candidate_once(app, commit.clone()),
            false,
        )
        .await
    }

    async fn commit_control_plane_candidate_once(
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
        let commit_lock = self.commit_lock(execution_id).await;
        let _commit_guard = commit_lock.lock().await;
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

    async fn run_committed_runtime_effects(
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

    pub(crate) async fn release_deleted_execution_tree(
        &self,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.release_execution_facet_contents(execution_id).await;
        Ok(())
    }

    #[cfg(test)]
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

    #[cfg(all(debug_assertions, feature = "desktop"))]
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
    async fn start_nodes(
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

    async fn record_node_start_failure(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        error: &WorkflowRuntimeError,
        failed: &mut Vec<crate::usecase::workflow::node_startup::FailedNodeStart>,
    ) -> Result<(), WorkflowRuntimeError> {
        use crate::domain::failure::RetryAction;
        self.queue
            .observe(
                &crate::usecase::work_queue::WorkKey::new("workflow_node_start", node_execution_id),
                &crate::usecase::work_queue::WorkFailure::from_error(error),
            )
            .await;
        if error.failure_kind().retry_action() != RetryAction::Stop {
            failed.push(crate::usecase::workflow::node_startup::FailedNodeStart {
                id: node_execution_id.into(),
                kind: error.failure_kind(),
            });
        }
        if error.failure_kind().retry_action() != RetryAction::Restart {
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

    async fn spawn_command_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        mut input: CommandExecutionInput,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.command_admission.read().await;
        if !command_admission.accepts_start() {
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

            let spawn_result = workflow_command_runner::spawn_shell_command(
                &input.worktree_path,
                &raw_command,
                command_env(&input, definition_env),
                "workflow command",
                workflow_command_runner::OutputLimit {
                    max_bytes: workflow_output_limit::MAX_OUTPUT_SIZE,
                    truncation_marker: workflow_output_limit::TRUNCATION_MARKER,
                },
            );
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
        let runtime_handle = tokio::runtime::Handle::current();
        let observer = tokio::task::spawn_blocking(move || {
            runtime_handle.block_on(async move {
                driver
                    .observe_command_completion(&observer_app, input, running)
                    .await;
                driver
                    .command_completion_observers
                    .lock()
                    .await
                    .remove(&observer_node_execution_id);
            });
        });
        self.command_completion_observers
            .lock()
            .await
            .insert(node_execution_id.clone(), observer);
        drop(command_admission);
        if !still_current {
            self.shutdown_active_command_execution(&node_execution_id)
                .await;
        }
        Ok(())
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

    async fn finish_command_execution(
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

    async fn command_execution_still_current(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
    ) -> bool {
        self.load_current_command(app, input)
            .await
            .is_ok_and(|execution| execution.is_some())
    }

    async fn commit_command_output(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: CommandExecutionInput,
        output: CommandRunOutput,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.command_admission.read().await;
        if !command_admission.accepts_completion() {
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

            let snapshot_for_commit = self
                .commit_required_events(
                    app,
                    RequiredEventCommit {
                        execution_id: &input.execution_id,

                        snapshot_before,
                        candidate,
                        required_events,
                        append_error_context: "command completion event append failed",
                    },
                )
                .await?;
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

    async fn fail_current_command_node(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
        reason: String,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.command_admission.read().await;
        if !command_admission.accepts_completion() {
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
            self.commit_control_plane_candidate(
                app,
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
            .map(Some)
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

    pub(crate) async fn shutdown_all_active_commands(&self) {
        self.command_admission.write().await.stop();
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
        .map_err(|error| match error {
            WorkflowRuntimeError::SessionStore(reason) => {
                WorkflowRuntimeError::SessionStore(format!("{append_error_context}: {reason}"))
            }
            WorkflowRuntimeError::StorageFailure { message, kind } => {
                WorkflowRuntimeError::StorageFailure {
                    message: format!("{append_error_context}: {message}"),
                    kind,
                }
            }
            other => other,
        })
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

    async fn settle_runtime_failure_for_node(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        error: &WorkflowRuntimeError,
    ) -> Result<(), WorkflowRuntimeError> {
        if let WorkflowRuntimeError::Conflict(reason) = error {
            return Err(WorkflowRuntimeError::Conflict(reason.clone()));
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

    async fn settle_node_failure_for_node(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        node_execution_id: &str,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
    ) -> Result<(), WorkflowRuntimeError> {
        let command_admission = self.command_admission.read().await;
        let timestamp = current_timestamp();
        let (snapshot_before, candidate, node_name, attempt) = {
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
            if node.kind == NodeKindName::Command && !command_admission.accepts_completion() {
                return Ok(());
            }
            (
                execution.clone(),
                execution.clone(),
                node.node_name.clone(),
                node.attempt,
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
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before,
                    candidate,
                    transition_outcome: TransitionOutcome::AlreadyApplied,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
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
    async fn write_log_required_batch(
        &self,
        app: &WorkflowRuntimeDependencies,
        events: &[WorkflowEvent],
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        workflow_event_log_writer::append_required_events_for_app(app, events).await
    }
}

#[cfg(test)]
mod workflow_host_tests {
    use super::test_helpers::{
        record_workflow_execution_broadcasts, take_workflow_execution_broadcasts,
    };
    use super::*;
    use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
    use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
    use crate::adaptor::gateway::local_event_store::node_events::NewNodeEventRow;
    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
    use crate::adaptor::gateway::workflow::WorkflowRuntimeCommandGateway;
    use crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository;
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::local_event::{
        LoadStreamRequest, LocalEventTransactionRepository, StreamId,
    };
    use crate::domain::provider_lifecycle::{
        ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
    };
    use crate::domain::workflow::{
        ChildEntry, ExecutionParentRef, ExecutionTreeLaunch, FacetRefs, NodeCompletion,
        NodeDefinition, NodeFact, NodeFactMeta, NodeKind, SequenceSpec,
        SessionExecutionTreeRootFacts, SessionPermission, SessionSpec, StartedFact, TreeRootFact,
        WorkflowDefinition,
    };
    use crate::domain::workspace_tree::{
        WorkspaceIdentity, WorkspaceNodeStatusClassification, WorkspaceTreeRepository,
    };
    use crate::usecase::provider_lifecycle::ProviderExecutionTreeStopCommand;
    use crate::usecase::workflow::command::{ApprovalCommand, SubmitOutputCommand};
    use crate::usecase::workflow::control_plane::WorkflowControlPlaneUsecase;
    use crate::usecase::workflow::runtime_resolver::{
        ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
    };

    const EFFECT_WORKTREE_PATH: &str = "/repo/effect-test";
    const EFFECT_NODE_NAME: &str = "agent";
    const EFFECT_AGENT_SESSION_ID: &str = "agent-session-effect-test";

    pub(super) struct UnusedWorkflowResolver;

    #[async_trait::async_trait]
    impl WorkflowDefinitionResolver for UnusedWorkflowResolver {
        async fn resolve(
            &self,
            _workflow_name: &str,
        ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
            Err(WorkflowDefinitionResolverError::Infrastructure(
                "unused in startup recovery".to_string(),
            ))
        }
    }

    struct UnusedWorktreeResolver;

    pub(super) struct AcceptingWorktreeResolver;

    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for UnusedWorktreeResolver {
        async fn resolve(
            &self,
            _worktree_path: String,
        ) -> Result<String, ManagedWorktreeResolverError> {
            Err(ManagedWorktreeResolverError::Validation(
                "unused in startup recovery".to_string(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl ManagedWorktreeResolver for AcceptingWorktreeResolver {
        async fn resolve(
            &self,
            worktree_path: String,
        ) -> Result<String, ManagedWorktreeResolverError> {
            Ok(worktree_path)
        }
    }

    struct FailingWorkflowAgentSessions;

    struct RecordingWorkflowAgentSessions {
        stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        prepare_calls: Arc<std::sync::Mutex<Vec<(String, String, WorkflowSessionLaunchConfig)>>>,
        provider_running_checks: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        recovery_fails: Arc<std::sync::atomic::AtomicBool>,
        failing_agent_session_id: String,
    }

    #[tokio::test]
    async fn test_command完了_承認要求ありなら承認後に完了し省略時は自動完了する() {
        // Given
        for parent in ["", "  main:\n    sequence: {children: [run]}\n"] {
            for (completion, exit_code) in [
                ("", 0),
                ("", 7),
                ("    completion: {require: approval}\n", 0),
                ("    completion: {require: approval}\n", 7),
            ] {
                let require_approval = !completion.is_empty();
                let directory = tempfile::tempdir().unwrap();
                let store = LocalEventStore::open(LocalEventStoreConfig::production(
                    directory.path().to_path_buf(),
                ))
                .unwrap();
                let app = test_helpers::dependencies(Some(store.clone()));
                let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                    crate::usecase::work_queue::shared().clone(),
                    Arc::new(UnusedWorkflowResolver),
                    Arc::new(AcceptingWorktreeResolver),
                    test_helpers::workspace_query(store.clone()),
                    Arc::new(FailingWorkflowAgentSessions),
                    Arc::new(test_helpers::TestWorktrees::default()),
                ));
                let node_name = if parent.is_empty() { "main" } else { "run" };
                let workflow = serde_saphyr::from_str::<WorkflowDefinition>(&format!(
                    "name: command-completion\ndescription: test\nnodes:\n{parent}  {node_name}:\n    command: 'true'\n{completion}"
                ))
                .unwrap();
                let worktree_path = directory.path().to_string_lossy().into_owned();
                let now = current_timestamp();
                let execution_id = uuid::Uuid::new_v4().to_string();
                let (started, applied) = host
                    .insert_workflow_execution(WorkflowExecutionInsert {
                        execution_id: execution_id.clone(),
                        workflow: workflow.clone(),
                        worktree_path: worktree_path.clone(),
                        request: None,
                        created_from: ExecutionOrigin::DesktopUi,
                        workflow_defaults: WorkflowDefaults,
                        now,
                    })
                    .await
                    .unwrap();
                let mut start_events = vec![WorkflowEvent::ExecutionStarted {
                    repository_root: None,
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.clone(),
                    created_from: ExecutionOrigin::DesktopUi,
                    request: String::new(),
                    definition: workflow,
                    timestamp: now,
                }];
                start_events.extend(applied.events);
                host.write_log_required_batch(&app, &start_events)
                    .await
                    .unwrap();
                let node = started
                    .node_executions
                    .iter()
                    .find(|node| node.node_name == node_name)
                    .unwrap();
                assert_eq!(node.status, NodeExecutionStatus::Running);
                let node_execution_id = node.id.clone();
                let input = CommandExecutionInput {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: node_name.to_string(),
                    attempt: node.attempt,
                    worktree_path,
                    raw_command: Some("true".to_string()),
                    definition_env: Vec::new(),
                    contract: None,
                    schemas: Default::default(),
                    session_id: None,
                };
                let mut broadcasts = record_workflow_execution_broadcasts(&app);

                // When
                host.commit_command_output(
                    &app,
                    input,
                    CommandRunOutput {
                        exit_code,
                        stdout: "command finished".to_string(),
                        stderr: String::new(),
                        duration_ms: 10,
                    },
                )
                .await
                .unwrap();

                // Then
                let records = workflow_fact_log::read_tree_records(&store, &execution_id)
                    .await
                    .unwrap();
                assert!(records.iter().any(|record| matches!(
                    &record.fact,
                    NodeFact::ArtifactProduced(fact) if record.meta.node_execution_id == node_execution_id && fact.value["stdout"] == "command finished" && fact.value["ok"] == (exit_code == 0)
                )));
                assert!(!records
                    .iter()
                    .any(|record| matches!(record.fact, NodeFact::ApprovalGranted(_))));
                assert!(!take_workflow_execution_broadcasts(&mut broadcasts).is_empty());
                let snapshot = host
                    .get_state_by_execution_id(&app, &execution_id)
                    .await
                    .unwrap();
                let expected = if require_approval {
                    NodeExecutionStatus::WaitingApproval
                } else {
                    NodeExecutionStatus::Succeeded
                };
                assert_eq!(
                    snapshot
                        .node_executions
                        .iter()
                        .find(|node| node.id == node_execution_id)
                        .unwrap()
                        .status,
                    expected
                );
                if require_approval {
                    let snapshot = host
                        .get_state_by_execution_id(&app, &execution_id)
                        .await
                        .unwrap();
                    assert_eq!(
                        snapshot
                            .node_executions
                            .iter()
                            .find(|node| node.id == node_execution_id)
                            .unwrap()
                            .status,
                        NodeExecutionStatus::WaitingApproval
                    );
                    assert_ne!(snapshot.state, RuntimeExecutionState::Completed);
                    let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                        app.clone(),
                        host.clone(),
                    ));
                    WorkflowControlPlaneUsecase::new(
                        crate::usecase::work_queue::shared().clone(),
                        gateway,
                    )
                    .resolve_approval(ApprovalCommand {
                        execution_id: execution_id.clone(),
                        node_name: node_name.to_string(),
                        node_execution_id: Some(node_execution_id.clone()),
                        comment: None,
                    })
                    .await
                    .unwrap();
                    let records = workflow_fact_log::read_tree_records(&store, &execution_id)
                        .await
                        .unwrap();
                    assert!(records
                        .iter()
                        .any(|record| record.meta.node_execution_id == node_execution_id
                            && matches!(record.fact, NodeFact::ApprovalGranted(_))));
                }
                let completed = host
                    .get_state_by_execution_id(&app, &execution_id)
                    .await
                    .unwrap();
                assert_eq!(completed.state, RuntimeExecutionState::Completed);
                assert!(completed
                    .node_executions
                    .iter()
                    .all(|node| node.status == NodeExecutionStatus::Succeeded));
                let folded = workflow_fact_log::fold_tree_from(
                    &workflow_fact_log::FactLogReadBackend::Live(store.clone()),
                    &execution_id,
                )
                .await
                .unwrap()
                .unwrap();
                let replayed = folded.aggregate.node_execution(&node_execution_id).unwrap();
                assert_eq!(replayed.status, NodeExecutionStatus::Succeeded);
                assert_eq!(replayed.artifact.as_ref().unwrap()["ok"], exit_code == 0);
                assert_eq!(replayed.artifact.as_ref().unwrap()["exit_code"], exit_code);
                assert_eq!(*folded.aggregate.state(), RuntimeExecutionState::Completed);
            }
        }
    }

    #[tokio::test]
    async fn test_command完了_追記結果不明でもdurableな完了へliveとactiveを収束する() {
        // Given
        for durable in [false, true] {
            let fixture = test_helpers::Fixture::new(0);
            let started = fixture
                .persist_started("  main: {command: 'true'}\n", "/repo")
                .await;
            let execution_id = &started.execution_id;
            let node = &started.node_executions[0];
            fixture
                .host
                .register_started_execution_tree(&fixture.app, execution_id)
                .await
                .unwrap();
            let input = CommandExecutionInput {
                execution_id: execution_id.clone(),
                node_execution_id: node.id.clone(),
                node_name: node.node_name.clone(),
                attempt: node.attempt,
                worktree_path: "/repo".into(),
                raw_command: Some("true".into()),
                definition_env: Vec::new(),
                contract: None,
                schemas: Default::default(),
                session_id: None,
            };
            if durable {
                fixture.store.fault_injector().arm_drop_reply();
            } else {
                fixture.store.close_write_queue_for_tests();
            }

            // When
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                fixture.host.commit_command_output(
                    &fixture.app,
                    input.clone(),
                    CommandRunOutput {
                        exit_code: 0,
                        stdout: "done".into(),
                        stderr: String::new(),
                        duration_ms: 1,
                    },
                ),
            )
            .await;

            // Then
            if durable {
                result.unwrap().unwrap();
            } else {
                assert!(result.is_err());
            }
            let records = workflow_fact_log::read_tree_records(&fixture.store, execution_id)
                .await
                .unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ExecutionCompleted))
                    .count(),
                usize::from(durable)
            );
            let expected_state = if durable {
                RuntimeExecutionState::Completed
            } else {
                RuntimeExecutionState::Running
            };
            let current = fixture
                .host
                .get_state_by_execution_id(&fixture.app, execution_id)
                .await
                .unwrap();
            assert_eq!(current.state, expected_state);
            let folded = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
                execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(folded.aggregate.state(), &expected_state);
            if durable {
                assert!(
                    !fixture
                        .host
                        .command_execution_still_current(&fixture.app, &input)
                        .await
                );
                fixture
                    .host
                    .commit_command_output(
                        &fixture.app,
                        input,
                        CommandRunOutput {
                            exit_code: 0,
                            stdout: "duplicate".into(),
                            stderr: String::new(),
                            duration_ms: 1,
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    workflow_fact_log::read_tree_records(&fixture.store, execution_id)
                        .await
                        .unwrap(),
                    records
                );
            }
        }
    }

    #[tokio::test]
    async fn test_承認完了_追記結果不明でもcanonicalな状態へliveとactiveを収束する() {
        // Given
        for durable in [false, true] {
            let fixture = test_helpers::Fixture::new(0);
            let started = fixture
                .persist_started(
                    "  main:\n    session: {provider: claude}\n    completion: {require: approval}\n",
                    "/repo",
                )
                .await;
            let execution_id = &started.execution_id;
            let node = &started.node_executions[0];
            let timestamp = current_timestamp();
            fixture
                .host
                .write_log_required_batch(
                    &fixture.app,
                    &[
                        WorkflowEvent::NodeSubmitReceived {
                            execution_id: execution_id.clone(),
                            node_execution_id: node.id.clone(),
                            timestamp,
                        },
                        WorkflowEvent::NodeStopReceived {
                            execution_id: execution_id.clone(),
                            node_execution_id: node.id.clone(),
                            timestamp,
                        },
                    ],
                )
                .await
                .unwrap();
            fixture
                .host
                .register_started_execution_tree(&fixture.app, execution_id)
                .await
                .unwrap();
            let before = fixture
                .host
                .get_state_by_execution_id(&fixture.app, execution_id)
                .await
                .unwrap();
            assert_eq!(before.state, RuntimeExecutionState::Running);
            assert_eq!(
                before.node_executions[0].status,
                NodeExecutionStatus::WaitingApproval
            );
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                fixture.app.clone(),
                Arc::new(fixture.host.clone()),
            ));
            if durable {
                fixture.store.fault_injector().arm_drop_reply();
            } else {
                fixture.store.close_write_queue_for_tests();
            }

            // When
            let control_plane = WorkflowControlPlaneUsecase::new(
                crate::usecase::work_queue::shared().clone(),
                gateway,
            );
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                control_plane.resolve_approval(ApprovalCommand {
                    execution_id: execution_id.clone(),
                    node_name: node.node_name.clone(),
                    node_execution_id: Some(node.id.clone()),
                    comment: None,
                }),
            )
            .await;

            // Then
            if durable {
                result.unwrap().unwrap();
            } else {
                assert!(result.is_err());
            }
            let records = workflow_fact_log::read_tree_records(&fixture.store, execution_id)
                .await
                .unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ApprovalGranted(_)))
                    .count(),
                usize::from(durable)
            );
            assert_eq!(
                records
                    .iter()
                    .filter(|record| matches!(record.fact, NodeFact::ExecutionCompleted))
                    .count(),
                usize::from(durable)
            );
            let expected_state = if durable {
                RuntimeExecutionState::Completed
            } else {
                RuntimeExecutionState::Running
            };
            let current = fixture
                .host
                .get_state_by_execution_id(&fixture.app, execution_id)
                .await
                .unwrap();
            assert_eq!(current.state, expected_state);
            assert_eq!(
                current.node_executions[0].status,
                if durable {
                    NodeExecutionStatus::Succeeded
                } else {
                    NodeExecutionStatus::WaitingApproval
                }
            );
            let folded = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone()),
                execution_id,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(folded.aggregate.state(), &expected_state);
        }
    }

    #[tokio::test]
    async fn test_command_env_未束縛inputではprocessを起動せずnode_failureにする() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ))
        .unwrap();
        let app = test_helpers::dependencies(Some(store.clone()));
        let host = WorkflowRuntimeHost::with_runtime_ports(
            crate::usecase::work_queue::shared().clone(),
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            test_helpers::workspace_query(store.clone()),
            Arc::new(FailingWorkflowAgentSessions),
            Arc::new(test_helpers::TestWorktrees::default()),
        );
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: missing-command-env
description: missing command env
nodes:
  main:
    command: 'printf spawned > command-spawned.marker'
    input:
      - document
    env:
      DOCUMENT: document
"#,
        )
        .unwrap();

        let execution_id = host
            .start_resolved_workflow(
                &app,
                workflow,
                directory.path().to_string_lossy().into_owned(),
                None,
                ExecutionOrigin::DesktopUi,
            )
            .await
            .unwrap();

        assert!(!directory.path().join("command-spawned.marker").exists());
        test_helpers::wait_startup_retries(&host).await;
        let snapshot = host
            .get_state_by_execution_id(&app, &execution_id)
            .await
            .unwrap();
        assert_eq!(snapshot.node_executions.len(), 1);
        assert_eq!(
            snapshot.node_executions.last().unwrap().status,
            NodeExecutionStatus::Running
        );
        let records = workflow_fact_log::read_tree_records(&store, &execution_id)
            .await
            .unwrap();
        assert!(records.iter().any(|record| matches!(
            &record.fact,
            NodeFact::RuntimeFailureObserved(fact) if !fact.reason.is_empty()
        )));
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
        let restored = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(store),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            restored.aggregate.node_executions.last().unwrap().status,
            NodeExecutionStatus::Running
        );
        assert!(restored
            .aggregate
            .node_executions
            .last()
            .unwrap()
            .can_retry(crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent));
    }

    #[tokio::test]
    async fn test_command_env_nulによるspawn失敗を既存node_failureにする() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ))
        .unwrap();
        let app = test_helpers::dependencies(Some(store.clone()));
        let host = WorkflowRuntimeHost::with_runtime_ports(
            crate::usecase::work_queue::shared().clone(),
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            test_helpers::workspace_query(store.clone()),
            Arc::new(FailingWorkflowAgentSessions),
            Arc::new(test_helpers::TestWorktrees::default()),
        );
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: nul-command-env
description: nul command env
nodes:
  main:
    sequence:
      children:
        - run:
            inputs:
              document: request
  run:
    command: 'printf spawned > command-spawned.marker'
    input:
      - document
    env:
      DOCUMENT: document
"#,
        )
        .unwrap();

        let execution_id = host
            .start_resolved_workflow(
                &app,
                workflow,
                directory.path().to_string_lossy().into_owned(),
                Some("before\0after".to_string()),
                ExecutionOrigin::DesktopUi,
            )
            .await
            .unwrap();

        assert!(!directory.path().join("command-spawned.marker").exists());
        let snapshot = host
            .get_state_by_execution_id(&app, &execution_id)
            .await
            .unwrap();
        assert_eq!(
            snapshot
                .node_executions
                .iter()
                .rev()
                .find(|node| node.node_name == "run")
                .map(|node| node.status),
            Some(NodeExecutionStatus::Running)
        );
        let records = workflow_fact_log::read_tree_records(&store, &execution_id)
            .await
            .unwrap();
        assert!(records.iter().any(|record| matches!(
            &record.fact,
            NodeFact::RuntimeFailureObserved(fact) if !fact.reason.is_empty()
        )));
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
        let restored = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(store),
            &execution_id,
        )
        .await
        .unwrap()
        .unwrap();
        let failed = restored
            .aggregate
            .node_executions
            .iter()
            .rev()
            .find(|node| node.node_name == "run")
            .unwrap();
        assert_eq!(failed.status, NodeExecutionStatus::Running);
        assert!(failed.can_retry(crate::domain::workflow::NodeProcessPresence::ConfirmedAbsent));
    }

    #[async_trait::async_trait]
    impl WorkflowAgentSessionPort for FailingWorkflowAgentSessions {
        async fn has_recoverable_conversation(
            &self,
            _id: &str,
        ) -> Result<bool, WorkflowRuntimeError> {
            Ok(true)
        }

        fn is_provider_available(&self, _provider: ProviderKind) -> bool {
            true
        }

        async fn prepare_workflow_agent_session(
            &self,
            _workspace_worktree_path: &str,
            _worktree_path: &str,
            _config: WorkflowSessionLaunchConfig,
            _workflow_execution_id: &str,
            _node_execution_id: &str,
            _initial_instruction: &str,
        ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
            Err(WorkflowRuntimeError::AgentSession(
                "intentional prepare failure".to_string(),
            ))
        }

        async fn activate_workflow_agent_session(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn confirm_workflow_agent_session_attachment(
            &self,
            _node_session_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn dispatch_continuation(
            &self,
            _node_session_id: &str,
            _child_execution_id: &str,
            _instruction: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            panic!("unexpected delegate continuation")
        }

        async fn recover_workflow_agent_session_provider(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn rollback_workflow_agent_session(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }
    }

    mod runtime_effect_tests {
        use super::*;

        fn recording_agent_sessions(
            stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
            provider_running_checks: Arc<std::sync::Mutex<Vec<(String, String)>>>,
            recovery_fails: Arc<std::sync::atomic::AtomicBool>,
            failing_agent_session_id: String,
        ) -> Arc<dyn WorkflowAgentSessionPort> {
            Arc::new(RecordingWorkflowAgentSessions {
                stop_calls,
                prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                provider_running_checks,
                recovery_fails,
                failing_agent_session_id,
            })
        }

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for RecordingWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                config: WorkflowSessionLaunchConfig,
                workflow_execution_id: &str,
                node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                self.prepare_calls.lock().unwrap().push((
                    workflow_execution_id.to_string(),
                    node_execution_id.to_string(),
                    config,
                ));
                Ok(NodeSessionInfo {
                    id: EFFECT_AGENT_SESSION_ID.to_string(),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.provider_running_checks
                    .lock()
                    .unwrap()
                    .push((node_execution_id.to_string(), node_session_id.to_string()));
                if self
                    .recovery_fails
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(WorkflowRuntimeError::AgentSession(
                        "intentional provider recovery failure".to_string(),
                    ));
                }
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.stop_calls
                    .lock()
                    .unwrap()
                    .push((node_execution_id.to_string(), node_session_id.to_string()));
                if node_session_id == self.failing_agent_session_id {
                    return Err(WorkflowRuntimeError::AgentSession(
                        "intentional stop failure".to_string(),
                    ));
                }
                Ok(())
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        struct StopDuringActivationWorkflowAgentSessions {
            control_plane: tokio::sync::Mutex<Option<Arc<WorkflowControlPlaneUsecase>>>,
            execution_id: std::sync::Mutex<Option<String>>,
            activation_count: std::sync::atomic::AtomicUsize,
            confirmation_count: std::sync::atomic::AtomicUsize,
        }

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for StopDuringActivationWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                workflow_execution_id: &str,
                _node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                *self.execution_id.lock().unwrap() = Some(workflow_execution_id.to_string());
                Ok(NodeSessionInfo {
                    id: EFFECT_AGENT_SESSION_ID.to_string(),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.activation_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let control_plane = self
                    .control_plane
                    .lock()
                    .await
                    .clone()
                    .expect("control plane is bound before activation");
                let execution_id = self
                    .execution_id
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("execution id is recorded during prepare");
                control_plane
                    .record_provider_stop(
                        ProviderExecutionTreeStopCommand {
                            agent_session_id: node_session_id.to_string(),
                            tree_id: execution_id,
                            node_execution_id: node_execution_id.to_string(),
                            binding_id: "binding-stop-during-activation".to_string(),
                        },
                        Vec::new(),
                    )
                    .await
                    .map_err(|error| {
                        WorkflowRuntimeError::InvalidState(format!(
                            "provider Stop during activation was rejected: {error}"
                        ))
                    })
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                assert_eq!(
                    self.activation_count
                        .load(std::sync::atomic::Ordering::SeqCst),
                    1
                );
                self.confirmation_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        #[derive(Debug, Clone, PartialEq, Eq)]
        enum RuntimeEffectCall {
            Activate {
                node_execution_id: String,
                agent_session_id: String,
            },
            Stop {
                node_execution_id: String,
                agent_session_id: String,
            },
        }

        struct OrderedWorkflowAgentSessions {
            calls: Arc<std::sync::Mutex<Vec<RuntimeEffectCall>>>,
        }

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for OrderedWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                _workflow_execution_id: &str,
                node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                Ok(NodeSessionInfo {
                    id: format!("agent-session-{node_execution_id}"),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.calls
                    .lock()
                    .unwrap()
                    .push(RuntimeEffectCall::Activate {
                        node_execution_id: node_execution_id.to_string(),
                        agent_session_id: node_session_id.to_string(),
                    });
                Ok(())
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.calls.lock().unwrap().push(RuntimeEffectCall::Stop {
                    node_execution_id: node_execution_id.to_string(),
                    agent_session_id: node_session_id.to_string(),
                });
                Ok(())
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        struct NeverResolvingStopWorkflowAgentSessions;

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for NeverResolvingStopWorkflowAgentSessions {
            async fn has_recoverable_conversation(
                &self,
                _id: &str,
            ) -> Result<bool, WorkflowRuntimeError> {
                Ok(true)
            }

            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _workspace_worktree_path: &str,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                _workflow_execution_id: &str,
                _node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                Ok(NodeSessionInfo {
                    id: EFFECT_AGENT_SESSION_ID.to_string(),
                })
            }

            async fn activate_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn confirm_workflow_agent_session_attachment(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn dispatch_continuation(
                &self,
                _node_session_id: &str,
                _child_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                panic!("unexpected delegate continuation")
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                std::future::pending::<()>().await;
                unreachable!()
            }

            async fn rollback_workflow_agent_session(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }
        }

        struct RuntimeEffectFixture {
            app: WorkflowRuntimeDependencies,
            store: Arc<LocalEventStore>,
            fault: Arc<FaultInjector>,
            host: Arc<WorkflowRuntimeHost>,
            control_plane: WorkflowControlPlaneUsecase,
            stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
            execution_id: String,
            node_execution_id: String,
            _directory: tempfile::TempDir,
        }

        struct SequentialRuntimeEffectFixture {
            _app: WorkflowRuntimeDependencies,
            host: Arc<WorkflowRuntimeHost>,
            control_plane: WorkflowControlPlaneUsecase,
            calls: Arc<std::sync::Mutex<Vec<RuntimeEffectCall>>>,
            execution_id: String,
            first_node_execution_id: String,
            first_agent_session_id: String,
            _directory: tempfile::TempDir,
        }

        async fn runtime_effect_fixture(
            completion: NodeCompletion,
            stop_fails: bool,
        ) -> RuntimeEffectFixture {
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sessions = recording_agent_sessions(
                stop_calls.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                if stop_fails {
                    EFFECT_AGENT_SESSION_ID.to_string()
                } else {
                    String::new()
                },
            );
            runtime_effect_fixture_with_sessions(completion, sessions, stop_calls).await
        }

        async fn runtime_effect_fixture_with_sessions(
            completion: NodeCompletion,
            sessions: Arc<dyn WorkflowAgentSessionPort>,
            stop_calls: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        ) -> RuntimeEffectFixture {
            let directory = tempfile::tempdir().unwrap();
            let fault = Arc::new(FaultInjector::new());
            let mut config = LocalEventStoreConfig::production(directory.path().to_path_buf());
            config.fault = fault.clone();
            let store = LocalEventStore::open(config).unwrap();
            let app = test_helpers::dependencies(Some(store.clone()));
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                sessions,
                Arc::new(test_helpers::TestWorktrees::default()),
            ));
            let nodes = vec![NodeDefinition {
                name: EFFECT_NODE_NAME.to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider: ProviderKind::Codex,
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
            }];
            let workflow = WorkflowDefinition {
                name: "runtime-effect-test".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes,
                entry: EFFECT_NODE_NAME.to_string(),
            };
            let execution_id = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host
                .get_state_by_execution_id(&app, &execution_id)
                .await
                .unwrap();
            let node_execution_id = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == EFFECT_NODE_NAME)
                .unwrap()
                .id
                .clone();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.id == node_execution_id)
                .unwrap();
            assert_eq!(
                node.status,
                NodeExecutionStatus::Running,
                "unexpected activation state"
            );
            assert_eq!(node.session_id.as_deref(), Some(EFFECT_AGENT_SESSION_ID));
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                app.clone(),
                host.clone(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(
                crate::usecase::work_queue::shared().clone(),
                gateway,
            )
            .with_startup(crate::adaptor::controller::wiring::wire_workflow_startup(
                crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                    crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
                )),
                app.clone(),
                host.clone(),
            ));
            RuntimeEffectFixture {
                app,
                store,
                fault,
                host,
                control_plane,
                stop_calls,
                execution_id,
                node_execution_id,
                _directory: directory,
            }
        }

        async fn sequential_runtime_effect_fixture() -> SequentialRuntimeEffectFixture {
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let app = test_helpers::dependencies(Some(store.clone()));
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                Arc::new(OrderedWorkflowAgentSessions {
                    calls: calls.clone(),
                }),
                Arc::new(test_helpers::TestWorktrees::default()),
            ));
            let session_node = |name: &str| NodeDefinition {
                name: name.to_string(),
                kind: NodeKind::Session(SessionSpec {
                    provider: ProviderKind::Codex,
                    model: None,
                    permission: None,
                    facets: FacetRefs {
                        instruction: Some("policy-confirmation".to_string()),
                        ..FacetRefs::default()
                    },
                }),
                artifact: None,
                input: Vec::new(),
                completion: NodeCompletion::default(),
                worktree: None,
            };
            let workflow = WorkflowDefinition {
                name: "runtime-effect-order-test".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![
                    NodeDefinition {
                        name: "main".to_string(),
                        kind: NodeKind::Sequence(SequenceSpec {
                            entry: None,
                            children: vec![
                                ChildEntry::reference("agent-one"),
                                ChildEntry::reference("agent-two"),
                            ],
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: NodeCompletion::default(),
                        worktree: None,
                    },
                    session_node("agent-one"),
                    session_node("agent-two"),
                ],
                entry: "main".to_string(),
            };
            let execution_id = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host
                .get_state_by_execution_id(&app, &execution_id)
                .await
                .unwrap();
            let first = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "agent-one")
                .unwrap();
            assert_eq!(first.status, NodeExecutionStatus::Running);
            let first_node_execution_id = first.id.clone();
            let first_agent_session_id = first.session_id.clone().unwrap();
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                app.clone(),
                host.clone(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(
                crate::usecase::work_queue::shared().clone(),
                gateway,
            )
            .with_startup(crate::adaptor::controller::wiring::wire_workflow_startup(
                crate::usecase::work_queue::WorkQueueUsecase::new(std::sync::Arc::new(
                    crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
                )),
                app.clone(),
                host.clone(),
            ));
            SequentialRuntimeEffectFixture {
                _app: app,
                host,
                control_plane,
                calls,
                execution_id,
                first_node_execution_id,
                first_agent_session_id,
                _directory: directory,
            }
        }

        fn provider_stop_command(
            fixture: &RuntimeEffectFixture,
        ) -> ProviderExecutionTreeStopCommand {
            ProviderExecutionTreeStopCommand {
                agent_session_id: EFFECT_AGENT_SESSION_ID.to_string(),
                tree_id: fixture.execution_id.clone(),
                node_execution_id: fixture.node_execution_id.clone(),
                binding_id: "binding-effect-test".to_string(),
            }
        }

        async fn persisted_node_status(fixture: &RuntimeEffectFixture) -> NodeExecutionStatus {
            persisted_node(fixture).await.status
        }

        async fn persisted_node(
            fixture: &RuntimeEffectFixture,
        ) -> crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution {
            persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &fixture.node_execution_id,
            )
            .await
        }

        async fn persisted_node_for(
            store: &Arc<LocalEventStore>,
            execution_id: &str,
            node_execution_id: &str,
        ) -> crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution {
            let backend = workflow_fact_log::FactLogReadBackend::Live(store.clone());
            workflow_fact_log::fold_tree_from(&backend, execution_id)
                .await
                .unwrap()
                .unwrap()
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == node_execution_id)
                .unwrap()
                .clone()
        }

        #[tokio::test]
        async fn test_deleted実行木解放_facet本文を除去する() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            assert!(fixture
                .host
                .execution_facet_contents
                .lock()
                .await
                .contains_key(&fixture.execution_id));
            fixture
                .host
                .release_deleted_execution_tree(&fixture.execution_id)
                .await
                .unwrap();
            assert!(!fixture
                .host
                .execution_facet_contents
                .lock()
                .await
                .contains_key(&fixture.execution_id));
        }

        #[tokio::test]
        async fn test_started実行木登録_store未管理ならsession_storeを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let unmanaged_app = test_helpers::dependencies(None);

            let error = fixture
                .host
                .register_started_execution_tree(&unmanaged_app, "unmanaged-tree")
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
        }

        #[tokio::test]
        async fn test_started実行木登録_tree不在ならexecution_not_foundを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let missing_tree_id = "missing-started-tree";

            let error = fixture
                .host
                .register_started_execution_tree(&fixture.app, missing_tree_id)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                WorkflowRuntimeError::ExecutionNotFound(tree_id) if tree_id == missing_tree_id
            ));
        }

        #[tokio::test]
        async fn test_started実行木登録_inactive_treeならinvalid_stateを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let session_id = "inactive-started-tree";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-inactive-started-tree",
                )
                .await
                .unwrap();
            workflow_fact_log::append_facts_for_events(
                &fixture.store,
                &[WorkflowEvent::ExecutionAborted {
                    execution_id: session_id.to_string(),
                    aborted_node: None,
                    timestamp: 2.0,
                }],
            )
            .await
            .unwrap();

            let error = fixture
                .host
                .register_started_execution_tree(&fixture.app, session_id)
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::InvalidState(_)));
        }

        #[tokio::test]
        async fn test_session実行木のreconciliationは喪失を記録せずstopを保持する() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let session_id = "agent-session-reserved-before-commit";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-reserved-before-commit",
                )
                .await
                .unwrap();

            test_helpers::reconcile_startup(&fixture.host, &fixture.app)
                .await
                .unwrap();

            let records = workflow_fact_log::read_tree_records(&fixture.store, session_id)
                .await
                .unwrap();
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
            fixture
                .host
                .register_started_execution_tree(&fixture.app, session_id)
                .await
                .unwrap();

            fixture
                .control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: session_id.to_string(),
                        tree_id: session_id.to_string(),
                        node_execution_id: session_id.to_string(),
                        binding_id: "binding-reserved-before-commit".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();

            let records = workflow_fact_log::read_tree_records(&fixture.store, session_id)
                .await
                .unwrap();
            assert!(records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::StopReceived(_))));
            let backend = workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone());
            let folded = workflow_fact_log::fold_tree_from(&backend, session_id)
                .await
                .unwrap()
                .unwrap();
            let node = folded
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == session_id)
                .unwrap();
            assert_eq!(
                node.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
            assert_eq!(node.status, NodeExecutionStatus::Running);
            let workspace_node = SqliteWorkspaceTreeRepository::new(fixture.store.clone())
                .load_node_by_node_execution_id(session_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                workspace_node.status_classification,
                WorkspaceNodeStatusClassification::Attention
            );

            let restarted = WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                test_helpers::workspace_query(fixture.store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway),
            );
            test_helpers::reconcile_startup(&restarted, &fixture.app)
                .await
                .unwrap();

            let restarted_fold = workflow_fact_log::fold_tree_from(&backend, session_id)
                .await
                .unwrap()
                .unwrap();
            let restarted_node = restarted_fold
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == session_id)
                .unwrap();
            assert_eq!(
                restarted_node.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
            assert_eq!(
                SqliteWorkspaceTreeRepository::new(fixture.store.clone())
                    .load_node_by_node_execution_id(session_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .status_classification,
                WorkspaceNodeStatusClassification::Attention
            );
            assert!(
                !workflow_fact_log::read_tree_records(&fixture.store, session_id)
                    .await
                    .unwrap()
                    .iter()
                    .any(|record| matches!(record.fact, NodeFact::ProcessExited(_)))
            );
        }

        #[tokio::test]
        async fn test_session実行木登録失敗後もreconciliationはプロセス喪失を記録しない() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let session_id = "agent-session-registration-failed";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-registration-failed",
                )
                .await
                .unwrap();
            let unmanaged_app = test_helpers::dependencies(None);
            assert!(fixture
                .host
                .register_started_execution_tree(&unmanaged_app, session_id)
                .await
                .is_err());

            test_helpers::reconcile_startup(&fixture.host, &fixture.app)
                .await
                .unwrap();

            let records = workflow_fact_log::read_tree_records(&fixture.store, session_id)
                .await
                .unwrap();
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
        }

        #[tokio::test]
        async fn test_session起動_provider起動時にはattach済みでstop_receivedになる() {
            // Given
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let app = test_helpers::dependencies(Some(store.clone()));
            let sessions = Arc::new(StopDuringActivationWorkflowAgentSessions {
                control_plane: tokio::sync::Mutex::new(None),
                execution_id: std::sync::Mutex::new(None),
                activation_count: std::sync::atomic::AtomicUsize::new(0),
                confirmation_count: std::sync::atomic::AtomicUsize::new(0),
            });
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                sessions.clone(),
                Arc::new(test_helpers::TestWorktrees::default()),
            ));
            let gateway = Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
                app.clone(),
                host.clone(),
            ));
            *sessions.control_plane.lock().await =
                Some(Arc::new(WorkflowControlPlaneUsecase::new(
                    crate::usecase::work_queue::shared().clone(),
                    gateway,
                )));
            let workflow = WorkflowDefinition {
                name: "stop-during-activation".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        provider: ProviderKind::Codex,
                        model: None,
                        permission: None,
                        facets: FacetRefs {
                            instruction: Some("policy-confirmation".to_string()),
                            ..FacetRefs::default()
                        },
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::default(),
                    worktree: None,
                }],
                entry: "main".to_string(),
            };

            // When
            let execution_id = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();

            // Then
            let snapshot = host
                .get_state_by_execution_id(&app, &execution_id)
                .await
                .unwrap();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "main")
                .unwrap();
            assert_eq!(
                node.status,
                NodeExecutionStatus::Running,
                "unexpected activation state"
            );
            assert_eq!(
                node.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
            assert_eq!(node.session_id.as_deref(), Some(EFFECT_AGENT_SESSION_ID));
            assert_eq!(
                sessions
                    .confirmation_count
                    .load(std::sync::atomic::Ordering::SeqCst),
                1
            );
            let records = workflow_fact_log::read_tree_records(&store, &execution_id)
                .await
                .unwrap();
            let attached_seq = records
                .iter()
                .find_map(|record| match &record.fact {
                    NodeFact::SessionAttached(attached)
                        if attached.session_id == EFFECT_AGENT_SESSION_ID =>
                    {
                        Some(record.seq)
                    }
                    _ => None,
                })
                .unwrap();
            let stop_seq = records
                .iter()
                .find_map(|record| {
                    matches!(record.fact, NodeFact::StopReceived(_)).then_some(record.seq)
                })
                .unwrap();
            assert!(attached_seq < stop_seq);
        }

        #[tokio::test]
        async fn test_provider_stopはlaunch区分の異なる実行木で同じsignal遷移になる() {
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            let standalone_id = "agent-session-standalone-stop";
            LocalAgentSessionRepository::new(fixture.store.clone())
                .create(
                    AgentSession::create(
                        standalone_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(standalone_id).unwrap(),
                    )
                    .unwrap(),
                    "create-standalone-stop",
                )
                .await
                .unwrap();
            fixture
                .host
                .register_started_execution_tree(&fixture.app, standalone_id)
                .await
                .unwrap();

            fixture
                .control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: standalone_id.to_string(),
                        tree_id: standalone_id.to_string(),
                        node_execution_id: standalone_id.to_string(),
                        binding_id: "binding-standalone-stop".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();

            let backend = workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone());
            for (tree_id, node_execution_id) in [
                (standalone_id, standalone_id),
                (
                    fixture.execution_id.as_str(),
                    fixture.node_execution_id.as_str(),
                ),
            ] {
                let folded = workflow_fact_log::fold_tree_from(&backend, tree_id)
                    .await
                    .unwrap()
                    .unwrap();
                let node = folded
                    .aggregate
                    .node_executions
                    .iter()
                    .find(|node| node.id == node_execution_id)
                    .unwrap();
                assert_eq!(
                    node.completion_signals,
                    crate::domain::workflow::NodeCompletionSignalState::StopReceived
                );
                assert_eq!(node.status, NodeExecutionStatus::Running);
            }
        }

        #[tokio::test]
        async fn test_session起動由来のactive木と同一worktreeでworkflowを起動できる() {
            // Given: 同じ worktree に active な Session 起動由来の木が登録されている
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let app = test_helpers::dependencies(Some(store.clone()));
            let session_id = "agent-session-workflow-coexistence";
            LocalAgentSessionRepository::new(store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-session-workflow-coexistence",
                )
                .await
                .unwrap();
            let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                Arc::new(RecordingWorkflowAgentSessions {
                    stop_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    provider_running_checks: Arc::new(std::sync::Mutex::new(Vec::new())),
                    recovery_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    failing_agent_session_id: String::new(),
                }),
                Arc::new(test_helpers::TestWorktrees::default()),
            ));
            host.register_started_execution_tree(&app, session_id)
                .await
                .unwrap();
            let workflow = WorkflowDefinition {
                name: "coexisting-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Session(SessionSpec {
                        provider: ProviderKind::Codex,
                        model: None,
                        permission: None,
                        facets: FacetRefs {
                            instruction: Some("policy-confirmation".to_string()),
                            ..FacetRefs::default()
                        },
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::default(),
                    worktree: None,
                }],
                entry: "main".to_string(),
            };

            // When: workflow の実行として同じ worktree に木を起こす
            let workflow_id = host
                .start_resolved_workflow(
                    &app,
                    workflow.clone(),
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();

            // Then: cache は両方を保持し、workflow registry は workflow だけを保持する
            assert!(host
                .get_state_by_execution_id(&app, session_id)
                .await
                .is_some());
            assert!(host
                .get_state_by_execution_id(&app, &workflow_id)
                .await
                .is_some());
            let second = host
                .start_resolved_workflow(
                    &app,
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await;
            assert!(matches!(
                second,
                Err(WorkflowRuntimeError::AlreadyActive(_))
            ));
        }

        async fn wait_for_single_terminal_stop(fixture: &RuntimeEffectFixture) {
            let observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if !fixture.stop_calls.lock().unwrap().is_empty() {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await;
            assert!(
                observed.is_ok(),
                "terminal stop effect must run after durable commit"
            );
            assert_eq!(
                fixture.stop_calls.lock().unwrap().as_slice(),
                &[(
                    fixture.node_execution_id.clone(),
                    EFFECT_AGENT_SESSION_ID.to_string(),
                )]
            );
        }

        #[tokio::test]
        async fn test_provider_stop_provider_lifecycle_commit失敗後も停止effectを実行する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            fixture.fault.arm_fail_before_commit();
            let scope = ProviderLifecycleScope::new(EFFECT_AGENT_SESSION_ID).unwrap();
            let lifecycle_events = vec![ScopedProviderLifecycleEvent::new(
                scope,
                ProviderLifecycleEvent::stop_observed("binding-effect-test").unwrap(),
            )];

            // When
            let result = fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), lifecycle_events)
                .await;

            // Then
            assert!(result.is_ok(), "unexpected provider Stop error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
            let page = fixture
                .store
                .load_stream(LoadStreamRequest {
                    stream_id: StreamId::provider_lifecycle(EFFECT_AGENT_SESSION_ID).unwrap(),
                    after: None,
                    limit: 10,
                })
                .await
                .unwrap();
            assert!(page.events.is_empty());
        }

        #[tokio::test]
        async fn test_submit_agent_session停止失敗でも成功とsucceededを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), true).await;
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();

            // When
            let result = fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await;

            // Then
            assert!(result.is_ok(), "unexpected Submit error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_session終端_後続activateを停止完了に依存させず両方を実行する() {
            // Given
            let fixture = sequential_runtime_effect_fixture().await;
            fixture.calls.lock().unwrap().clear();
            fixture
                .control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: fixture.first_agent_session_id.clone(),
                        tree_id: fixture.execution_id.clone(),
                        node_execution_id: fixture.first_node_execution_id.clone(),
                        binding_id: "binding-order-test".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            assert!(fixture.calls.lock().unwrap().is_empty());

            // When
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.first_node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();

            // Then
            let snapshot = fixture
                .host
                .get_state_by_execution_id(&fixture._app, &fixture.execution_id)
                .await
                .unwrap();
            let second = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "agent-two")
                .unwrap();
            assert_eq!(second.status, NodeExecutionStatus::Running);
            let activate = RuntimeEffectCall::Activate {
                node_execution_id: second.id.clone(),
                agent_session_id: second.session_id.clone().unwrap(),
            };
            assert!(
                fixture.calls.lock().unwrap().contains(&activate),
                "Submit acceptance must activate the next Session without waiting for the stop effect"
            );
            let expected_stop = RuntimeEffectCall::Stop {
                node_execution_id: fixture.first_node_execution_id.clone(),
                agent_session_id: fixture.first_agent_session_id.clone(),
            };
            let observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if fixture.calls.lock().unwrap().contains(&expected_stop) {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await;
            assert!(
                observed.is_ok(),
                "terminal stop effect must run after durable commit"
            );
        }

        #[tokio::test]
        async fn test_provider_stop受理_停止effect未完了でもcommitと後続処理が完了する() {
            // Given
            let fixture = runtime_effect_fixture_with_sessions(
                NodeCompletion::default(),
                Arc::new(NeverResolvingStopWorkflowAgentSessions),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )
            .await;
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();

            // When
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                fixture
                    .control_plane
                    .record_provider_stop(provider_stop_command(&fixture), Vec::new()),
            )
            .await;

            // Then
            let result =
                result.expect("provider Stop acceptance must not block on the session stop effect");
            assert!(result.is_ok(), "unexpected provider Stop error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
        }

        #[tokio::test]
        async fn test_終端済みsessionへの再stop_確定状態と停止回数を変えない() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), false).await;
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;

            // When
            let result = fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await;

            // Then
            assert!(result.is_ok(), "unexpected repeated Stop error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_承認_agent_session停止失敗でも成功とsucceededを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::require_approval(), true).await;
            fixture
                .control_plane
                .submit_output(SubmitOutputCommand {
                    node_execution_id: fixture.node_execution_id.clone(),
                    artifact: None,
                })
                .await
                .unwrap();
            fixture
                .control_plane
                .record_provider_stop(provider_stop_command(&fixture), Vec::new())
                .await
                .unwrap();
            let waiting = fixture
                .host
                .get_state_by_execution_id(&fixture.app, &fixture.execution_id)
                .await
                .unwrap();
            assert_eq!(
                waiting
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::WaitingApproval
            );

            // When
            let result = fixture
                .control_plane
                .resolve_approval(ApprovalCommand {
                    execution_id: fixture.execution_id.clone(),
                    node_name: EFFECT_NODE_NAME.to_string(),
                    node_execution_id: Some(fixture.node_execution_id.clone()),
                    comment: None,
                })
                .await;

            // Then
            assert!(result.is_ok(), "unexpected approval error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_failure_settlement_異常の記録はsessionをrunningのまま維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), true).await;
            let runtime_error = WorkflowRuntimeError::AgentSession("runtime failed".to_string());

            // When
            let result = fixture
                .host
                .settle_runtime_failure_for_node(
                    &fixture.app,
                    &fixture.execution_id,
                    &fixture.node_execution_id,
                    &runtime_error,
                )
                .await;

            // Then
            assert!(
                result.is_ok(),
                "unexpected failure settlement error: {result:?}"
            );
            let settled = fixture
                .host
                .get_state_by_execution_id(&fixture.app, &fixture.execution_id)
                .await
                .unwrap();
            assert_eq!(
                settled
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Running
            );
            assert_eq!(fixture.stop_calls.lock().unwrap().len(), 0);
        }

        #[tokio::test]
        async fn test_abort_agent_session停止失敗でも成功とabortedを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::default(), true).await;

            // When
            let result = fixture
                .host
                .abort_workflow_execution(&fixture.app, &fixture.execution_id, None)
                .await;

            // Then
            assert!(result.is_ok(), "unexpected abort error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture).await,
                NodeExecutionStatus::Aborted
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_committed_runtime_effects_停止失敗後も残りのagent_sessionを停止する() {
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sessions: Arc<dyn WorkflowAgentSessionPort> =
                Arc::new(RecordingWorkflowAgentSessions {
                    stop_calls: stop_calls.clone(),
                    prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    provider_running_checks: Arc::new(std::sync::Mutex::new(Vec::new())),
                    recovery_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    failing_agent_session_id: "agent-session-1".to_string(),
                });

            WorkflowRuntimeHost::run_committed_runtime_effects(
                sessions,
                vec![
                    WorkflowRuntimeEffect::BroadcastState,
                    WorkflowRuntimeEffect::StopWorkflowAgentSession {
                        node_execution_id: "node-1".to_string(),
                        agent_session_id: "agent-session-1".to_string(),
                    },
                    WorkflowRuntimeEffect::StopWorkflowAgentSession {
                        node_execution_id: "node-2".to_string(),
                        agent_session_id: "agent-session-2".to_string(),
                    },
                ],
            )
            .await;

            assert_eq!(
                stop_calls.lock().unwrap().as_slice(),
                &[
                    ("node-1".to_string(), "agent-session-1".to_string()),
                    ("node-2".to_string(), "agent-session-2".to_string()),
                ]
            );
        }
    }

    mod startup_recovery_tests {
        use super::*;

        #[tokio::test]
        async fn test_startup_reconciliation_stop受信済みsession木をcacheへ載せてleafを再起動しない(
        ) {
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let session_id = "agent-session-startup";
            LocalAgentSessionRepository::new(store.clone())
                .create(
                    AgentSession::create(
                        session_id,
                        WorkspaceIdentity::new("/repo/session-startup"),
                        "/repo/session-startup",
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(session_id).unwrap(),
                    )
                    .unwrap(),
                    "create-startup-session",
                )
                .await
                .unwrap();
            workflow_fact_log::append_facts_for_events(
                &store,
                &[WorkflowEvent::NodeStopReceived {
                    execution_id: session_id.to_string(),
                    node_execution_id: session_id.to_string(),
                    timestamp: 2.0,
                }],
            )
            .await
            .unwrap();
            let before = workflow_fact_log::read_tree_records(&store, session_id)
                .await
                .unwrap()
                .len();
            let app = test_helpers::dependencies(Some(store.clone()));
            let host = WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway),
            );

            test_helpers::reconcile_startup(&host, &app).await.unwrap();

            let snapshot = host
                .get_state_by_execution_id(&app, session_id)
                .await
                .unwrap();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.id == session_id)
                .unwrap();
            assert_eq!(
                node.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
            assert_eq!(
                workflow_fact_log::read_tree_records(&store, session_id)
                    .await
                    .unwrap()
                    .len(),
                before
            );
        }

        async fn append_started_session_tree(
            store: &Arc<LocalEventStore>,
            tree_id: &str,
            worktree_path: &str,
            timestamp_ms: i64,
        ) {
            let definition = WorkflowDefinition {
                name: format!("workflow-{tree_id}"),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![
                    NodeDefinition {
                        name: "main".to_string(),
                        kind: NodeKind::Sequence(SequenceSpec {
                            entry: None,
                            children: vec![ChildEntry::reference("impl")],
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: crate::domain::workflow::NodeCompletion::default(),
                        worktree: None,
                    },
                    NodeDefinition {
                        name: "impl".to_string(),
                        kind: NodeKind::Session(SessionSpec {
                            provider: ProviderKind::Codex,
                            model: None,
                            permission: None,
                            facets: Default::default(),
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: crate::domain::workflow::NodeCompletion::default(),
                        worktree: None,
                    },
                ],
                entry: "main".to_string(),
            };
            let root_meta = NodeFactMeta {
                tree_id: tree_id.to_string(),
                node_execution_id: tree_id.to_string(),
                parent_id: None,
                node_name: "main".to_string(),
                kind: NodeKindName::Sequence,
                attempt: 1,
            };
            workflow_fact_log::append_single_fact(
                store,
                &root_meta,
                &NodeFact::Started(StartedFact {
                    worktree: None,
                    parent: None,
                    root: Some(Box::new(TreeRootFact {
                        repository_root: None,
                        workspace_identity: worktree_path.to_string(),
                        worktree_path: worktree_path.to_string(),
                        created_from: ExecutionOrigin::DesktopUi,
                        request: String::new(),
                        workflow_name: definition.name.clone(),
                        definition: Some(definition),
                        launched_as: ExecutionTreeLaunch::Workflow,
                    })),
                }),
                timestamp_ms,
            )
            .await
            .unwrap();
            let child_meta = NodeFactMeta {
                tree_id: tree_id.to_string(),
                node_execution_id: format!("{tree_id}-session"),
                parent_id: Some(tree_id.to_string()),
                node_name: "impl".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
            };
            workflow_fact_log::append_single_fact(
                store,
                &child_meta,
                &NodeFact::Started(StartedFact {
                    worktree: None,
                    parent: Some(ExecutionParentRef::sequence_child(tree_id)),
                    root: None,
                }),
                timestamp_ms + 1,
            )
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn test_startup_reconciliation_壊れたtreeの後続treeも処理する() {
            const CORRUPT_TREE_ID: &str = "00000000-0000-4000-8000-000000000001";
            const VALID_TREE_ID: &str = "00000000-0000-4000-8000-000000000002";

            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            append_started_session_tree(&store, CORRUPT_TREE_ID, "/repo/corrupt", 1).await;
            store
                .append_node_event(
                    NewNodeEventRow {
                        tree_id: CORRUPT_TREE_ID.to_string(),
                        node_execution_id: CORRUPT_TREE_ID.to_string(),
                        parent_id: None,
                        node_name: "main".to_string(),
                        kind: "session".to_string(),
                        attempt: 1,
                        event_type: "submit_received".to_string(),
                        session_id: None,
                        detail: "{".to_string(),
                    },
                    Some(4),
                )
                .await
                .unwrap();
            append_started_session_tree(&store, VALID_TREE_ID, "/repo/valid", 5).await;
            let valid_records = workflow_fact_log::read_tree_records(&store, VALID_TREE_ID)
                .await
                .unwrap();
            workflow_fact_log::append_facts_for_events(
                &store,
                &[WorkflowEvent::SessionAttached {
                    execution_id: VALID_TREE_ID.into(),
                    node_execution_id: valid_records[1].meta.node_execution_id.clone(),
                    session_id: "already-started".into(),
                    timestamp: 0.006,
                }],
            )
            .await
            .unwrap();
            let corrupt_count = workflow_fact_log::read_tree_records(&store, CORRUPT_TREE_ID)
                .await
                .unwrap_err();
            assert!(corrupt_count.to_string().contains("decode"));
            let valid_count = workflow_fact_log::read_tree_records(&store, VALID_TREE_ID)
                .await
                .unwrap()
                .len();

            let app = test_helpers::dependencies(Some(store.clone()));
            let host = WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway),
            );

            let error = test_helpers::reconcile_startup(&host, &app)
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
            assert_eq!(
                workflow_fact_log::read_tree_records(&store, VALID_TREE_ID)
                    .await
                    .unwrap()
                    .len(),
                valid_count
            );
        }

        #[tokio::test]
        async fn test_startup_reconciliation_未対応permissionはabortせず要対応を記録する() {
            const TREE_ID: &str = "00000000-0000-4000-8000-000000000004";
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let mut fact = SessionExecutionTreeRootFacts::new(
                TREE_ID,
                "/repo",
                "/repo",
                ProviderKind::Claude,
                None,
            )
            .unwrap()
            .started;
            let NodeFact::Started(StartedFact {
                worktree: None,
                root: Some(root),
                ..
            }) = &mut fact
            else {
                unreachable!();
            };
            let NodeKind::Session(spec) = &mut root.definition.as_mut().unwrap().nodes[0].kind
            else {
                unreachable!();
            };
            spec.permission = Some(SessionPermission::Auto);
            let legacy_detail = fact_codec::encode_detail(&fact).unwrap().replace(
                r#""permission":"auto""#,
                r#""permission":"bypassPermissions""#,
            );
            store
                .append_node_event(
                    NewNodeEventRow {
                        tree_id: TREE_ID.to_string(),
                        node_execution_id: TREE_ID.to_string(),
                        parent_id: None,
                        node_name: "session".to_string(),
                        kind: "session".to_string(),
                        attempt: 1,
                        event_type: "started".to_string(),
                        session_id: None,
                        detail: legacy_detail,
                    },
                    Some(1),
                )
                .await
                .unwrap();

            assert!(workflow_fact_log::read_tree_records(&store, TREE_ID)
                .await
                .is_err());

            let app = test_helpers::dependencies(Some(store.clone()));
            let host = WorkflowRuntimeHost::with_runtime_ports(
                crate::usecase::work_queue::shared().clone(),
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                test_helpers::workspace_query(store.clone()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway),
            );

            use crate::adaptor::gateway::workflow::startup_repository::{
                HostWorkflowStartup, StoredWorkflowStartupRepository,
            };
            use crate::domain::workflow::repository::WorkflowStartupRepository;
            let repository = Arc::new(StoredWorkflowStartupRepository(store));
            let connection = rusqlite::Connection::open(
                crate::adaptor::gateway::local_event_store::layout::StoreLayout::new(
                    directory.path(),
                )
                .database_path(),
            )
            .unwrap();
            let queue = crate::usecase::work_queue::WorkQueueUsecase::new(Arc::new(
                crate::usecase::work_queue::ImmediateWorkQueueRuntime::default(),
            ));
            let runtime = Arc::new(HostWorkflowStartup {
                host: Arc::new(host),
                app,
            });
            for _ in 0..2 {
                let startup = crate::usecase::workflow::startup::WorkflowStartupUsecase::new(
                    queue.clone(),
                    repository.clone(),
                    runtime.clone(),
                );
                assert!(startup
                    .execute()
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("bypassPermissions"));
                let after = repository.load(TREE_ID).await.unwrap().unwrap();
                let count: i64 = connection
                    .query_row("SELECT COUNT(*) FROM node_events", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(count, 1);
                assert!(after.execution.is_active());
                let observations = queue.failure_query().records(TREE_ID).await;
                assert_eq!(observations.len(), 1);
                assert_eq!(
                    observations[0].record.kind,
                    crate::domain::failure::FailureKind::StateRequired
                );
                assert!(observations[0].requires_attention);
            }
        }
    }
}

#[cfg(test)]
mod command_env_tests {
    use super::*;

    #[test]
    fn command_env_includes_worktree_path() {
        let input = CommandExecutionInput {
            execution_id: "execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            node_name: "check".to_string(),
            attempt: 1,
            worktree_path: "/repo/worktree".to_string(),
            raw_command: Some("true".to_string()),
            definition_env: Vec::new(),
            contract: None,
            schemas: BTreeMap::new(),
            session_id: None,
        };

        let env = command_env(
            &input,
            vec![
                ("DOC".to_string(), "document".to_string()),
                (
                    "RELEASH_WORKTREE_PATH".to_string(),
                    "/definition/attempted-override".to_string(),
                ),
            ],
        );

        assert!(env.contains(&("DOC".to_string(), "document".to_string())));
        assert!(env.contains(&(
            "RELEASH_WORKTREE_PATH".to_string(),
            "/repo/worktree".to_string()
        )));
        assert_eq!(
            env.iter()
                .rev()
                .find(|(name, _)| name == "RELEASH_WORKTREE_PATH")
                .map(|(_, value)| value.as_str()),
            Some("/repo/worktree")
        );
    }

    #[tokio::test]
    async fn test_command_env_yaml定義と束縛から子processへstringとjsonを渡す() {
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: env-runtime
description: env runtime
nodes:
  main:
    command: 'printf "%s\n" "$DOC" "$META" "$COUNT"'
    input:
      - document
      - metadata
    env:
      DOC: document
      META: metadata
      COUNT: metadata.count
"#,
        )
        .unwrap();
        let command = workflow.entry_node().unwrap().command_spec().unwrap();
        let bindings = vec![
            (
                "document".to_string(),
                serde_json::Value::String("plain document".to_string()),
            ),
            (
                "metadata".to_string(),
                serde_json::json!({"count": 2, "ready": true}),
            ),
        ];
        let definition_env =
            workflow_reference::resolve_command_environment(&command.env, &bindings).unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let input = CommandExecutionInput {
            execution_id: "execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            node_name: "main".to_string(),
            attempt: 1,
            worktree_path: cwd.path().to_string_lossy().into_owned(),
            raw_command: Some(command.command.clone()),
            definition_env: Vec::new(),
            contract: None,
            schemas: BTreeMap::new(),
            session_id: None,
        };

        let output = workflow_command_runner::spawn_shell_command(
            cwd.path(),
            &command.command,
            command_env(&input, definition_env),
            "workflow command",
            workflow_command_runner::OutputLimit {
                max_bytes: workflow_output_limit::MAX_OUTPUT_SIZE,
                truncation_marker: workflow_output_limit::TRUNCATION_MARKER,
            },
        )
        .unwrap()
        .wait()
        .await
        .unwrap();

        assert_eq!(output.exit_code, 0);
        assert_eq!(
            output.stdout,
            "plain document\n{\"count\":2,\"ready\":true}\n2\n"
        );
    }
}

#[cfg(test)]
#[path = "workflow_host/isolated_worktree_test.rs"]
mod isolated_worktree_tests;

#[cfg(test)]
#[path = "workflow_host/test_helpers.rs"]
pub(crate) mod test_helpers;

#[cfg(test)]
#[path = "workflow_host/secret_redaction_test.rs"]
mod secret_redaction_tests;

#[cfg(test)]
#[path = "workflow_host/shutdown_test.rs"]
mod shutdown_tests;

#[cfg(test)]
#[path = "workflow_host_test.rs"]
mod workflow_host_persistence_tests;
