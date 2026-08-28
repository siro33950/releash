//! Workflow execution host gateway.
//!
//! The domain aggregate owns lifecycle transitions and decisions, while
//! `usecase::workflow::runtime_driver` owns their application procedure and
//! transaction ordering. This gateway retains the aggregates, delegates
//! decisions to them, and connects event storage, agent sessions, processes,
//! and notifications.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;

mod activation;
pub(crate) mod approval_runtime;
mod command_preparation;
pub(crate) mod execution_registry;
pub(crate) mod execution_state;
mod lifecycle_commands;
pub(crate) mod node_settings;
pub(crate) mod output_limit;
pub(crate) mod prompt_rendering;
pub(crate) mod runtime_commit;
pub(crate) mod runtime_session;

use activation::{run_runtime_activation, RuntimeActivationGate};
use command_preparation::{command_execution_input_is_current, CommandExecutionInput};

use crate::adaptor::gateway::workflow::event_log_writer as workflow_event_log_writer;
use crate::adaptor::gateway::workflow::execution_store::{
    ExecutionOrigin, ExecutionStatus, ExecutionStore, ExecutionStoreError,
    WorkflowExecutionMetadata,
};
use crate::adaptor::gateway::workflow::fact_log as workflow_fact_log;
use crate::adaptor::gateway::workflow::node_session_boundary::{
    ProviderWorkflowAgentSessionPort, WorkflowAgentSessionPort, WorkflowSessionLaunchConfig,
};
use crate::adaptor::gateway::workflow::secret_source;
use crate::domain::workflow::entities::workflow_execution::{
    AppliedAdvance, LeafStart, RuntimeNodeExecutionStatus as NodeExecutionStatus,
    RuntimeNodeResumePreviousState, TransitionOutcome,
};
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::reference as workflow_reference;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::WorkflowFacetContents;
use crate::domain::workflow::{
    ContractValidationResult, FailureClassification, FailureDisposition, NodeExecutionFailureKind,
    SchemaDef as DomainSchemaDef,
};
use crate::domain::workflow::{ExecutionTreeLaunch, NodeKindName, WorkflowDefinition};
use crate::infrastructure::process::command_runner::{
    self as workflow_command_runner, ActiveCommandHandle, CommandRunOutput, CommandRunnerError,
};
use crate::usecase::agent_session::{
    AgentSessionInitialInstructionUsecase, AgentSessionInterruptUsecase, AgentSessionLaunchUsecase,
    AgentSessionLifecycleUsecase,
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
use execution_registry::find_any_by_worktree;
use execution_state::DomainWorkflowExecution;
use node_settings::WorkflowDefaults;
use output_limit as workflow_output_limit;
use prompt_rendering as workflow_prompt;
use runtime_commit::{
    self as workflow_runtime_commit, AbortOutcome, AbortTargetLookup, RequiredEventCommit,
};
use runtime_session as workflow_runtime_session;
use tauri::Manager as _;

fn current_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

/// Workflow 集約を保持し、usecase の駆動手順を外界へ接続する gateway host。
#[derive(Clone)]
pub struct WorkflowRuntimeHost {
    /// `execution_id` → `DomainWorkflowExecution` の in-memory マッピング。
    /// HashMap キーは `DomainWorkflowExecution.id`（= `execution_id`）と一致する。
    /// `worktree_path` は `DomainWorkflowExecution.worktree_path` 属性として保持し、
    /// `worktree_path → execution_id` の補助解決は Execution Store の secondary index 経由で行う。
    executions: Arc<Mutex<HashMap<String, DomainWorkflowExecution>>>,
    /// create commit 前の Session 実行木を startup reconciliation から保護する予約。
    execution_tree_reservations: Arc<Mutex<HashSet<String>>>,
    /// execution_id → 解決済み facet 本文。workflow state / event には含めない runtime-local read model。
    execution_facet_contents: Arc<Mutex<HashMap<String, WorkflowFacetContents>>>,
    /// execution_id → runtime activation serialization lock.
    ///
    /// Weak references keep session/fanout startup and stop/abort mutually exclusive without
    /// retaining one lock for every historical execution.
    runtime_activation_locks: Arc<Mutex<HashMap<String, Weak<RuntimeActivationGate>>>>,
    /// node_execution_id → active command process shutdown handle.
    active_commands: Arc<Mutex<HashMap<String, ActiveCommandHandle>>>,
    /// node_execution_id → owning workflow execution_id.
    active_command_executions: Arc<Mutex<HashMap<String, String>>>,
    /// node_execution_id → command completion observer task owned by this workflow runtime.
    command_completion_observers: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// node_execution_id → shutdown reason consumed by the completion observer.
    command_shutdown_intents: Arc<Mutex<HashMap<String, ActiveCommandShutdownIntent>>>,
    /// Startup handoff replay may advance canonical workflow state but must
    /// never activate the next provider/command node in the recovery pass.
    recovery_effect_suppression: Arc<Mutex<HashSet<String>>>,
    startup_recovery_lock: Arc<Mutex<()>>,
    /// active な WorkflowExecutionMetadata の管理および execution metadata の永続化を担う Execution Store。
    /// worktree_path → active execution_id の secondary index は Execution Store 内で保持する。
    execution_store: Arc<ExecutionStore>,
    workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
    worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
    worktree_ledger: Arc<dyn crate::domain::workflow::IsolatedWorktreeLedgerRepository>,
    worktree_inventory: Arc<dyn crate::domain::workflow::WorktreeInventoryGateway>,
}

enum RequiredEventCommitFailure {
    /// No event fact became visible; rollbackable resources may be discarded.
    BeforeDurableAppend(WorkflowRuntimeError),
}

impl RequiredEventCommitFailure {
    fn into_workflow_error(self) -> WorkflowRuntimeError {
        let Self::BeforeDurableAppend(error) = self;
        error
    }
}

struct ControlPlaneCommitCandidate<'a> {
    execution_id: &'a str,
    snapshot_before: DomainWorkflowExecution,
    candidate: DomainWorkflowExecution,
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

fn commit_snapshot_is_current(
    exec: &DomainWorkflowExecution,
    snapshot: &RuntimeCommitSnapshot,
) -> bool {
    exec.id == snapshot.execution_id
        && exec.updated_at == snapshot.updated_at
        && exec.state() == &snapshot.state
        && exec.current_session_id == snapshot.current_session_id
        && exec.node_executions == snapshot.node_executions
}

// [08] `lookup_node_contract` は domain の contract service に移動済み。
// driver と CLI の双方が同じ domain service を参照するため、本モジュールではメモのみ残す。

impl WorkflowRuntimeHost {
    pub(crate) fn ensure_node_recovery_available(
        &self,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let snapshot = self
            .worktree_ledger
            .snapshot_for_tree(execution_id)
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        if let Some(cause) = snapshot.recovery_cause_for_node(execution_id, node_execution_id) {
            return Err(WorkflowRuntimeError::InvalidState(cause.to_string()));
        }
        Ok(())
    }

    pub(crate) async fn load_control_plane_execution(
        &self,
        execution_id: &str,
    ) -> Option<DomainWorkflowExecution> {
        self.executions.lock().await.get(execution_id).cloned()
    }

    pub(crate) async fn reserve_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let mut reservations = self.execution_tree_reservations.lock().await;
        if self.executions.lock().await.contains_key(tree_id) {
            return Ok(());
        }
        reservations.insert(tree_id.to_string());
        Ok(())
    }

    pub(crate) async fn release_started_execution_tree_reservation(
        &self,
        tree_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.execution_tree_reservations
            .lock()
            .await
            .remove(tree_id);
        Ok(())
    }

    async fn execution_tree_is_registered_or_reserved(&self, tree_id: &str) -> bool {
        let reservations = self.execution_tree_reservations.lock().await;
        self.executions.lock().await.contains_key(tree_id) || reservations.contains(tree_id)
    }

    pub(crate) async fn register_started_execution_tree<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        tree_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        if self.executions.lock().await.contains_key(tree_id) {
            return Ok(());
        }
        let store = app
            .try_state::<std::sync::Arc<
                crate::adaptor::gateway::local_event_store::LocalEventStore,
            >>()
            .map(|store| store.inner().clone())
            .ok_or_else(|| {
                WorkflowRuntimeError::SessionStore(
                    "workflow SQLite event authority is not managed".to_string(),
                )
            })?;
        let backend = workflow_fact_log::FactLogReadBackend::Live(store);
        let folded = workflow_fact_log::fold_tree_from(&backend, tree_id)
            .map_err(WorkflowRuntimeError::SessionStore)?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(tree_id.to_string()))?;
        if !folded.aggregate.is_active() {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution tree '{tree_id}' is not active"
            )));
        }
        if folded.aggregate.launched_as == ExecutionTreeLaunch::Workflow {
            let model = crate::domain::workflow::services::fact_replay::derive_read_model(&folded);
            let metadata = WorkflowExecutionMetadata {
                execution_id: model.id.clone(),
                workflow_name: model.workflow_name.clone(),
                status: model.status,
                worktree_path: model.worktree_path.clone(),
                current_node: model.current_node.clone(),
                created_from: model.created_from,
                started_at: model.started_at,
                updated_at: model.updated_at,
                completed_at: model.completed_at,
                error_reason: model.error_reason.clone(),
                interruption_reason: model.interruption_reason,
                resume_from_node: model.resume_from_node.clone(),
                total_token_usage: model.total_token_usage.clone(),
            };
            self.execution_store
                .register_active_execution(metadata)
                .await
                .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        }
        let mut executions = self.executions.lock().await;
        if executions.contains_key(tree_id) {
            return Ok(());
        }
        executions.insert(tree_id.to_string(), folded.aggregate);
        Ok(())
    }

    pub(crate) async fn commit_workflow_control_plane<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        commit: crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
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

    pub(crate) async fn finish_workflow_control_plane_commit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowRuntimeError> {
        self.finish_control_plane_commit(app, worktree_path, snapshot, outcome)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_canonical(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        data_dir: Option<std::path::PathBuf>,
        workspace_query: Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
        agent_session_launch: Arc<AgentSessionLaunchUsecase>,
        agent_session_initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
        agent_session_interrupt: Arc<AgentSessionInterruptUsecase>,
        agent_session_lifecycle: Arc<AgentSessionLifecycleUsecase>,
        provider_availability: Arc<dyn crate::domain::agent_session::ProviderAvailabilityReader>,
        worktree_ledger: Arc<dyn crate::domain::workflow::IsolatedWorktreeLedgerRepository>,
        worktree_inventory: Arc<dyn crate::domain::workflow::WorktreeInventoryGateway>,
    ) -> Self {
        Self::with_execution_store(
            workflow_resolver,
            worktree_resolver,
            Arc::new(ExecutionStore::new_canonical(data_dir, workspace_query)),
            Arc::new(ProviderWorkflowAgentSessionPort::new(
                agent_session_launch,
                agent_session_initial_instruction,
                agent_session_interrupt,
                agent_session_lifecycle,
                provider_availability,
            )),
            worktree_ledger,
            worktree_inventory,
        )
    }

    fn with_execution_store(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        execution_store: Arc<ExecutionStore>,
        workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
        worktree_ledger: Arc<dyn crate::domain::workflow::IsolatedWorktreeLedgerRepository>,
        worktree_inventory: Arc<dyn crate::domain::workflow::WorktreeInventoryGateway>,
    ) -> Self {
        Self {
            executions: Arc::new(Mutex::new(HashMap::new())),
            execution_tree_reservations: Arc::new(Mutex::new(HashSet::new())),
            execution_facet_contents: Arc::new(Mutex::new(HashMap::new())),
            runtime_activation_locks: Arc::new(Mutex::new(HashMap::new())),
            active_commands: Arc::new(Mutex::new(HashMap::new())),
            active_command_executions: Arc::new(Mutex::new(HashMap::new())),
            command_completion_observers: Arc::new(Mutex::new(HashMap::new())),
            command_shutdown_intents: Arc::new(Mutex::new(HashMap::new())),
            recovery_effect_suppression: Arc::new(Mutex::new(HashSet::new())),
            startup_recovery_lock: Arc::new(Mutex::new(())),
            execution_store,
            workflow_resolver,
            worktree_resolver,
            workflow_agent_sessions,
            worktree_ledger,
            worktree_inventory,
        }
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

    async fn reserve_workflow_execution(
        &self,
        workflow: &WorkflowDefinition,
        worktree_path: &str,
        _request: Option<String>,
        created_from: ExecutionOrigin,
        now: f64,
    ) -> Result<String, WorkflowRuntimeError> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        // worktree 排他は in-memory 登録（+ 起動時の fold 再構築）で判定する。
        // 永続層に worktree-owner CAS は存在しない（純粋事実ログの規約）。
        self.execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: workflow.name.clone(),
                status: ExecutionStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node: workflow.initial_execution_node().map(|n| n.name.clone()),
                created_from,
                started_at: now,
                updated_at: now,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .map_err(|e| match e {
                ExecutionStoreError::WorktreeAlreadyActive { .. } => {
                    WorkflowRuntimeError::AlreadyActive(workflow.name.clone())
                }
                other => WorkflowRuntimeError::SessionStore(format!(
                    "ExecutionStore register failed: {other}"
                )),
            })?;
        Ok(execution_id)
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
        let mut execution = crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: execution_id.clone(),
            workflow: workflow.clone(),
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(RuntimeExecutionState::Running),
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
            worktree_path: worktree_path.clone(),
            launched_as: crate::domain::workflow::ExecutionTreeLaunch::Workflow,
        };

        let mut execs = self.executions.lock().await;
        DomainWorkflowExecution::validate_start(
            &workflow,
            find_any_by_worktree(&execs, &worktree_path),
        )?;
        // 実行木の起動カスケード: root（合成子なら実効 entry の leaf まで）を開始する。
        let mut new_id = new_node_execution_id;
        let applied = execution
            .start_root(&mut new_id, now)
            .map_err(|error| WorkflowRuntimeError::InvalidState(error.to_string()))?;
        execs.insert(execution_id.clone(), execution);
        let snapshot = RuntimeCommitSnapshot::from_execution(execs.get(&execution_id).unwrap())?;
        Ok((snapshot, applied))
    }

    pub(crate) async fn application_shutdown_target_execution_ids(
        &self,
    ) -> Result<Vec<String>, String> {
        let mut ids = self
            .execution_store
            .list_active()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|summary| summary.execution_id)
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    /// 冪等 reconciliation の1周: 事実ログの fold で導出された状態を見て、
    /// まだ実行していない行動を実行し、実行した事実を追記する。
    ///
    /// - 前プロセスと共に消えた実行中プロセスは、喪失の観測（process_exited）
    ///   として追記される（Paused は fold の導出）。provider CLI はアプリ内
    ///   Terminal Surface で動くため、再起動を跨いで生き残る実プロセスは無い。
    /// - 前進の実行と事実の追記の間で落ちた場合の未実行の前進
    ///   （次の子の起動・fanout 展開の続き）は、導出された差分として検出・実行
    ///   される。既に事実が揃っている行動は差分に現れないため二重実行されない。
    ///
    /// 起動時復旧はこの1周目と同一であり、復旧専用経路は存在しない。
    pub async fn reconcile_startup<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<(), WorkflowRuntimeError> {
        let _reconcile_guard = self.startup_recovery_lock.lock().await;
        let Some(store) = app.try_state::<std::sync::Arc<
            crate::adaptor::gateway::local_event_store::LocalEventStore,
        >>() else {
            // canonical store の無い（テスト）構成では対象の木が無い。
            return Ok(());
        };
        let store = store.inner().clone();
        // inventory を読めないことは「全 worktree が消えた」ではない。隔離環境の
        // 突合だけを止め、プロセス喪失の観測と未実行の前進は続ける。
        let worktree_inventory = match self.worktree_inventory.snapshot() {
            Ok(inventory) => Some(inventory),
            Err(error) => {
                log::warn!("isolated worktree inventory is unavailable: {error}");
                None
            }
        };
        let backend = workflow_fact_log::FactLogReadBackend::Live(store.clone());
        let tree_ids = workflow_fact_log::list_tree_ids(&backend, None)
            .map_err(WorkflowRuntimeError::SessionStore)?;
        let mut first_recovery_error = None;
        for tree_id in tree_ids {
            if self
                .execution_tree_is_registered_or_reserved(&tree_id)
                .await
            {
                continue;
            }
            let now = current_timestamp();
            let mut new_id = new_node_execution_id;
            let reconciliation = match workflow_fact_log::reconcile_tree_pass(
                &store,
                &tree_id,
                now,
                &mut new_id,
                worktree_inventory.as_ref().map(|inventory| {
                    workflow_fact_log::WorktreeReconciliationPorts {
                        ledger: self.worktree_ledger.as_ref(),
                        inventory,
                    }
                }),
            ) {
                Ok(Some(reconciliation)) => reconciliation,
                Ok(None) => continue,
                Err(error) => {
                    let error = WorkflowRuntimeError::SessionStore(format!(
                        "workflow {tree_id}: reconciliation pass failed: {error}"
                    ));
                    log::warn!("{error}");
                    first_recovery_error.get_or_insert(error);
                    continue;
                }
            };
            let folded = reconciliation.folded;
            let pending_leaves = reconciliation.leaves;
            if !folded.aggregate.is_active() {
                continue;
            }
            // 導出状態を engine の作業状態（in-memory・非永続）として登録する。
            let worktree_path = folded.aggregate.worktree_path.clone();
            if folded.aggregate.launched_as == ExecutionTreeLaunch::Workflow {
                let model =
                    crate::domain::workflow::services::fact_replay::derive_read_model(&folded);
                let metadata = WorkflowExecutionMetadata {
                    execution_id: model.id.clone(),
                    workflow_name: model.workflow_name.clone(),
                    status: model.status,
                    worktree_path: model.worktree_path.clone(),
                    current_node: model.current_node.clone(),
                    created_from: model.created_from,
                    started_at: model.started_at,
                    updated_at: model.updated_at,
                    completed_at: model.completed_at,
                    error_reason: model.error_reason.clone(),
                    interruption_reason: model.interruption_reason,
                    resume_from_node: model.resume_from_node.clone(),
                    total_token_usage: model.total_token_usage.clone(),
                };
                let metadata = match self
                    .execution_store
                    .reconcile_orphan_from_projection(metadata, &model)
                    .await
                {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        let error = WorkflowRuntimeError::SessionStore(format!(
                            "workflow {tree_id}: reconciliation metadata refresh failed: {error}"
                        ));
                        log::warn!("{error}");
                        first_recovery_error.get_or_insert(error);
                        continue;
                    }
                };
                if let Err(error) = self
                    .execution_store
                    .register_active_execution(metadata)
                    .await
                {
                    let error = WorkflowRuntimeError::SessionStore(format!(
                        "workflow {tree_id}: reconciliation active registry restore failed: {error}"
                    ));
                    log::warn!("{error}");
                    first_recovery_error.get_or_insert(error);
                    continue;
                }
            }
            {
                let mut executions = self.executions.lock().await;
                executions.insert(tree_id.clone(), folded.aggregate.clone());
            }
            // 4) 未起動または前進で生まれた leaf を起動する。失敗した tree は
            //    registry から戻し、次の reconciliation 呼び出しで再試行できるようにする。
            if !pending_leaves.is_empty() {
                if let Err(error) = self
                    .start_leaves(app, &tree_id, &worktree_path, pending_leaves)
                    .await
                {
                    self.executions.lock().await.remove(&tree_id);
                    first_recovery_error.get_or_insert(error);
                    continue;
                }
            }
        }
        match first_recovery_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
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
    async fn start_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
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

        // ===== Phase 2: 副作用（Execution Store reservation 先取り → 親 session 作成 → executions 登録） =====
        // Spec issues-1011 finding 5/8: 並行起動でも parent ChatSession を孤立させないために
        // Execution Store reservation を「最初の副作用」にする。reservation が失敗（同一 worktree
        // への並行起動）した場合は AlreadyActive として返り、他の副作用は走らない。
        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(|e| WorkflowRuntimeError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let now = current_timestamp();
        let execution_id = self
            .reserve_workflow_execution(
                &workflow,
                &worktree_path,
                request.clone(),
                created_from,
                now,
            )
            .await?;
        self.execution_facet_contents
            .lock()
            .await
            .insert(execution_id.clone(), facet_contents);

        // 以降の副作用で失敗した場合は Execution Store reservation を確実に撤回する helper。
        // Spec issues-1011 finding 9: reservation 撤回専用 API (`cancel_reservation`) を使い、
        // 失敗した起動を completed 一覧（terminal entry）に残さない。撤回自体の失敗は
        // warn を出した上で reservation を completed_at=now の Failed として最低限 metadata に
        // 残し、Execution Store と driver の状態スキューを抑える。
        // 撤回 helper は最終的な Result を返し、呼出側で start_workflow の Err に伝播させる。
        let rollback_execution_id = execution_id.clone();
        let rollback_reservation = |reason: String| async move {
            if let Err(rs_err) = self
                .execution_store
                .cancel_reservation(&rollback_execution_id)
                .await
            {
                log::warn!(
                    "ExecutionStore cancel_reservation failed during start rollback for {rollback_execution_id}: {rs_err}; reason={reason}"
                );
            }
        };

        let _ = data_dir; // unused after parent session removal
        let workflow_defaults = WorkflowDefaults;

        // validate_start → insert → スナップショット確定を同一ロックで原子的に実行。
        // reservation 段階で worktree 衝突は撥ねているが、executions 側にも terminal execution が
        // 残っている可能性があるため `find_any_by_worktree` で active な existing を見て
        // validate_start する。
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
                rollback_reservation(format!("validate_start failed: {e}")).await;
                return Err(e);
            }
        };

        // [04] commit point: ExecutionStarted と起動カスケードの NodeStarted 群を
        // 同一の required batch で append する。
        let mut required_start_events = vec![WorkflowEvent::ExecutionStarted {
            execution_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            worktree_path: worktree_path.clone(),
            created_from,
            request: request.clone().unwrap_or_default(),
            definition: workflow.clone(),
            timestamp: now,
        }];
        required_start_events.extend(applied.events);
        if let Err(e) = self.write_log_required_batch(app, &required_start_events) {
            let mut execs = self.executions.lock().await;
            execs.remove(&execution_id);
            drop(execs);
            self.release_execution_facet_contents(&execution_id).await;
            rollback_reservation(format!("initial workflow event batch failed: {e}")).await;
            return Err(WorkflowRuntimeError::SessionStore(format!(
                "write initial workflow event batch failed: {e}"
            )));
        }

        // [04] post-commit: broadcast。ExecutionStarted は append 済みのため command は既に受理。
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;

        // [04] post-commit: ExecutionStarted append 済みのため start primitive は既に受理。
        //    初回 runtime 起動失敗は Failed 状態遷移として観測し、
        //    start primitive は Ok(execution_id) を返す（spec [04]『command 受理境界』Rule）。
        if let crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision::StartLeaves(leaves) =
            applied.decision
        {
            if let Err(e) = self
                .start_leaves(app, &execution_id, &worktree_path, leaves)
                .await
            {
                if let Err(settle_error) = self
                    .settle_runtime_failure(app, &worktree_path, &execution_id, &e)
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

    pub(crate) async fn start_resolved_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
    ) -> Result<String, WorkflowRuntimeError> {
        self.start_workflow(app, workflow, worktree_path, request, created_from)
            .await
    }

    async fn restart_paused_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.restart_workflow_command_node(app, execution_id, node_execution_id)
            .await
    }

    async fn restart_workflow_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let timestamp = current_timestamp();
        let new_node_execution_id = new_node_execution_id();
        let (snapshot_before, mut candidate, worktree_path) = {
            let executions = self.executions.lock().await;
            let current = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            (
                current.clone(),
                current.clone(),
                current.worktree_path.clone(),
            )
        };
        let restarted = candidate
            .restart_node_attempt_at(
                node_execution_id,
                new_node_execution_id,
                timestamp,
                crate::domain::workflow::entities::workflow_execution::NodeRestartMode::CommandResume,
            )
            .ok_or_else(|| {
                WorkflowRuntimeError::InvalidState(format!(
                    "node execution '{node_execution_id}' is not a paused Command retry target"
                ))
            })?;
        let new_attempt = restarted.attempt;
        let events = vec![
            WorkflowEvent::NodeRetryRequested {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                timestamp,
            },
            WorkflowEvent::NodeStarted {
                execution_id: execution_id.to_string(),
                node_execution_id: new_attempt.id.clone(),
                node_name: new_attempt.node_name.clone(),
                kind: new_attempt.kind,
                attempt: new_attempt.attempt,
                parent: new_attempt.parent.clone(),
                timestamp,
            },
        ];
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before,
                    candidate,
                    transition_outcome: TransitionOutcome::Applied,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
        self.finish_control_plane_commit(
            app,
            &worktree_path,
            &snapshot,
            Some(NodeOutcome::StartLeaves(
                snapshot.clone(),
                vec![restarted.leaf],
            )),
        )
        .await?;
        Ok(())
    }

    async fn commit_control_plane_candidate<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
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
        let mut executions = self.executions.lock().await;
        let current = executions
            .get_mut(execution_id)
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        let launched_as = current.launched_as;
        let persisted = transaction
            .persist_async(current, |events| async move {
                if provider_events.is_empty() {
                    workflow_event_log_writer::append_required_events_for_app(app, &events)
                } else {
                    match workflow_event_log_writer::append_provider_stop_for_app(
                        app,
                        &events,
                        provider_events,
                    )
                    .await?
                    {
                        workflow_event_log_writer::ProviderStopCommitOutcome::Committed => {}
                        workflow_event_log_writer::ProviderStopCommitOutcome::CanonicalFactsCommittedWithProviderLifecycleFailure(error) => {
                            log::warn!(
                                "workflow provider Stop facts were committed but provider lifecycle commit failed: {error}"
                            );
                        }
                    }
                    Ok(())
                }
            })
            .await;
        let durable = match persisted {
            Ok(durable) => durable,
            Err(WorkflowTransactionCommitError::StaleCandidate) => {
                return Err(WorkflowRuntimeError::Conflict(format!(
                    "execution '{execution_id}' changed before control-plane commit"
                )));
            }
            Err(WorkflowTransactionCommitError::Persistence(error)) => {
                let backend =
                    workflow_fact_log::FactLogReadBackend::Live(
                        app.try_state::<std::sync::Arc<
                            crate::adaptor::gateway::local_event_store::LocalEventStore,
                        >>()
                        .map(|store| store.inner().clone())
                        .ok_or_else(|| {
                            WorkflowRuntimeError::SessionStore(
                                "workflow SQLite event authority is not managed".to_string(),
                            )
                        })?,
                    );
                let refreshed_snapshot = match workflow_fact_log::fold_tree_from(
                    &backend,
                    execution_id,
                ) {
                    Ok(Some(folded)) => {
                        *current = folded.aggregate;
                        RuntimeCommitSnapshot::from_execution(current).ok()
                    }
                    Ok(None) => None,
                    Err(refresh_error) => {
                        log::error!(
                            "workflow {execution_id}: failed to reconcile facts after partial persistence: {refresh_error}"
                        );
                        None
                    }
                };
                drop(executions);
                if launched_as == ExecutionTreeLaunch::Workflow {
                    let Some(refreshed_snapshot) = refreshed_snapshot else {
                        return Err(WorkflowRuntimeError::SessionStore(error));
                    };
                    if let Err(refresh_error) = self
                        .sync_state_after_required_event_commit(launched_as, &refreshed_snapshot)
                        .await
                    {
                        log::error!(
                            "workflow {execution_id}: failed to refresh execution registry after partial persistence: {refresh_error}"
                        );
                    }
                }
                return Err(WorkflowRuntimeError::SessionStore(error));
            }
        };
        let effects = durable.into_effects();
        drop(executions);
        self.spawn_committed_runtime_effects(effects);
        if launched_as == ExecutionTreeLaunch::Workflow {
            if let Err(error) = self
                .sync_state_after_required_event_commit(launched_as, &snapshot)
                .await
            {
                log::warn!(
                    "workflow {execution_id}: derived execution projection refresh failed after control-plane commit: {error}"
                );
            }
        }
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

    async fn finish_control_plane_commit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
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
            workflow_runtime_session::broadcast_state(app, worktree_path, snapshot.clone()).await;
            Ok(())
        }
    }

    pub(crate) async fn release_deleted_execution_tree(
        &self,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.executions.lock().await.remove(execution_id);
        self.release_execution_facet_contents(execution_id).await;
        Ok(())
    }

    async fn recovery_effects_suppressed(&self, execution_id: &str) -> bool {
        self.recovery_effect_suppression
            .lock()
            .await
            .contains(execution_id)
    }

    /// `execution_id` から `RuntimeCommitSnapshot` を取得する。
    #[cfg(test)]
    pub async fn get_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Option<RuntimeCommitSnapshot> {
        let execs = self.executions.lock().await;
        execs
            .get(execution_id)
            .and_then(|execution| RuntimeCommitSnapshot::from_execution(execution).ok())
    }

    #[cfg(debug_assertions)]
    pub(crate) async fn acceptance_state_by_execution_id(
        &self,
        execution_id: &str,
    ) -> Option<crate::domain::workflow::WorkflowRuntimeSnapshot> {
        let execs = self.executions.lock().await;
        execs.get(execution_id)
            .and_then(|execution| RuntimeCommitSnapshot::from_execution(execution).ok())
            .map(
                crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot,
            )
    }

    async fn release_execution_facet_contents(&self, execution_id: &str) {
        self.execution_facet_contents
            .lock()
            .await
            .remove(execution_id);
    }

    async fn release_terminal_execution(&self, execution_id: &str) {
        let removed = {
            let mut execs = self.executions.lock().await;
            if execs
                .get(execution_id)
                .is_some_and(|exec| exec.is_terminal())
            {
                execs.remove(execution_id);
                true
            } else {
                false
            }
        };
        if removed {
            self.release_execution_facet_contents(execution_id).await;
        }
    }

    // ---- 内部メソッド ----

    /// advance が返した leaf 群を起動する。Session はまとめて prepare →
    /// SessionAttached を一括 commit → activate、Command は spawn する。
    async fn start_leaves<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        worktree_path: &str,
        leaves: Vec<LeafStart>,
    ) -> Result<(), WorkflowRuntimeError> {
        if leaves.is_empty() {
            return Ok(());
        }
        let (workflow, attempts_by_id) = {
            let executions = self.executions.lock().await;
            let exec = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let attempts_by_id: HashMap<String, u32> = exec
                .node_executions
                .iter()
                .map(|execution| (execution.id.clone(), execution.attempt))
                .collect();
            (exec.workflow.clone(), attempts_by_id)
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
            match leaf.kind {
                NodeKindName::Command => {
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
                            worktree_path: worktree_path.to_string(),
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
                NodeKindName::Session => {
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
                        launch_config,
                        initial_instruction,
                    ));
                }
                NodeKindName::Fanout | NodeKindName::Sequence => {
                    return Err(WorkflowRuntimeError::InvalidState(format!(
                        "composite node '{}' has no leaf runtime to start",
                        leaf.node_name
                    )));
                }
            }
        }

        // Session を先に全 prepare し、SessionAttached を一括 commit してから activate する。
        let mut session_setups: Vec<(String, String)> = Vec::with_capacity(session_plans.len());
        for (node_execution_id, launch_config, initial_instruction) in session_plans {
            let prepared = self
                .workflow_agent_sessions
                .prepare_workflow_agent_session(
                    worktree_path,
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
                    return match self.rollback_prepared_sessions(&session_setups).await {
                        Some(rollback_error) => Err(WorkflowRuntimeError::AgentSession(format!(
                            "{launch_error}; rollback failed: {rollback_error}"
                        ))),
                        None => Err(launch_error),
                    };
                }
            }
        }
        if !session_setups.is_empty() {
            let timestamp = current_timestamp();
            let commit_result: Result<RuntimeCommitSnapshot, WorkflowRuntimeError> = async {
                let (snapshot_before, snapshot, events) = {
                    let mut executions = self.executions.lock().await;
                    let execution = executions.get_mut(&execution_id).ok_or_else(|| {
                        WorkflowRuntimeError::ExecutionNotFound(execution_id.clone())
                    })?;
                    let snapshot_before = execution.clone();
                    let mut events = Vec::new();
                    for (node_execution_id, session_id) in &session_setups {
                        if execution.attach_node_session(
                            node_execution_id,
                            session_id.clone(),
                            timestamp,
                        ) != TransitionOutcome::Applied
                        {
                            *execution = snapshot_before;
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
                    (
                        snapshot_before,
                        RuntimeCommitSnapshot::from_execution(execution)?,
                        events,
                    )
                };
                let execution_store_snapshot_before = self
                    .execution_store
                    .active_execution_snapshot(&execution_id)
                    .await;
                self.commit_required_events(
                    app,
                    RequiredEventCommit {
                        execution_id: &execution_id,
                        snapshot_for_commit: &snapshot,
                        snapshot_before,
                        execution_store_snapshot_before,
                        required_events: events,
                        append_error_context: "session attachment event append failed",
                    },
                )
                .await?;
                Ok(snapshot)
            }
            .await;
            let snapshot = match commit_result {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return match self.rollback_prepared_sessions(&session_setups).await {
                        Some(rollback_error) => Err(WorkflowRuntimeError::AgentSession(format!(
                            "{error}; rollback failed: {rollback_error}"
                        ))),
                        None => Err(error),
                    };
                }
            };
            workflow_runtime_session::broadcast_state(app, worktree_path, snapshot).await;
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
                    if let Err(settlement_error) = self
                        .settle_runtime_failure_for_node(
                            app,
                            worktree_path,
                            &execution_id,
                            node_execution_id,
                            &error,
                        )
                        .await
                    {
                        return match self.rollback_prepared_sessions(&session_setups).await {
                            Some(rollback_error) => Err(WorkflowRuntimeError::AgentSession(
                                format!("{settlement_error}; rollback failed: {rollback_error}"),
                            )),
                            None => Err(settlement_error),
                        };
                    }
                    log::warn!(
                        "workflow {execution_id}: NodeExecution '{node_execution_id}' failed to activate: {error}"
                    );
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
                self.settle_runtime_failure_for_node(
                    app,
                    worktree_path,
                    &execution_id,
                    &node_execution_id,
                    &error,
                )
                .await?;
                log::warn!(
                    "workflow {execution_id}: Command NodeExecution '{node_execution_id}' failed to activate: {error}"
                );
            }
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

    async fn spawn_command_execution<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        mut input: CommandExecutionInput,
    ) -> Result<(), WorkflowRuntimeError> {
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
            let executions = self.executions.lock().await;
            let Some(execution) = executions.get(&input.execution_id) else {
                return Ok(());
            };
            if !command_execution_input_is_current(execution, &input) {
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
                self.active_commands
                    .lock()
                    .await
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
                self.active_commands
                    .lock()
                    .await
                    .remove(&input.node_execution_id);
                self.active_command_executions
                    .lock()
                    .await
                    .remove(&input.node_execution_id);
                return Ok(());
            }
            Err(error) => {
                running.handle().request_shutdown();
                self.active_commands
                    .lock()
                    .await
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
        let still_current = self.command_execution_still_current(&input).await;
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
        if !still_current {
            self.shutdown_active_command_execution(&node_execution_id)
                .await;
        }
        Ok(())
    }

    async fn observe_command_completion<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        input: CommandExecutionInput,
        running: workflow_command_runner::RunningCommand,
    ) {
        let output = running.wait().await;
        self.active_commands
            .lock()
            .await
            .remove(&input.node_execution_id);
        self.active_command_executions
            .lock()
            .await
            .remove(&input.node_execution_id);

        match output {
            Ok(output) => {
                let failure_input = input.clone();
                if let Err(error) = self.commit_command_output(app, input, output).await {
                    let reason = format!("command completion failed: {error}");
                    log::warn!("{reason}");
                    if self.command_execution_still_current(&failure_input).await {
                        if let Err(settle_error) = self
                            .settle_runtime_failure(
                                app,
                                &failure_input.worktree_path,
                                &failure_input.execution_id,
                                &error,
                            )
                            .await
                        {
                            log::error!(
                            "workflow {}: command completion failed and NodeFailed settlement also failed: {settle_error}",
                            failure_input.execution_id
                        );
                        }
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
                        "workflow {}: command cancelled for graceful shutdown without durable interruption",
                        input.execution_id
                    );
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

    async fn command_execution_still_current(&self, input: &CommandExecutionInput) -> bool {
        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(&input.execution_id) else {
            return false;
        };
        command_execution_input_is_current(exec, input)
    }

    async fn commit_command_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        input: CommandExecutionInput,
        output: CommandRunOutput,
    ) -> Result<(), WorkflowRuntimeError> {
        let secrets = secret_source::collect_configured_secret_values(app);
        let artifact =
            build_command_artifact(&input.schemas, input.contract.as_deref(), output, &secrets);
        let artifact_value = artifact.value.clone();
        let artifact_event_contract = artifact.event_contract.clone();
        let result_summary = artifact.result_summary.clone();
        let timestamp = current_timestamp();

        let (outcome, snapshot_before, snapshot_for_commit, worktree_path, required_events) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&input.execution_id) else {
                return Ok(());
            };
            if !command_execution_input_is_current(exec, &input) {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let requires_approval = exec
                .workflow
                .node_by_name(&input.node_name)
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
                contract: artifact_event_contract,
                value: artifact_value,
                request_id: None,
                submitted_at: None,
                timestamp,
            }];
            let outcome = if requires_approval {
                // completion: approval — exit code での既定完了後、human の承認まで完了しない。
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
                match workflow_runtime_driver::node_outcome_from_advance(exec, applied.decision) {
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
                RuntimeCommitSnapshot::from_execution(exec)?,
                exec.worktree_path.clone(),
                required_events,
            )
        };

        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(&input.execution_id)
            .await;
        self.commit_required_events(
            app,
            RequiredEventCommit {
                execution_id: &input.execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                execution_store_snapshot_before,
                required_events,
                append_error_context: "command completion event append failed",
            },
        )
        .await?;
        self.finalize_after_commit(app, &snapshot_for_commit, &worktree_path)
            .await;
        if let Some(outcome) = outcome {
            Box::pin(self.dispatch_node_outcome_side_effects(app, &worktree_path, outcome)).await?;
        }
        Ok(())
    }

    async fn fail_current_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        input: &CommandExecutionInput,
        reason: String,
    ) -> Result<(), WorkflowRuntimeError> {
        let is_current = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(&input.execution_id) else {
                return Ok(());
            };
            let command_is_active = exec.node_executions.iter().any(|node_execution| {
                node_execution.id == input.node_execution_id
                    && node_execution.node_name == input.node_name
                    && node_execution.attempt == input.attempt
                    && node_execution.status.is_active()
            });
            if !exec.is_active() || !command_is_active {
                return Ok(());
            }
            true
        };
        if is_current {
            self.settle_node_failure_for_node(
                app,
                &input.worktree_path,
                &input.execution_id,
                &input.node_execution_id,
                reason,
                NodeExecutionFailureKind::InfrastructureCrash,
            )
            .await?;
        }
        Ok(())
    }

    async fn shutdown_active_command_execution(&self, node_execution_id: &str) {
        if let Some(handle) = self.active_commands.lock().await.remove(node_execution_id) {
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
        let commands = {
            let active_commands = self.active_commands.lock().await;
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
            let mut active_commands = self.active_commands.lock().await;
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

    /// [04] post-commit projection phase: required event append 後に Execution Store の
    /// active projection / terminal metadata を snapshot に揃える。
    /// append-only event fact が command の最初の不可逆な可視 commit point であり、
    /// Execution Store metadata はその projection として同期する。
    async fn sync_state_after_required_event_commit(
        &self,
        launched_as: ExecutionTreeLaunch,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError> {
        if launched_as != ExecutionTreeLaunch::Workflow {
            return Ok(());
        }
        let execution_id = snapshot.execution_id.clone();
        workflow_runtime_commit::sync_execution_store_from_snapshot(
            &self.execution_store,
            &execution_id,
            snapshot,
        )
        .await
    }

    async fn commit_required_events<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        commit: RequiredEventCommit<'_>,
    ) -> Result<(), WorkflowRuntimeError> {
        self.commit_required_events_with_phase(app, commit)
            .await
            .map_err(RequiredEventCommitFailure::into_workflow_error)
    }

    async fn commit_required_events_with_phase<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        commit: RequiredEventCommit<'_>,
    ) -> Result<(), RequiredEventCommitFailure> {
        let RequiredEventCommit {
            execution_id,
            snapshot_for_commit,
            snapshot_before,
            execution_store_snapshot_before,
            required_events,
            append_error_context,
        } = commit;

        // Reacquire the execution mutex and keep it through the synchronous append. Every runtime
        // mutation uses this mutex, so a newer stop/completion cannot overtake this event commit.
        // If another mutation already won, this stale commit emits no fact and performs no
        // rollback. On append failure, restore the driver snapshot before releasing the mutex so
        // no concurrent mutation can observe or overwrite the failed pre-commit state.
        let (append_result, launched_as) = {
            let mut executions = self.executions.lock().await;
            let Some(current) = executions.get_mut(execution_id) else {
                return Err(RequiredEventCommitFailure::BeforeDurableAppend(
                    WorkflowRuntimeError::InvalidState(format!(
                        "execution {execution_id} disappeared before required event commit"
                    )),
                ));
            };
            if !commit_snapshot_is_current(current, snapshot_for_commit) {
                return Err(RequiredEventCommitFailure::BeforeDurableAppend(
                    WorkflowRuntimeError::InvalidState(format!(
                        "execution {execution_id} changed before required event commit"
                    )),
                ));
            }
            let launched_as = current.launched_as;
            let transaction = match PreparedWorkflowTransaction::capture_applied(
                snapshot_before.clone(),
                current.clone(),
                required_events,
                vec![WorkflowRuntimeEffect::BroadcastState],
            ) {
                Ok(transaction) => transaction,
                Err(error) => {
                    return Err(RequiredEventCommitFailure::BeforeDurableAppend(
                        WorkflowRuntimeError::InvalidState(format!(
                            "invalid workflow transaction preparation: {error:?}"
                        )),
                    ));
                }
            };
            *current = snapshot_before;
            let append_result = transaction
                .persist(current, |events| self.write_log_required_batch(app, events))
                .map(|durable| durable.into_effects())
                .map_err(|error| match error {
                    WorkflowTransactionCommitError::StaleCandidate => {
                        "workflow transaction candidate became stale".to_string()
                    }
                    WorkflowTransactionCommitError::Persistence(error) => error,
                });
            (append_result, launched_as)
        };
        let effects = match append_result {
            Ok(effects) => effects,
            Err(e) => {
                let _ = workflow_runtime_commit::restore_execution_store_active_snapshot(
                    &self.execution_store,
                    execution_store_snapshot_before,
                )
                .await;
                return Err(RequiredEventCommitFailure::BeforeDurableAppend(
                    WorkflowRuntimeError::SessionStore(format!("{append_error_context}: {e}")),
                ));
            }
        };

        self.spawn_committed_runtime_effects(effects);

        if let Err(e) = self
            .sync_state_after_required_event_commit(launched_as, snapshot_for_commit)
            .await
        {
            // Required events are the SQLite commit authority.  The
            // ExecutionStore/JSON view is a rebuildable post-commit
            // projection; its failure must not reverse an accepted command.
            log::warn!(
                "workflow {execution_id}: derived execution projection refresh failed after canonical commit: {e}"
            );
        }

        Ok(())
    }

    /// [04] post-commit phase: broadcast and runtime release. Every required
    /// transition/terminal event is already in the canonical commit; this
    /// phase contains only derived notifications and in-memory cleanup.
    async fn finalize_after_commit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &RuntimeCommitSnapshot,
        worktree_path: &str,
    ) {
        let execution_id = snapshot.execution_id.clone();
        let is_finished = matches!(
            snapshot.state,
            RuntimeExecutionState::Completed | RuntimeExecutionState::Aborted
        );
        workflow_runtime_session::broadcast_state(app, worktree_path, snapshot.clone()).await;
        if is_finished {
            self.release_terminal_execution(&execution_id).await;
        }
    }

    async fn settle_runtime_failure<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        execution_id: &str,
        error: &WorkflowRuntimeError,
    ) -> Result<(), WorkflowRuntimeError> {
        let node_execution_id = {
            let executions = self.executions.lock().await;
            let execution = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
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
        self.settle_runtime_failure_for_node(
            app,
            worktree_path,
            execution_id,
            &node_execution_id,
            error,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn settle_runtime_failure_for_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        execution_id: &str,
        node_execution_id: &str,
        error: &WorkflowRuntimeError,
    ) -> Result<(), WorkflowRuntimeError> {
        let failure_kind = error.workflow_failure_kind();
        let reason = format!("workflow runtime activation failed: {error}");
        let mut last_error = None;
        for attempt in 1..=crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS {
            match self
                .settle_node_failure_for_node(
                    app,
                    worktree_path,
                    execution_id,
                    node_execution_id,
                    reason.clone(),
                    failure_kind,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(
                    error @ (WorkflowRuntimeError::Conflict(_)
                    | WorkflowRuntimeError::SessionStore(_)),
                ) if attempt < crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS => {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            WorkflowRuntimeError::InvalidState(
                "bounded failure settlement retry ended without an error".to_string(),
            )
        }))
    }

    #[allow(clippy::too_many_arguments)]
    async fn settle_node_failure_for_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        execution_id: &str,
        node_execution_id: &str,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
    ) -> Result<(), WorkflowRuntimeError> {
        let timestamp = current_timestamp();
        let (snapshot_before, mut candidate, node_name, attempt, session_id, is_fanout_child) = {
            let executions = self.executions.lock().await;
            let execution = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
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
            (
                execution.clone(),
                execution.clone(),
                node.node_name.clone(),
                node.attempt,
                node.session_id.clone(),
                node.is_fanout_child(),
            )
        };
        let transition = candidate.fail_leaf_execution(
            node_execution_id,
            reason.clone(),
            failure_kind,
            FailureDisposition::Terminal,
            timestamp,
        );
        if transition
            != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        {
            return Ok(());
        }
        if !is_fanout_child {
            candidate.record_history_entry(
                crate::domain::workflow::NodeHistoryEntry {
                    node_name: node_name.clone(),
                    completed_at: timestamp,
                    result: Some(reason.clone()),
                    session_id,
                    token_usage: None,
                    artifact: None,
                    attempt,
                    fanout_children: None,
                    state: crate::domain::workflow::NODE_STATUS_FAILED.to_string(),
                },
                timestamp,
            );
        }
        let mut events = vec![WorkflowEvent::NodeFailed {
            execution_id: execution_id.to_string(),
            node_execution_id: node_execution_id.to_string(),
            node_name,
            attempt,
            reason,
            failure_kind,
            retry_count: None,
            timestamp,
        }];
        // children エントリの on_failure（自動 retry / ignore）。決定と適用は
        // domain が所有し、NodeFailed と同一バッチで事実を追記する。
        let candidate_before_treatment = candidate.clone();
        let mut new_id = new_node_execution_id;
        let treatment =
            match candidate.apply_on_failure_treatment(node_execution_id, &mut new_id, timestamp) {
                Ok(treatment) => treatment,
                Err(error) => {
                    log::warn!(
                        "workflow {execution_id}: on_failure treatment was not applied: {error}"
                    );
                    None
                }
            };
        let treatment_applied = treatment.is_some();
        if !treatment_applied {
            // 処遇が得られなかった場合（Err / 防御分岐の None）は、途中まで適用された
            // 状態変化が対応イベントなしで commit されないよう処遇前へ戻す。
            candidate = candidate_before_treatment;
        }
        let mut leaves = Vec::new();
        if let Some(treatment) = treatment {
            events.extend(treatment.events);
            leaves = treatment.leaves;
        }
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before,
                    candidate,
                    transition_outcome: TransitionOutcome::Applied,
                    events: &events,
                    provider_events: Vec::new(),
                },
            )
            .await?;
        let outcome = if !leaves.is_empty() {
            Some(NodeOutcome::StartLeaves(snapshot.clone(), leaves))
        } else if treatment_applied {
            // ignore 前進が leaf 起動なしで完了へ到達した場合も finalize を通す。
            Some(NodeOutcome::Persist(snapshot.clone()))
        } else {
            None
        };
        self.finish_control_plane_commit(app, worktree_path, &snapshot, outcome)
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
    async fn dispatch_node_outcome_side_effects<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        outcome: NodeOutcome,
    ) -> Result<(), WorkflowRuntimeError> {
        if self
            .recovery_effects_suppressed(&outcome.snapshot().execution_id)
            .await
        {
            return Ok(());
        }
        match outcome {
            NodeOutcome::Persist(_) => Ok(()),
            NodeOutcome::StartLeaves(snapshot, leaves) => {
                if let Err(e) =
                    Box::pin(self.start_leaves(app, &snapshot.execution_id, worktree_path, leaves))
                        .await
                {
                    if let Err(settle_error) = Box::pin(self.settle_runtime_failure(
                        app,
                        worktree_path,
                        &snapshot.execution_id,
                        &e,
                    ))
                    .await
                    {
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
    fn write_log_required_batch<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        events: &[WorkflowEvent],
    ) -> Result<(), String> {
        workflow_event_log_writer::append_required_events_for_app(app, events)
    }
}

#[cfg(test)]
mod workflow_host_tests {
    use super::*;
    use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
    use crate::adaptor::gateway::local_event_store::fault::FaultInjector;
    use crate::adaptor::gateway::local_event_store::node_events::NewNodeEventRow;
    use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
    use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
    use crate::adaptor::gateway::workflow::TauriWorkflowRuntimeCommandGateway;
    use crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository;
    use crate::adaptor::protocol::workflow::{
        NodeExecutionStatusView, WorkflowExecutionChangedPayloadView,
    };
    use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
    use crate::domain::agent_session::repository::AgentSessionRepository;
    use crate::domain::local_event::{
        LoadStreamRequest, LocalEventTransactionRepository, StreamId,
    };
    use crate::domain::provider_lifecycle::{
        ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
    };
    use crate::domain::workflow::{
        ChildEntry, CommandSpec, ExecutionParentRef, ExecutionTreeLaunch, FacetRefs, FanoutSpec,
        NodeCompletion, NodeDefinition, NodeFact, NodeFactMeta, NodeKind, SequenceSpec,
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
    use tauri::Listener as _;

    const EFFECT_WORKTREE_PATH: &str = "/repo/effect-test";
    const EFFECT_NODE_NAME: &str = "agent";
    const EFFECT_AGENT_SESSION_ID: &str = "agent-session-effect-test";

    struct UnusedWorkflowResolver;

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

    struct AcceptingWorktreeResolver;

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
        dispatch_fails: Arc<std::sync::atomic::AtomicBool>,
        failing_agent_session_id: String,
    }

    struct MissingRepoWorktreeInventory;

    impl crate::domain::workflow::WorktreeInventoryGateway for MissingRepoWorktreeInventory {
        fn snapshot(
            &self,
        ) -> Result<
            Vec<crate::domain::workflow::RepositoryWorktreeInventory>,
            crate::domain::workflow::WorkflowError,
        > {
            Ok(vec![
                crate::domain::workflow::RepositoryWorktreeInventory::new("/repo", Vec::new()),
            ])
        }
    }

    #[tokio::test]
    async fn test_command_env_未束縛inputではprocessを起動せずnode_failureにする() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ))
        .unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(store.clone());
        app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            directory.path().to_path_buf(),
        ));
        let host = WorkflowRuntimeHost::with_execution_store(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            Arc::new(FailingWorkflowAgentSessions),
            Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            ),
            Arc::new(MissingRepoWorktreeInventory),
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
                app.handle(),
                workflow,
                directory.path().to_string_lossy().into_owned(),
                None,
                ExecutionOrigin::DesktopUi,
            )
            .await
            .unwrap();

        assert!(!directory.path().join("command-spawned.marker").exists());
        let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();
        assert_eq!(snapshot.node_executions.len(), 1);
        assert_eq!(
            snapshot.node_executions[0].status,
            NodeExecutionStatus::Failed
        );
        let records = workflow_fact_log::read_tree_records(&store, &execution_id).unwrap();
        assert!(records.iter().any(|record| matches!(
            &record.fact,
            NodeFact::ProcessExited(fact) if fact.failure_reason.is_some()
        )));
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
    }

    #[tokio::test]
    async fn test_command_env_nulによるspawn失敗を既存node_failureにする() {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ))
        .unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(store.clone());
        app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
            directory.path().to_path_buf(),
        ));
        let host = WorkflowRuntimeHost::with_execution_store(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            Arc::new(FailingWorkflowAgentSessions),
            Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            ),
            Arc::new(MissingRepoWorktreeInventory),
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
                app.handle(),
                workflow,
                directory.path().to_string_lossy().into_owned(),
                Some("before\0after".to_string()),
                ExecutionOrigin::DesktopUi,
            )
            .await
            .unwrap();

        assert!(!directory.path().join("command-spawned.marker").exists());
        let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();
        assert_eq!(
            snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "run")
                .map(|node| node.status),
            Some(NodeExecutionStatus::Failed)
        );
        let records = workflow_fact_log::read_tree_records(&store, &execution_id).unwrap();
        assert!(records.iter().any(|record| matches!(
            &record.fact,
            NodeFact::ProcessExited(fact) if fact.failure_reason.is_some()
        )));
        assert!(!records
            .iter()
            .any(|record| matches!(record.fact, NodeFact::CommandSpawned(_))));
    }

    #[async_trait::async_trait]
    impl WorkflowAgentSessionPort for FailingWorkflowAgentSessions {
        fn is_provider_available(&self, _provider: ProviderKind) -> bool {
            true
        }

        async fn prepare_workflow_agent_session(
            &self,
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

        async fn dispatch_initial_instruction(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
            _instruction: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn recover_workflow_agent_session_provider(
            &self,
            _node_session_id: &str,
            _node_execution_id: &str,
        ) -> Result<(), WorkflowRuntimeError> {
            unreachable!()
        }

        async fn interrupt_workflow_agent_session(
            &self,
            _node_session_id: &str,
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
            dispatch_fails: Arc<std::sync::atomic::AtomicBool>,
            failing_agent_session_id: String,
        ) -> Arc<dyn WorkflowAgentSessionPort> {
            Arc::new(RecordingWorkflowAgentSessions {
                stop_calls,
                prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                provider_running_checks,
                recovery_fails,
                dispatch_fails,
                failing_agent_session_id,
            })
        }

        const RESUME_PAUSED_SIBLING_NODES: [(&str, NodeKindName); 3] = [
            ("resume-paused-sequence", NodeKindName::Sequence),
            ("resume-paused-fanout", NodeKindName::Fanout),
            ("resume-paused-command", NodeKindName::Command),
        ];

        fn resume_paused_sibling_definitions(child_node_name: &str) -> Vec<NodeDefinition> {
            vec![
                NodeDefinition {
                    name: RESUME_PAUSED_SIBLING_NODES[0].0.to_string(),
                    kind: NodeKind::Sequence(SequenceSpec {
                        entry: None,
                        output: None,
                        children: vec![ChildEntry::reference(child_node_name)],
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::Auto,
                    worktree: None,
                },
                NodeDefinition {
                    name: RESUME_PAUSED_SIBLING_NODES[1].0.to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        children: vec![ChildEntry::reference(child_node_name)],
                        items: None,
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::Auto,
                    worktree: None,
                },
                NodeDefinition {
                    name: RESUME_PAUSED_SIBLING_NODES[2].0.to_string(),
                    kind: NodeKind::Command(CommandSpec {
                        command: "unused".to_string(),
                        env: Default::default(),
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::Auto,
                    worktree: None,
                },
            ]
        }

        #[derive(Debug, Clone, Copy)]
        enum ResumeCommitFailureMode {
            StaleCandidate,
            Persistence,
            PersistenceWithCompensationFailure,
        }

        #[derive(Clone)]
        struct ResumeCommitFailureBinding {
            host: std::sync::Weak<WorkflowRuntimeHost>,
            store: Arc<LocalEventStore>,
            execution_id: String,
            database_path: std::path::PathBuf,
        }

        struct ResumeCommitFailureWorkflowAgentSessions {
            mode: ResumeCommitFailureMode,
            binding: std::sync::Mutex<Option<ResumeCommitFailureBinding>>,
            provider_launches: std::sync::atomic::AtomicUsize,
            recovery_calls: std::sync::atomic::AtomicUsize,
        }

        #[derive(Clone)]
        struct PartialRecoveryBinding {
            store: Arc<LocalEventStore>,
            execution_id: String,
            database_path: std::path::PathBuf,
        }

        struct PartialRecoveryWorkflowAgentSessions {
            binding: std::sync::Mutex<Option<PartialRecoveryBinding>>,
            open_sessions: std::sync::Mutex<HashSet<String>>,
            failing_node_execution_id: std::sync::Mutex<Option<String>>,
            failure_enabled: std::sync::atomic::AtomicBool,
            compensation_fails: std::sync::atomic::AtomicBool,
            persist_resumes: std::sync::atomic::AtomicBool,
            provider_launches: std::sync::Mutex<HashMap<String, usize>>,
            recovery_calls: std::sync::Mutex<Vec<(String, String)>>,
        }

        impl PartialRecoveryWorkflowAgentSessions {
            fn new() -> Self {
                Self {
                    binding: std::sync::Mutex::new(None),
                    open_sessions: std::sync::Mutex::new(HashSet::new()),
                    failing_node_execution_id: std::sync::Mutex::new(None),
                    failure_enabled: std::sync::atomic::AtomicBool::new(true),
                    compensation_fails: std::sync::atomic::AtomicBool::new(false),
                    persist_resumes: std::sync::atomic::AtomicBool::new(true),
                    provider_launches: std::sync::Mutex::new(HashMap::new()),
                    recovery_calls: std::sync::Mutex::new(Vec::new()),
                }
            }

            fn bind(
                &self,
                store: Arc<LocalEventStore>,
                execution_id: &str,
                database_path: std::path::PathBuf,
            ) {
                *self.binding.lock().unwrap() = Some(PartialRecoveryBinding {
                    store,
                    execution_id: execution_id.to_string(),
                    database_path,
                });
            }

            fn close_all(&self) {
                self.open_sessions.lock().unwrap().clear();
            }

            fn fail_on(&self, node_execution_id: &str) {
                *self.failing_node_execution_id.lock().unwrap() =
                    Some(node_execution_id.to_string());
            }

            fn allow_recovery(&self) {
                self.failure_enabled
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }

            fn fail_compensation(&self) {
                self.compensation_fails
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }

            fn skip_persisted_resume(&self) {
                self.persist_resumes
                    .store(false, std::sync::atomic::Ordering::SeqCst);
            }

            fn is_open(&self, session_id: &str) -> bool {
                self.open_sessions.lock().unwrap().contains(session_id)
            }

            fn provider_launch_count(&self, session_id: &str) -> usize {
                self.provider_launches
                    .lock()
                    .unwrap()
                    .get(session_id)
                    .copied()
                    .unwrap_or(0)
            }

            fn append_provider_resume(&self, node_execution_id: &str) {
                let binding = self.binding.lock().unwrap().clone().unwrap();
                let records =
                    workflow_fact_log::read_tree_records(&binding.store, &binding.execution_id)
                        .unwrap();
                let meta = records
                    .iter()
                    .find(|record| record.meta.node_execution_id == node_execution_id)
                    .unwrap()
                    .meta
                    .clone();
                workflow_fact_log::append_single_fact(
                    &binding.store,
                    &meta,
                    &NodeFact::ResumeRequested,
                    records.last().unwrap().timestamp_ms + 1,
                )
                .unwrap();
            }
        }

        impl ResumeCommitFailureWorkflowAgentSessions {
            fn new(mode: ResumeCommitFailureMode) -> Self {
                Self {
                    mode,
                    binding: std::sync::Mutex::new(None),
                    provider_launches: std::sync::atomic::AtomicUsize::new(0),
                    recovery_calls: std::sync::atomic::AtomicUsize::new(0),
                }
            }

            fn bind(&self, fixture: &RuntimeEffectFixture) {
                *self.binding.lock().unwrap() = Some(ResumeCommitFailureBinding {
                    host: Arc::downgrade(&fixture.host),
                    store: fixture.store.clone(),
                    execution_id: fixture.execution_id.clone(),
                    database_path: fixture._directory.path().join("local-event-store.sqlite3"),
                });
            }

            fn clear_persistence_failure(&self) {
                let binding = self.binding.lock().unwrap().clone().unwrap();
                rusqlite::Connection::open(binding.database_path)
                    .unwrap()
                    .execute_batch("DROP TRIGGER IF EXISTS fail_resume_control_commit")
                    .unwrap();
            }
        }

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for ResumeCommitFailureWorkflowAgentSessions {
            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
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

            async fn dispatch_initial_instruction(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.recovery_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let binding = self
                    .binding
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("resume failure fixture is bound before recovery");
                let repository = LocalAgentSessionRepository::new(binding.store.clone());
                let session = repository.find(node_session_id).await.unwrap().unwrap();
                if session.session().lifecycle()
                    == crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open
                {
                    return Ok(());
                }
                self.provider_launches
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let records =
                    workflow_fact_log::read_tree_records(&binding.store, &binding.execution_id)
                        .unwrap();
                let meta = records
                    .iter()
                    .find(|record| record.meta.node_execution_id == node_execution_id)
                    .unwrap()
                    .meta
                    .clone();
                workflow_fact_log::append_single_fact(
                    &binding.store,
                    &meta,
                    &NodeFact::ResumeRequested,
                    records.last().unwrap().timestamp_ms + 1,
                )
                .unwrap();
                match self.mode {
                    ResumeCommitFailureMode::StaleCandidate => {
                        let host = binding.host.upgrade().unwrap();
                        let mut executions = host.executions.lock().await;
                        executions
                            .get_mut(&binding.execution_id)
                            .unwrap()
                            .updated_at += 1.0;
                    }
                    ResumeCommitFailureMode::Persistence => {
                        rusqlite::Connection::open(&binding.database_path)
                            .unwrap()
                            .execute_batch(
                                "CREATE TRIGGER fail_resume_control_commit
                                 BEFORE INSERT ON node_events
                                 WHEN NEW.event_type = 'resume_requested'
                                 BEGIN
                                   SELECT RAISE(ABORT, 'injected resume persistence failure');
                                 END;",
                            )
                            .unwrap();
                    }
                    ResumeCommitFailureMode::PersistenceWithCompensationFailure => {
                        rusqlite::Connection::open(&binding.database_path)
                            .unwrap()
                            .execute_batch(
                                "CREATE TRIGGER fail_resume_control_commit
                                 BEFORE INSERT ON node_events
                                 WHEN NEW.event_type IN ('resume_requested', 'process_exited')
                                 BEGIN
                                   SELECT RAISE(ABORT, 'injected resume persistence failure');
                                 END;",
                            )
                            .unwrap();
                    }
                }
                Ok(())
            }

            async fn interrupt_workflow_agent_session(
                &self,
                _node_session_id: &str,
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

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for PartialRecoveryWorkflowAgentSessions {
            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
                _worktree_path: &str,
                _config: WorkflowSessionLaunchConfig,
                _workflow_execution_id: &str,
                node_execution_id: &str,
                _initial_instruction: &str,
            ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
                let session_id = format!("agent-session-{node_execution_id}");
                self.open_sessions
                    .lock()
                    .unwrap()
                    .insert(session_id.clone());
                Ok(NodeSessionInfo { id: session_id })
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

            async fn dispatch_initial_instruction(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                node_session_id: &str,
                node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                self.recovery_calls
                    .lock()
                    .unwrap()
                    .push((node_execution_id.to_string(), node_session_id.to_string()));
                if self.is_open(node_session_id) {
                    return Ok(());
                }
                if self
                    .failure_enabled
                    .load(std::sync::atomic::Ordering::SeqCst)
                    && self.failing_node_execution_id.lock().unwrap().as_deref()
                        == Some(node_execution_id)
                {
                    if self
                        .compensation_fails
                        .load(std::sync::atomic::Ordering::SeqCst)
                    {
                        let binding = self.binding.lock().unwrap().clone().unwrap();
                        rusqlite::Connection::open(binding.database_path)
                            .unwrap()
                            .execute_batch(
                                "CREATE TRIGGER fail_partial_recovery_compensation
                                 BEFORE INSERT ON node_events
                                 WHEN NEW.event_type = 'process_exited'
                                 BEGIN
                                   SELECT RAISE(ABORT, 'injected partial recovery compensation failure');
                                 END;",
                            )
                            .unwrap();
                    }
                    return Err(WorkflowRuntimeError::AgentSession(
                        "intentional partial provider recovery failure".to_string(),
                    ));
                }
                if self
                    .persist_resumes
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    self.append_provider_resume(node_execution_id);
                }
                self.open_sessions
                    .lock()
                    .unwrap()
                    .insert(node_session_id.to_string());
                *self
                    .provider_launches
                    .lock()
                    .unwrap()
                    .entry(node_session_id.to_string())
                    .or_default() += 1;
                Ok(())
            }

            async fn interrupt_workflow_agent_session(
                &self,
                _node_session_id: &str,
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

        #[async_trait::async_trait]
        impl WorkflowAgentSessionPort for RecordingWorkflowAgentSessions {
            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
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

            async fn dispatch_initial_instruction(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                if self
                    .dispatch_fails
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Err(WorkflowRuntimeError::AgentSession(
                        "intentional instruction dispatch failure".to_string(),
                    ));
                }
                Ok(())
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

            async fn interrupt_workflow_agent_session(
                &self,
                _node_session_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
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
            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
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

            async fn dispatch_initial_instruction(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn interrupt_workflow_agent_session(
                &self,
                _node_session_id: &str,
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
            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
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

            async fn dispatch_initial_instruction(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn interrupt_workflow_agent_session(
                &self,
                _node_session_id: &str,
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
            fn is_provider_available(&self, _provider: ProviderKind) -> bool {
                true
            }

            async fn prepare_workflow_agent_session(
                &self,
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

            async fn dispatch_initial_instruction(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
                _instruction: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn recover_workflow_agent_session_provider(
                &self,
                _node_session_id: &str,
                _node_execution_id: &str,
            ) -> Result<(), WorkflowRuntimeError> {
                Ok(())
            }

            async fn interrupt_workflow_agent_session(
                &self,
                _node_session_id: &str,
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
            app: tauri::App<tauri::test::MockRuntime>,
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
            _app: tauri::App<tauri::test::MockRuntime>,
            host: Arc<WorkflowRuntimeHost>,
            control_plane: WorkflowControlPlaneUsecase,
            calls: Arc<std::sync::Mutex<Vec<RuntimeEffectCall>>>,
            execution_id: String,
            first_node_execution_id: String,
            first_agent_session_id: String,
            _directory: tempfile::TempDir,
        }

        struct MultiResumeFixture {
            app: tauri::App<tauri::test::MockRuntime>,
            store: Arc<LocalEventStore>,
            host: Arc<WorkflowRuntimeHost>,
            execution_id: String,
            sessions: Vec<(String, String)>,
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
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                directory.path().to_path_buf(),
            ));
            let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
            Arc::new(UnusedWorkflowResolver),
            Arc::new(AcceptingWorktreeResolver),
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            sessions,
            Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            ),
            Arc::new(MissingRepoWorktreeInventory),
        ));
            let mut nodes = vec![NodeDefinition {
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
            nodes.extend(resume_paused_sibling_definitions(EFFECT_NODE_NAME));
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
                    app.handle(),
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();
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
                "unexpected activation failure: {:?}",
                node.failure
            );
            assert_eq!(node.session_id.as_deref(), Some(EFFECT_AGENT_SESSION_ID));
            let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
            let gateway = Arc::new(TauriWorkflowRuntimeCommandGateway::new_with_driver(
                app.handle().clone(),
                host.clone(),
                repository,
                store.installation_id().to_string(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(gateway);
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

        async fn multi_resume_fixture(
            sessions: Arc<dyn WorkflowAgentSessionPort>,
        ) -> MultiResumeFixture {
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                directory.path().to_path_buf(),
            ));
            let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                sessions,
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        store.clone(),
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
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
                completion: NodeCompletion::Auto,
                worktree: None,
            };
            let mut nodes = vec![
                NodeDefinition {
                    name: "main".to_string(),
                    kind: NodeKind::Fanout(FanoutSpec {
                        children: vec![
                            ChildEntry::reference("agent-first"),
                            ChildEntry::reference("agent-second"),
                        ],
                        items: None,
                    }),
                    artifact: None,
                    input: Vec::new(),
                    completion: NodeCompletion::Auto,
                    worktree: None,
                },
                session_node("agent-first"),
                session_node("agent-second"),
            ];
            nodes.extend(resume_paused_sibling_definitions("agent-first"));
            let workflow = WorkflowDefinition {
                name: "partial-provider-recovery".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes,
                entry: "main".to_string(),
            };
            let execution_id = host
                .start_resolved_workflow(
                    app.handle(),
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();
            let mut session_nodes = snapshot
                .node_executions
                .iter()
                .filter(|node| node.kind == NodeKindName::Session)
                .collect::<Vec<_>>();
            session_nodes.sort_by(|left, right| left.node_name.cmp(&right.node_name));
            let sessions = session_nodes
                .into_iter()
                .map(|node| (node.id.clone(), node.session_id.clone().unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(sessions.len(), 2);
            MultiResumeFixture {
                app,
                store,
                host,
                execution_id,
                sessions,
                _directory: directory,
            }
        }

        async fn sequential_runtime_effect_fixture() -> SequentialRuntimeEffectFixture {
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                directory.path().to_path_buf(),
            ));
            let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(OrderedWorkflowAgentSessions {
                    calls: calls.clone(),
                }),
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        store.clone(),
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
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
                completion: NodeCompletion::Auto,
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
                            output: None,
                            children: vec![
                                ChildEntry::reference("agent-one"),
                                ChildEntry::reference("agent-two"),
                            ],
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: NodeCompletion::Auto,
                        worktree: None,
                    },
                    session_node("agent-one"),
                    session_node("agent-two"),
                ],
                entry: "main".to_string(),
            };
            let execution_id = host
                .start_resolved_workflow(
                    app.handle(),
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();
            let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();
            let first = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "agent-one")
                .unwrap();
            assert_eq!(first.status, NodeExecutionStatus::Running);
            let first_node_execution_id = first.id.clone();
            let first_agent_session_id = first.session_id.clone().unwrap();
            let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
            let gateway = Arc::new(TauriWorkflowRuntimeCommandGateway::new_with_driver(
                app.handle().clone(),
                host.clone(),
                repository,
                store.installation_id().to_string(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(gateway);
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

        fn persisted_node_status(fixture: &RuntimeEffectFixture) -> NodeExecutionStatus {
            persisted_node(fixture).status
        }

        fn persisted_node(
            fixture: &RuntimeEffectFixture,
        ) -> crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution {
            persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &fixture.node_execution_id,
            )
        }

        fn persisted_node_for(
            store: &Arc<LocalEventStore>,
            execution_id: &str,
            node_execution_id: &str,
        ) -> crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution {
            let backend = workflow_fact_log::FactLogReadBackend::Live(store.clone());
            workflow_fact_log::fold_tree_from(&backend, execution_id)
                .unwrap()
                .unwrap()
                .aggregate
                .node_executions
                .iter()
                .find(|node| node.id == node_execution_id)
                .unwrap()
                .clone()
        }

        fn append_process_exit(fixture: &RuntimeEffectFixture, exit_code: Option<i32>) {
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &fixture.node_execution_id,
                exit_code,
            );
        }

        fn append_process_exit_for_node(
            store: &Arc<LocalEventStore>,
            execution_id: &str,
            node_execution_id: &str,
            exit_code: Option<i32>,
        ) {
            let records = workflow_fact_log::read_tree_records(store, execution_id).unwrap();
            let meta = records
                .iter()
                .find(|record| record.meta.node_execution_id == node_execution_id)
                .unwrap()
                .meta
                .clone();
            workflow_fact_log::append_single_fact(
                store,
                &meta,
                &NodeFact::ProcessExited(crate::domain::workflow::ProcessExitedFact {
                    exit_code,
                    result_summary: None,
                    failure_reason: None,
                    failure_kind: None,
                }),
                records.last().unwrap().timestamp_ms + 1,
            )
            .unwrap();
        }

        async fn stop_with_resume_paused_siblings<R: tauri::Runtime + 'static>(
            app: &tauri::AppHandle<R>,
            store: &Arc<LocalEventStore>,
            host: &Arc<WorkflowRuntimeHost>,
            execution_id: &str,
        ) -> Vec<String> {
            let mut timestamp_ms = workflow_fact_log::read_tree_records(store, execution_id)
                .unwrap()
                .last()
                .unwrap()
                .timestamp_ms
                + 1;
            let node_execution_ids = RESUME_PAUSED_SIBLING_NODES
                .iter()
                .map(|(node_name, kind)| {
                    let node_execution_id = format!("{execution_id}-{node_name}");
                    workflow_fact_log::append_single_fact(
                        store,
                        &NodeFactMeta {
                            tree_id: execution_id.to_string(),
                            node_execution_id: node_execution_id.clone(),
                            parent_id: None,
                            node_name: (*node_name).to_string(),
                            kind: *kind,
                            attempt: 1,
                        },
                        &NodeFact::Started(StartedFact {
                            parent: None,
                            root: None,
                        }),
                        timestamp_ms,
                    )
                    .unwrap();
                    timestamp_ms += 1;
                    node_execution_id
                })
                .collect::<Vec<_>>();
            let durable = workflow_fact_log::fold_tree_from(
                &workflow_fact_log::FactLogReadBackend::Live(store.clone()),
                execution_id,
            )
            .unwrap()
            .unwrap()
            .aggregate;
            host.executions
                .lock()
                .await
                .insert(execution_id.to_string(), durable);

            host.stop_workflow_execution(app, execution_id)
                .await
                .unwrap();
            let snapshot = host.get_state_by_execution_id(execution_id).await.unwrap();
            for node_execution_id in &node_execution_ids {
                assert_eq!(
                    snapshot
                        .node_executions
                        .iter()
                        .find(|node| node.id == *node_execution_id)
                        .unwrap()
                        .status,
                    NodeExecutionStatus::Paused
                );
            }
            node_execution_ids
        }

        fn record_workflow_execution_broadcasts<R: tauri::Runtime>(
            app: &tauri::AppHandle<R>,
        ) -> Arc<std::sync::Mutex<Vec<WorkflowExecutionChangedPayloadView>>> {
            let broadcasts = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = broadcasts.clone();
            app.listen("workflow-execution-changed", move |event| {
                recorded
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(event.payload()).unwrap());
            });
            broadcasts
        }

        async fn assert_resume_paused_siblings_in_memory(
            host: &Arc<WorkflowRuntimeHost>,
            execution_id: &str,
            node_execution_ids: &[String],
        ) {
            let snapshot = host.get_state_by_execution_id(execution_id).await.unwrap();
            for node_execution_id in node_execution_ids {
                assert_eq!(
                    snapshot
                        .node_executions
                        .iter()
                        .find(|node| node.id == *node_execution_id)
                        .unwrap()
                        .status,
                    NodeExecutionStatus::Paused
                );
            }
        }

        fn assert_resume_paused_siblings_in_latest_broadcast(
            broadcasts: &Arc<std::sync::Mutex<Vec<WorkflowExecutionChangedPayloadView>>>,
            node_execution_ids: &[String],
        ) {
            let broadcasts = broadcasts.lock().unwrap();
            let snapshot = &broadcasts
                .last()
                .expect("resume compensation broadcasts its restored snapshot")
                .workflow_execution;
            for node_execution_id in node_execution_ids {
                assert_eq!(
                    snapshot
                        .node_executions
                        .iter()
                        .find(|node| node.id == *node_execution_id)
                        .unwrap()
                        .status,
                    NodeExecutionStatusView::Paused
                );
            }
        }

        #[tokio::test]
        async fn test_deleted実行木解放_executions_cacheを除去する() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
            assert!(fixture
                .host
                .executions
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
                .executions
                .lock()
                .await
                .contains_key(&fixture.execution_id));
        }

        #[tokio::test]
        async fn test_登録済みworkflow実行木の予約はno_opになる() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;

            fixture
                .host
                .reserve_started_execution_tree(&fixture.execution_id)
                .await
                .unwrap();

            assert!(!fixture
                .host
                .execution_tree_reservations
                .lock()
                .await
                .contains(&fixture.execution_id));
            assert!(fixture
                .host
                .executions
                .lock()
                .await
                .contains_key(&fixture.execution_id));
        }

        #[tokio::test]
        async fn test_started実行木登録_store未管理ならsession_storeを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
            let unmanaged_app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();

            let error = fixture
                .host
                .register_started_execution_tree(unmanaged_app.handle(), "unmanaged-tree")
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
        }

        #[tokio::test]
        async fn test_started実行木登録_tree不在ならexecution_not_foundを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
            let missing_tree_id = "missing-started-tree";

            let error = fixture
                .host
                .register_started_execution_tree(fixture.app.handle(), missing_tree_id)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                WorkflowRuntimeError::ExecutionNotFound(tree_id) if tree_id == missing_tree_id
            ));
        }

        #[tokio::test]
        async fn test_started実行木登録_inactive_treeならinvalid_stateを返す() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
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
            .unwrap();

            let error = fixture
                .host
                .register_started_execution_tree(fixture.app.handle(), session_id)
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::InvalidState(_)));
        }

        #[tokio::test]
        async fn test_session実行木予約中のreconciliationは喪失を記録せず登録後のstopを保持する() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
            let session_id = "agent-session-reserved-before-commit";
            fixture
                .host
                .reserve_started_execution_tree(session_id)
                .await
                .unwrap();
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

            fixture
                .host
                .reconcile_startup(fixture.app.handle())
                .await
                .unwrap();

            let records = workflow_fact_log::read_tree_records(&fixture.store, session_id).unwrap();
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ProcessExited(_))));
            fixture
                .host
                .register_started_execution_tree(fixture.app.handle(), session_id)
                .await
                .unwrap();
            fixture
                .host
                .release_started_execution_tree_reservation(session_id)
                .await
                .unwrap();
            assert!(fixture
                .host
                .execution_tree_reservations
                .lock()
                .await
                .is_empty());

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

            let records = workflow_fact_log::read_tree_records(&fixture.store, session_id).unwrap();
            assert!(records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::StopReceived(_))));
            let backend = workflow_fact_log::FactLogReadBackend::Live(fixture.store.clone());
            let folded = workflow_fact_log::fold_tree_from(&backend, session_id)
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
                .unwrap()
                .unwrap();
            assert_eq!(
                workspace_node.status_classification,
                WorkspaceNodeStatusClassification::Attention
            );

            let restarted = WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        fixture.store.clone(),
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
            );
            restarted
                .reconcile_startup(fixture.app.handle())
                .await
                .unwrap();

            let restarted_fold = workflow_fact_log::fold_tree_from(&backend, session_id)
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
                    .unwrap()
                    .unwrap()
                    .status_classification,
                WorkspaceNodeStatusClassification::Attention
            );
            assert!(
                !workflow_fact_log::read_tree_records(&fixture.store, session_id)
                    .unwrap()
                    .iter()
                    .any(|record| matches!(record.fact, NodeFact::ProcessExited(_)))
            );
        }

        #[tokio::test]
        async fn test_session実行木登録失敗後に予約を解放するとreconciliation対象へ戻る() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
            let session_id = "agent-session-registration-failed";
            fixture
                .host
                .reserve_started_execution_tree(session_id)
                .await
                .unwrap();
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
            let unmanaged_app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            assert!(fixture
                .host
                .register_started_execution_tree(unmanaged_app.handle(), session_id)
                .await
                .is_err());

            fixture
                .host
                .release_started_execution_tree_reservation(session_id)
                .await
                .unwrap();
            fixture
                .host
                .reconcile_startup(fixture.app.handle())
                .await
                .unwrap();

            let records = workflow_fact_log::read_tree_records(&fixture.store, session_id).unwrap();
            assert!(records
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
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                directory.path().to_path_buf(),
            ));
            let sessions = Arc::new(StopDuringActivationWorkflowAgentSessions {
                control_plane: tokio::sync::Mutex::new(None),
                execution_id: std::sync::Mutex::new(None),
                activation_count: std::sync::atomic::AtomicUsize::new(0),
                confirmation_count: std::sync::atomic::AtomicUsize::new(0),
            });
            let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                sessions.clone(),
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        store.clone(),
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
            ));
            let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
            let gateway = Arc::new(TauriWorkflowRuntimeCommandGateway::new_with_driver(
                app.handle().clone(),
                host.clone(),
                repository,
                store.installation_id().to_string(),
            ));
            *sessions.control_plane.lock().await =
                Some(Arc::new(WorkflowControlPlaneUsecase::new(gateway)));
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
                    completion: NodeCompletion::Auto,
                    worktree: None,
                }],
                entry: "main".to_string(),
            };

            // When
            let execution_id = host
                .start_resolved_workflow(
                    app.handle(),
                    workflow,
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();

            // Then
            let snapshot = host.get_state_by_execution_id(&execution_id).await.unwrap();
            let node = snapshot
                .node_executions
                .iter()
                .find(|node| node.node_name == "main")
                .unwrap();
            assert_eq!(
                node.status,
                NodeExecutionStatus::Running,
                "unexpected activation failure: {:?}",
                node.failure
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
            let records = workflow_fact_log::read_tree_records(&store, &execution_id).unwrap();
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
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
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
                .register_started_execution_tree(fixture.app.handle(), standalone_id)
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
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                directory.path().to_path_buf(),
            ));
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
            let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(RecordingWorkflowAgentSessions {
                    stop_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    prepare_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                    provider_running_checks: Arc::new(std::sync::Mutex::new(Vec::new())),
                    recovery_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    dispatch_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    failing_agent_session_id: String::new(),
                }),
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        store,
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
            ));
            host.register_started_execution_tree(app.handle(), session_id)
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
                    completion: NodeCompletion::Auto,
                    worktree: None,
                }],
                entry: "main".to_string(),
            };

            // When: workflow の実行として同じ worktree に木を起こす
            let workflow_id = host
                .start_resolved_workflow(
                    app.handle(),
                    workflow.clone(),
                    EFFECT_WORKTREE_PATH.to_string(),
                    None,
                    ExecutionOrigin::DesktopUi,
                )
                .await
                .unwrap();

            // Then: cache は両方を保持し、workflow registry は workflow だけを保持する
            assert!(host.get_state_by_execution_id(session_id).await.is_some());
            assert!(host.get_state_by_execution_id(&workflow_id).await.is_some());
            assert_eq!(host.execution_store.list_active().await.unwrap().len(), 1);
            let second = host
                .start_resolved_workflow(
                    app.handle(),
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
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
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
                persisted_node_status(&fixture),
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
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, true).await;
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
                persisted_node_status(&fixture),
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
                .get_state_by_execution_id(&fixture.execution_id)
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
                NodeCompletion::Auto,
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
                persisted_node_status(&fixture),
                NodeExecutionStatus::Succeeded
            );
        }

        #[tokio::test]
        async fn test_終端済みsessionへの再stop_確定状態と停止回数を変えない() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
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
                persisted_node_status(&fixture),
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
                persisted_node_status(&fixture),
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_承認_agent_session停止失敗でも成功とsucceededを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::Approval, true).await;
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
                .get_state_by_execution_id(&fixture.execution_id)
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
                persisted_node_status(&fixture),
                NodeExecutionStatus::Succeeded
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_failure_settlement_agent_session停止失敗でも成功とfailedを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, true).await;
            let runtime_error = WorkflowRuntimeError::AgentSession("runtime failed".to_string());

            // When
            let result = fixture
                .host
                .settle_runtime_failure_for_node(
                    fixture.app.handle(),
                    EFFECT_WORKTREE_PATH,
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
                .get_state_by_execution_id(&fixture.execution_id)
                .await
                .unwrap();
            assert_eq!(
                settled
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Failed
            );
            wait_for_single_terminal_stop(&fixture).await;
        }

        #[tokio::test]
        async fn test_workflow起動木_runtime失敗sessionをresume対象にしない() {
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, false).await;
            let runtime_error = WorkflowRuntimeError::AgentSession("runtime failed".to_string());
            fixture
                .host
                .settle_runtime_failure_for_node(
                    fixture.app.handle(),
                    EFFECT_WORKTREE_PATH,
                    &fixture.execution_id,
                    &fixture.node_execution_id,
                    &runtime_error,
                )
                .await
                .unwrap();
            assert_eq!(persisted_node_status(&fixture), NodeExecutionStatus::Failed);

            fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap();

            assert_eq!(persisted_node_status(&fixture), NodeExecutionStatus::Failed);
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &fixture.execution_id)
                    .unwrap();
            assert!(!records.iter().any(|record| {
                record.meta.node_execution_id == fixture.node_execution_id
                    && matches!(record.fact, NodeFact::ResumeRequested)
            }));
        }

        #[tokio::test]
        async fn test_process_exited事実からresume対象を再構成してresume_requestedを記録する() {
            for (exit_code, expected_before_resume) in [
                (Some(1), NodeExecutionStatus::Failed),
                (Some(0), NodeExecutionStatus::Paused),
            ] {
                let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
                let provider_running_checks = Arc::new(std::sync::Mutex::new(Vec::new()));
                let sessions = recording_agent_sessions(
                    stop_calls.clone(),
                    provider_running_checks.clone(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    String::new(),
                );
                let fixture = runtime_effect_fixture_with_sessions(
                    NodeCompletion::Auto,
                    sessions,
                    stop_calls,
                )
                .await;
                append_process_exit(&fixture, exit_code);

                assert_eq!(persisted_node_status(&fixture), expected_before_resume);
                assert_eq!(
                    fixture
                        .host
                        .get_state_by_execution_id(&fixture.execution_id)
                        .await
                        .unwrap()
                        .node_executions
                        .iter()
                        .find(|node| node.id == fixture.node_execution_id)
                        .unwrap()
                        .status,
                    NodeExecutionStatus::Running
                );

                fixture
                    .host
                    .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                    .await
                    .unwrap();

                assert_eq!(
                    persisted_node_status(&fixture),
                    NodeExecutionStatus::Running
                );
                assert_eq!(
                    provider_running_checks.lock().unwrap().as_slice(),
                    &[(
                        fixture.node_execution_id.clone(),
                        EFFECT_AGENT_SESSION_ID.to_string(),
                    )]
                );
                let records =
                    workflow_fact_log::read_tree_records(&fixture.store, &fixture.execution_id)
                        .unwrap();
                assert!(records.iter().any(|record| {
                    record.meta.node_execution_id == fixture.node_execution_id
                        && matches!(record.fact, NodeFact::ResumeRequested)
                }));
            }
        }

        #[tokio::test]
        async fn test_provider復旧失敗ではresumeを失敗させnodeをfailedに維持する() {
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let provider_running_checks = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recovery_fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sessions = recording_agent_sessions(
                stop_calls.clone(),
                provider_running_checks.clone(),
                recovery_fails.clone(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                String::new(),
            );
            let fixture =
                runtime_effect_fixture_with_sessions(NodeCompletion::Auto, sessions, stop_calls)
                    .await;
            append_process_exit(&fixture, Some(1));
            recovery_fails.store(true, std::sync::atomic::Ordering::SeqCst);

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(error.to_string().contains("provider recovery failure"));
            assert_eq!(persisted_node_status(&fixture), NodeExecutionStatus::Failed);
            assert_eq!(provider_running_checks.lock().unwrap().len(), 1);
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &fixture.execution_id)
                    .unwrap();
            assert!(!records.iter().any(|record| {
                record.meta.node_execution_id == fixture.node_execution_id
                    && matches!(record.fact, NodeFact::ResumeRequested)
            }));
        }

        #[tokio::test]
        async fn test_複数sessionのprovider復旧部分失敗は先行nodeを元状態へ補償し再試行できる() {
            let sessions = Arc::new(PartialRecoveryWorkflowAgentSessions::new());
            let fixture = multi_resume_fixture(sessions.clone()).await;
            sessions.bind(
                fixture.store.clone(),
                &fixture.execution_id,
                fixture._directory.path().join("local-event-store.sqlite3"),
            );
            let (first_node_execution_id, first_session_id) = fixture.sessions[0].clone();
            let (second_node_execution_id, second_session_id) = fixture.sessions[1].clone();
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &first_node_execution_id,
                Some(7),
            );
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &second_node_execution_id,
                Some(0),
            );
            let first_before = persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &first_node_execution_id,
            );
            let second_before = persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &second_node_execution_id,
            );
            assert_eq!(first_before.status, NodeExecutionStatus::Failed);
            assert_eq!(second_before.status, NodeExecutionStatus::Paused);
            sessions.close_all();
            sessions.fail_on(&second_node_execution_id);

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(error
                .to_string()
                .contains("partial provider recovery failure"));
            let first_restored = persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &first_node_execution_id,
            );
            let second_restored = persisted_node_for(
                &fixture.store,
                &fixture.execution_id,
                &second_node_execution_id,
            );
            assert_eq!(first_restored.status, NodeExecutionStatus::Failed);
            assert_eq!(first_restored.failure, first_before.failure);
            assert!(first_restored.can_resume());
            assert_eq!(second_restored.status, NodeExecutionStatus::Paused);
            assert!(second_restored.can_resume());
            assert!(sessions.is_open(&first_session_id));
            assert!(!sessions.is_open(&second_session_id));
            assert_eq!(sessions.provider_launch_count(&first_session_id), 1);
            assert_eq!(sessions.provider_launch_count(&second_session_id), 0);
            assert_eq!(
                sessions.recovery_calls.lock().unwrap().as_slice(),
                &[
                    (first_node_execution_id.clone(), first_session_id.clone()),
                    (second_node_execution_id.clone(), second_session_id.clone()),
                ]
            );

            sessions.allow_recovery();
            fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap();

            assert_eq!(
                persisted_node_for(
                    &fixture.store,
                    &fixture.execution_id,
                    &first_node_execution_id,
                )
                .status,
                NodeExecutionStatus::Running
            );
            assert_eq!(
                persisted_node_for(
                    &fixture.store,
                    &fixture.execution_id,
                    &second_node_execution_id,
                )
                .status,
                NodeExecutionStatus::Running
            );
            assert_eq!(sessions.provider_launch_count(&first_session_id), 1);
            assert_eq!(sessions.provider_launch_count(&second_session_id), 1);
            assert_eq!(
                sessions.recovery_calls.lock().unwrap().as_slice(),
                &[
                    (first_node_execution_id.clone(), first_session_id.clone()),
                    (second_node_execution_id.clone(), second_session_id.clone()),
                    (first_node_execution_id, first_session_id),
                    (second_node_execution_id, second_session_id),
                ]
            );
        }

        #[tokio::test]
        async fn test_resume_provider復旧失敗の補償は対象外のpaused兄弟nodeを維持する() {
            let sessions = Arc::new(PartialRecoveryWorkflowAgentSessions::new());
            let fixture = multi_resume_fixture(sessions.clone()).await;
            sessions.bind(
                fixture.store.clone(),
                &fixture.execution_id,
                fixture._directory.path().join("local-event-store.sqlite3"),
            );
            let paused_sibling_ids = stop_with_resume_paused_siblings(
                fixture.app.handle(),
                &fixture.store,
                &fixture.host,
                &fixture.execution_id,
            )
            .await;
            let first_node_execution_id = fixture.sessions[0].0.clone();
            let second_node_execution_id = fixture.sessions[1].0.clone();
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &first_node_execution_id,
                Some(7),
            );
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &second_node_execution_id,
                Some(0),
            );
            sessions.close_all();
            sessions.fail_on(&second_node_execution_id);
            let broadcasts = record_workflow_execution_broadcasts(fixture.app.handle());

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(error
                .to_string()
                .contains("partial provider recovery failure"));
            assert_resume_paused_siblings_in_memory(
                &fixture.host,
                &fixture.execution_id,
                &paused_sibling_ids,
            )
            .await;
            assert_resume_paused_siblings_in_latest_broadcast(&broadcasts, &paused_sibling_ids);
        }

        #[tokio::test]
        async fn test_resume_control_plane_commit失敗の補償は対象外のpaused兄弟nodeを維持する() {
            let sessions = Arc::new(ResumeCommitFailureWorkflowAgentSessions::new(
                ResumeCommitFailureMode::Persistence,
            ));
            let fixture = runtime_effect_fixture_with_sessions(
                NodeCompletion::Auto,
                sessions.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )
            .await;
            sessions.bind(&fixture);
            let paused_sibling_ids = stop_with_resume_paused_siblings(
                fixture.app.handle(),
                &fixture.store,
                &fixture.host,
                &fixture.execution_id,
            )
            .await;
            append_process_exit(&fixture, Some(7));
            let broadcasts = record_workflow_execution_broadcasts(fixture.app.handle());

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
            assert_resume_paused_siblings_in_memory(
                &fixture.host,
                &fixture.execution_id,
                &paused_sibling_ids,
            )
            .await;
            assert_resume_paused_siblings_in_latest_broadcast(&broadcasts, &paused_sibling_ids);
        }

        #[tokio::test]
        async fn test_resume_instruction配送失敗の補償は対象外のpaused兄弟nodeを維持する() {
            let dispatch_fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sessions = recording_agent_sessions(
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                dispatch_fails.clone(),
                String::new(),
            );
            let fixture = runtime_effect_fixture_with_sessions(
                NodeCompletion::Auto,
                sessions,
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )
            .await;
            let paused_sibling_ids = stop_with_resume_paused_siblings(
                fixture.app.handle(),
                &fixture.store,
                &fixture.host,
                &fixture.execution_id,
            )
            .await;
            append_process_exit(&fixture, Some(7));
            dispatch_fails.store(true, std::sync::atomic::Ordering::SeqCst);
            let broadcasts = record_workflow_execution_broadcasts(fixture.app.handle());

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(error.to_string().contains("instruction dispatch failure"));
            assert_resume_paused_siblings_in_memory(
                &fixture.host,
                &fixture.execution_id,
                &paused_sibling_ids,
            )
            .await;
            assert_resume_paused_siblings_in_latest_broadcast(&broadcasts, &paused_sibling_ids);
        }

        #[tokio::test]
        async fn test_resume補償eventなしのearly_returnでも対象外のpaused兄弟nodeを維持する() {
            let sessions = Arc::new(PartialRecoveryWorkflowAgentSessions::new());
            let fixture = multi_resume_fixture(sessions.clone()).await;
            sessions.bind(
                fixture.store.clone(),
                &fixture.execution_id,
                fixture._directory.path().join("local-event-store.sqlite3"),
            );
            let paused_sibling_ids = stop_with_resume_paused_siblings(
                fixture.app.handle(),
                &fixture.store,
                &fixture.host,
                &fixture.execution_id,
            )
            .await;
            let first_node_execution_id = fixture.sessions[0].0.clone();
            let second_node_execution_id = fixture.sessions[1].0.clone();
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &first_node_execution_id,
                Some(7),
            );
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &second_node_execution_id,
                Some(0),
            );
            sessions.close_all();
            sessions.skip_persisted_resume();
            sessions.fail_on(&second_node_execution_id);
            let broadcasts = record_workflow_execution_broadcasts(fixture.app.handle());

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(error
                .to_string()
                .contains("partial provider recovery failure"));
            assert_resume_paused_siblings_in_memory(
                &fixture.host,
                &fixture.execution_id,
                &paused_sibling_ids,
            )
            .await;
            assert!(broadcasts.lock().unwrap().is_empty());
        }

        #[tokio::test]
        async fn test_複数sessionのprovider復旧部分失敗後の補償失敗は両errorを合成する() {
            let sessions = Arc::new(PartialRecoveryWorkflowAgentSessions::new());
            let fixture = multi_resume_fixture(sessions.clone()).await;
            sessions.bind(
                fixture.store.clone(),
                &fixture.execution_id,
                fixture._directory.path().join("local-event-store.sqlite3"),
            );
            let first_node_execution_id = fixture.sessions[0].0.clone();
            let second_node_execution_id = fixture.sessions[1].0.clone();
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &first_node_execution_id,
                Some(1),
            );
            append_process_exit_for_node(
                &fixture.store,
                &fixture.execution_id,
                &second_node_execution_id,
                Some(0),
            );
            sessions.close_all();
            sessions.fail_on(&second_node_execution_id);
            sessions.fail_compensation();

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                WorkflowRuntimeError::InvalidState(ref message)
                    if message.contains("partial provider recovery failure")
                        && message.contains("failed to restore unactivated resumed nodes")
                        && message.contains("node event storage is unavailable")
            ));
        }

        #[tokio::test]
        async fn test_resume_control_plane_commit失敗は元状態へ補償しproviderを再起動せず再試行できる(
        ) {
            for mode in [
                ResumeCommitFailureMode::StaleCandidate,
                ResumeCommitFailureMode::Persistence,
            ] {
                for exit_code in [Some(1), Some(0)] {
                    let sessions = Arc::new(ResumeCommitFailureWorkflowAgentSessions::new(mode));
                    let fixture = runtime_effect_fixture_with_sessions(
                        NodeCompletion::Auto,
                        sessions.clone(),
                        Arc::new(std::sync::Mutex::new(Vec::new())),
                    )
                    .await;
                    sessions.bind(&fixture);
                    append_process_exit(&fixture, exit_code);
                    let before = persisted_node(&fixture);

                    let result = fixture
                        .host
                        .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                        .await;
                    assert!(
                        result.is_err(),
                        "resume commit must fail for {mode:?} with exit code {exit_code:?}; recovery calls: {}",
                        sessions
                            .recovery_calls
                            .load(std::sync::atomic::Ordering::SeqCst)
                    );
                    let error = result.unwrap_err();

                    match mode {
                        ResumeCommitFailureMode::StaleCandidate => {
                            assert!(matches!(error, WorkflowRuntimeError::Conflict(_)));
                        }
                        ResumeCommitFailureMode::Persistence => {
                            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
                        }
                        ResumeCommitFailureMode::PersistenceWithCompensationFailure => {
                            unreachable!()
                        }
                    }
                    let restored = persisted_node(&fixture);
                    assert_eq!(restored.status, before.status);
                    assert_eq!(restored.failure, before.failure);
                    assert!(restored.can_resume());
                    let repository = LocalAgentSessionRepository::new(fixture.store.clone());
                    let restored_session = repository
                        .find(EFFECT_AGENT_SESSION_ID)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        restored_session.session().lifecycle(),
                        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Open
                    );
                    assert_eq!(
                        sessions
                            .provider_launches
                            .load(std::sync::atomic::Ordering::SeqCst),
                        1
                    );
                    sessions.clear_persistence_failure();

                    fixture
                        .host
                        .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                        .await
                        .unwrap();

                    assert_eq!(
                        persisted_node_status(&fixture),
                        NodeExecutionStatus::Running
                    );
                    assert_eq!(
                        sessions
                            .provider_launches
                            .load(std::sync::atomic::Ordering::SeqCst),
                        1
                    );
                    assert_eq!(
                        sessions
                            .recovery_calls
                            .load(std::sync::atomic::Ordering::SeqCst),
                        2
                    );
                }
            }
        }

        #[tokio::test]
        async fn test_resume_commit失敗後の補償失敗は元errorと補償errorをinvalid_stateへ合成する() {
            let sessions = Arc::new(ResumeCommitFailureWorkflowAgentSessions::new(
                ResumeCommitFailureMode::PersistenceWithCompensationFailure,
            ));
            let fixture = runtime_effect_fixture_with_sessions(
                NodeCompletion::Auto,
                sessions.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )
            .await;
            sessions.bind(&fixture);
            append_process_exit(&fixture, Some(1));

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                WorkflowRuntimeError::InvalidState(ref message)
                    if message.contains("failed to restore unactivated resumed nodes")
                        && message.matches("node fact append failed").count() == 2
            ));
            assert_eq!(
                sessions
                    .provider_launches
                    .load(std::sync::atomic::Ordering::SeqCst),
                1
            );
        }

        #[tokio::test]
        async fn test_provider動作中のpaused_sessionは再起動せずrunningへ戻す() {
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let provider_running_checks = Arc::new(std::sync::Mutex::new(Vec::new()));
            let sessions = recording_agent_sessions(
                stop_calls.clone(),
                provider_running_checks.clone(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                String::new(),
            );
            let fixture =
                runtime_effect_fixture_with_sessions(NodeCompletion::Auto, sessions, stop_calls)
                    .await;
            fixture
                .host
                .stop_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap();
            assert_eq!(
                fixture
                    .host
                    .get_state_by_execution_id(&fixture.execution_id)
                    .await
                    .unwrap()
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Paused
            );

            fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap();

            assert_eq!(
                fixture
                    .host
                    .get_state_by_execution_id(&fixture.execution_id)
                    .await
                    .unwrap()
                    .node_executions
                    .iter()
                    .find(|node| node.id == fixture.node_execution_id)
                    .unwrap()
                    .status,
                NodeExecutionStatus::Running
            );
            assert_eq!(
                provider_running_checks.lock().unwrap().as_slice(),
                &[(
                    fixture.node_execution_id.clone(),
                    EFFECT_AGENT_SESSION_ID.to_string(),
                )]
            );
            let records =
                workflow_fact_log::read_tree_records(&fixture.store, &fixture.execution_id)
                    .unwrap();
            assert!(records.iter().any(|record| {
                record.meta.node_execution_id == fixture.node_execution_id
                    && matches!(record.fact, NodeFact::ResumeRequested)
            }));
        }

        #[tokio::test]
        async fn test_resume後の指示配送失敗はnodeをresume前の状態へ戻す() {
            for (exit_code, expected_status) in [
                (Some(1), NodeExecutionStatus::Failed),
                (Some(0), NodeExecutionStatus::Paused),
            ] {
                let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
                let dispatch_fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let sessions = recording_agent_sessions(
                    stop_calls.clone(),
                    Arc::new(std::sync::Mutex::new(Vec::new())),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    dispatch_fails.clone(),
                    String::new(),
                );
                let fixture = runtime_effect_fixture_with_sessions(
                    NodeCompletion::Auto,
                    sessions,
                    stop_calls,
                )
                .await;
                append_process_exit(&fixture, exit_code);
                dispatch_fails.store(true, std::sync::atomic::Ordering::SeqCst);

                let error = fixture
                    .host
                    .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                    .await
                    .unwrap_err();

                assert!(error.to_string().contains("instruction dispatch failure"));
                assert_eq!(persisted_node_status(&fixture), expected_status);
                assert_eq!(
                    fixture
                        .host
                        .get_state_by_execution_id(&fixture.execution_id)
                        .await
                        .unwrap()
                        .node_executions
                        .iter()
                        .find(|node| node.id == fixture.node_execution_id)
                        .unwrap()
                        .status,
                    expected_status
                );
            }
        }

        #[tokio::test]
        async fn test_resume後の指示配送失敗後の補償失敗は元errorと補償errorをinvalid_stateへ合成する(
        ) {
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let dispatch_fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sessions = recording_agent_sessions(
                stop_calls.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                dispatch_fails.clone(),
                String::new(),
            );
            let fixture =
                runtime_effect_fixture_with_sessions(NodeCompletion::Auto, sessions, stop_calls)
                    .await;
            append_process_exit(&fixture, Some(1));
            rusqlite::Connection::open(fixture._directory.path().join("local-event-store.sqlite3"))
                .unwrap()
                .execute_batch(
                    "CREATE TRIGGER fail_resume_dispatch_compensation
                 BEFORE INSERT ON node_events
                 WHEN NEW.event_type = 'process_exited'
                 BEGIN
                   SELECT RAISE(ABORT, 'injected dispatch compensation failure');
                 END;",
                )
                .unwrap();
            dispatch_fails.store(true, std::sync::atomic::Ordering::SeqCst);

            let error = fixture
                .host
                .resume_workflow_execution(fixture.app.handle(), &fixture.execution_id)
                .await
                .unwrap_err();

            assert!(matches!(
                error,
                WorkflowRuntimeError::InvalidState(ref message)
                    if message.contains("intentional instruction dispatch failure")
                        && message.contains("failed to restore unactivated resumed nodes")
                        && message.contains("node fact append failed")
            ));
        }

        #[tokio::test]
        async fn test_stop受領後にpausedとなったnodeのresume指示配送失敗は正常終了事実で補償する() {
            const SESSION_ID: &str = "00000000-0000-4000-8000-000000000005";
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            app.manage(crate::infrastructure::platform::app_data_dir::TestDataDir(
                directory.path().to_path_buf(),
            ));
            let stop_calls = Arc::new(std::sync::Mutex::new(Vec::new()));
            let dispatch_fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sessions = recording_agent_sessions(
                stop_calls,
                Arc::new(std::sync::Mutex::new(Vec::new())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                dispatch_fails.clone(),
                String::new(),
            );
            let execution_store = Arc::new(ExecutionStore::new_in_memory_for_tests());
            let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                execution_store.clone(),
                sessions,
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        store.clone(),
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
            ));

            // Given: Session 起動由来の単独実行木で Stop 後の正常終了が Paused を導出している
            LocalAgentSessionRepository::new(store.clone())
                .create(
                    AgentSession::create(
                        SESSION_ID,
                        WorkspaceIdentity::new(EFFECT_WORKTREE_PATH),
                        EFFECT_WORKTREE_PATH,
                        ProviderKind::Codex,
                        AgentSessionTreeLocation::session_tree_root(SESSION_ID).unwrap(),
                    )
                    .unwrap(),
                    "create-standalone-resume-rollback-session",
                )
                .await
                .unwrap();
            host.register_started_execution_tree(app.handle(), SESSION_ID)
                .await
                .unwrap();
            let backend = workflow_fact_log::FactLogReadBackend::Live(store.clone());
            let folded = workflow_fact_log::fold_tree_from(&backend, SESSION_ID)
                .unwrap()
                .unwrap();
            let model = crate::domain::workflow::services::fact_replay::derive_read_model(&folded);
            execution_store
                .register_active_execution(WorkflowExecutionMetadata {
                    execution_id: model.id,
                    workflow_name: model.workflow_name,
                    status: model.status,
                    worktree_path: model.worktree_path,
                    current_node: model.current_node,
                    created_from: model.created_from,
                    started_at: model.started_at,
                    updated_at: model.updated_at,
                    completed_at: model.completed_at,
                    error_reason: model.error_reason,
                    interruption_reason: model.interruption_reason,
                    resume_from_node: model.resume_from_node,
                    total_token_usage: model.total_token_usage,
                })
                .await
                .unwrap();
            let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
            let gateway = Arc::new(TauriWorkflowRuntimeCommandGateway::new_with_driver(
                app.handle().clone(),
                host.clone(),
                repository,
                store.installation_id().to_string(),
            ));
            let control_plane = WorkflowControlPlaneUsecase::new(gateway);
            control_plane
                .record_provider_stop(
                    ProviderExecutionTreeStopCommand {
                        agent_session_id: SESSION_ID.to_string(),
                        tree_id: SESSION_ID.to_string(),
                        node_execution_id: SESSION_ID.to_string(),
                        binding_id: "binding-standalone-resume-rollback".to_string(),
                    },
                    Vec::new(),
                )
                .await
                .unwrap();
            append_process_exit_for_node(&store, SESSION_ID, SESSION_ID, Some(0));
            let before = persisted_node_for(&store, SESSION_ID, SESSION_ID);
            assert_eq!(before.status, NodeExecutionStatus::Paused);
            assert_eq!(
                before.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
            let records_before = workflow_fact_log::read_tree_records(&store, SESSION_ID)
                .unwrap()
                .len();
            dispatch_fails.store(true, std::sync::atomic::Ordering::SeqCst);

            // When: provider 復旧後の initial instruction 配送が失敗する
            let error = host
                .resume_workflow_execution(app.handle(), SESSION_ID)
                .await
                .unwrap_err();

            // Then: resume は失敗し、事実列の再 fold でも StopReceived を保った Paused に戻る
            assert!(error.to_string().contains("instruction dispatch failure"));
            let restored = persisted_node_for(&store, SESSION_ID, SESSION_ID);
            assert_eq!(restored.status, NodeExecutionStatus::Paused);
            assert_eq!(
                restored.completion_signals,
                crate::domain::workflow::NodeCompletionSignalState::StopReceived
            );
            assert!(restored.can_resume());
            let appended = workflow_fact_log::read_tree_records(&store, SESSION_ID)
                .unwrap()
                .into_iter()
                .skip(records_before)
                .collect::<Vec<_>>();
            assert!(appended.iter().any(|record| matches!(
                &record.fact,
                NodeFact::ProcessExited(fact) if fact.exit_code == Some(0)
                    && fact.failure_reason.is_none()
                    && fact.failure_kind.is_none()
            )));
            assert!(appended
                .iter()
                .any(|record| matches!(record.fact, NodeFact::SessionAttached(_))));

            dispatch_fails.store(false, std::sync::atomic::Ordering::SeqCst);
            host.resume_workflow_execution(app.handle(), SESSION_ID)
                .await
                .unwrap();
            assert_eq!(
                persisted_node_for(&store, SESSION_ID, SESSION_ID).status,
                NodeExecutionStatus::Running
            );
        }

        #[tokio::test]
        async fn test_abort_agent_session停止失敗でも成功とabortedを維持する() {
            // Given
            let fixture = runtime_effect_fixture(NodeCompletion::Auto, true).await;

            // When
            let result = fixture
                .host
                .abort_workflow_execution(fixture.app.handle(), &fixture.execution_id, None)
                .await;

            // Then
            assert!(result.is_ok(), "unexpected abort error: {result:?}");
            assert_eq!(
                persisted_node_status(&fixture),
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
                    dispatch_fails: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
            .unwrap();
            let before = workflow_fact_log::read_tree_records(&store, session_id)
                .unwrap()
                .len();
            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            app.manage(store.clone());
            let host = WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(FailingWorkflowAgentSessions),
                Arc::new(
                    crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                        store.clone(),
                    ),
                ),
                Arc::new(MissingRepoWorktreeInventory),
            );

            host.reconcile_startup(app.handle()).await.unwrap();

            let snapshot = host.get_state_by_execution_id(session_id).await.unwrap();
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
                    .unwrap()
                    .len(),
                before
            );
            assert!(host.execution_store.list_active().await.unwrap().is_empty());
        }

        fn append_started_session_tree(
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
                            output: None,
                            children: vec![ChildEntry::reference("impl")],
                        }),
                        artifact: None,
                        input: Vec::new(),
                        completion: crate::domain::workflow::NodeCompletion::Auto,
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
                        completion: crate::domain::workflow::NodeCompletion::Auto,
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
                    parent: None,
                    root: Some(TreeRootFact {
                        workspace_identity: worktree_path.to_string(),
                        worktree_path: worktree_path.to_string(),
                        created_from: ExecutionOrigin::DesktopUi,
                        request: String::new(),
                        definition,
                        launched_as: ExecutionTreeLaunch::Workflow,
                    }),
                }),
                timestamp_ms,
            )
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
                    parent: Some(ExecutionParentRef::sequence_child(tree_id)),
                    root: None,
                }),
                timestamp_ms + 1,
            )
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
            append_started_session_tree(&store, CORRUPT_TREE_ID, "/repo/corrupt", 1);
            let corrupt_child_meta = NodeFactMeta {
                tree_id: CORRUPT_TREE_ID.to_string(),
                node_execution_id: format!("{CORRUPT_TREE_ID}-session"),
                parent_id: Some(CORRUPT_TREE_ID.to_string()),
                node_name: "impl".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
            };
            workflow_fact_log::append_single_fact(
                &store,
                &corrupt_child_meta,
                &NodeFact::IsolatedWorktreeCreated(
                    crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact {
                        repository_root: "/repo".to_string(),
                        worktree_path: format!(
                            "/repo-worktrees/.releash-isolated/{CORRUPT_TREE_ID}-session-a1"
                        ),
                        branch: format!("releash/isolated/{CORRUPT_TREE_ID}-session-a1"),
                    },
                ),
                3,
            )
            .unwrap();
            store
                .append_node_event_blocking(
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
                .unwrap();
            append_started_session_tree(&store, VALID_TREE_ID, "/repo/valid", 5);
            let corrupt_count =
                workflow_fact_log::read_tree_records(&store, CORRUPT_TREE_ID).unwrap_err();
            assert!(corrupt_count.contains("decode"));
            let valid_count = workflow_fact_log::read_tree_records(&store, VALID_TREE_ID)
                .unwrap()
                .len();

            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            let worktree_ledger = Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            );
            app.manage(store.clone());
            let host = WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(FailingWorkflowAgentSessions),
                worktree_ledger,
                Arc::new(MissingRepoWorktreeInventory),
            );

            let error = host.reconcile_startup(app.handle()).await.unwrap_err();

            assert!(matches!(error, WorkflowRuntimeError::SessionStore(_)));
            assert!(host
                .execution_store
                .list_active()
                .await
                .unwrap()
                .iter()
                .any(|execution| execution.execution_id == VALID_TREE_ID));
            assert_eq!(
                workflow_fact_log::read_tree_records(&store, VALID_TREE_ID)
                    .unwrap()
                    .len(),
                valid_count
            );
            assert!(!workflow_fact_log::read_tree_records(&store, VALID_TREE_ID)
                .unwrap()
                .iter()
                .any(|record| matches!(record.fact, NodeFact::IsolatedWorktreeLost)));
            let corrupt_lost_count = store
                .submit_indexed_query_blocking(move |connection| {
                    crate::adaptor::gateway::local_event_store::node_events::read_tree(
                        connection,
                        CORRUPT_TREE_ID,
                    )
                    .map(|rows| {
                        rows.into_iter()
                            .filter(|row| row.event_type == "isolated_worktree_lost")
                            .count()
                    })
                    .map_err(|_| crate::domain::local_event::LocalEventQueryError::InvalidRequest)
                })
                .unwrap();
            assert_eq!(corrupt_lost_count, 0);
        }

        #[tokio::test]
        async fn test_startup_reconciliation_provider固有permissionのstarted_factをsession_store_errorにする(
        ) {
            const TREE_ID: &str = "00000000-0000-4000-8000-000000000004";
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            let mut fact =
                SessionExecutionTreeRootFacts::new(TREE_ID, "/repo", "/repo", ProviderKind::Claude)
                    .unwrap()
                    .started;
            let NodeFact::Started(StartedFact {
                root: Some(root), ..
            }) = &mut fact
            else {
                unreachable!();
            };
            let NodeKind::Session(spec) = &mut root.definition.nodes[0].kind else {
                unreachable!();
            };
            spec.permission = Some(SessionPermission::Auto);
            let legacy_detail = fact.encode_detail().unwrap().replace(
                r#""permission":"auto""#,
                r#""permission":"bypassPermissions""#,
            );
            store
                .append_node_event_blocking(
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
                .unwrap();

            let history_error = workflow_fact_log::read_tree_records(&store, TREE_ID).unwrap_err();
            assert!(history_error.contains("node fact decode failed"));
            assert!(history_error.contains("bypassPermissions"));

            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            let worktree_ledger = Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            );
            app.manage(store);
            let host = WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(UnusedWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(FailingWorkflowAgentSessions),
                worktree_ledger,
                Arc::new(MissingRepoWorktreeInventory),
            );

            let error = host.reconcile_startup(app.handle()).await.unwrap_err();

            assert!(
                matches!(error, WorkflowRuntimeError::SessionStore(message) if
                message.contains("bypassPermissions"))
            );
        }

        #[tokio::test]
        async fn test_隔離worktree喪失後のresumeは事実を追記せず拒否する() {
            use crate::domain::workflow::value_objects::IsolatedWorktreeCreatedFact;

            const TREE_ID: &str = "00000000-0000-4000-8000-000000000003";
            let directory = tempfile::tempdir().unwrap();
            let store = LocalEventStore::open(LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ))
            .unwrap();
            append_started_session_tree(&store, TREE_ID, "/repo", 1);
            let child_meta = NodeFactMeta {
                tree_id: TREE_ID.to_string(),
                node_execution_id: format!("{TREE_ID}-session"),
                parent_id: Some(TREE_ID.to_string()),
                node_name: "impl".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
            };
            workflow_fact_log::append_single_fact(
                &store,
                &child_meta,
                &NodeFact::IsolatedWorktreeCreated(IsolatedWorktreeCreatedFact {
                    repository_root: "/repo".to_string(),
                    worktree_path: format!(
                        "/repo-worktrees/.releash-isolated/{TREE_ID}-session-a1"
                    ),
                    branch: format!("releash/isolated/{TREE_ID}-session-a1"),
                }),
                3,
            )
            .unwrap();

            let app = tauri::test::mock_builder()
                .build(tauri::test::mock_context(tauri::test::noop_assets()))
                .unwrap();
            let worktree_ledger = Arc::new(
                crate::adaptor::gateway::workflow::NodeEventIsolatedWorktreeLedgerRepository::new(
                    store.clone(),
                ),
            );
            app.manage(store.clone());
            let host = WorkflowRuntimeHost::with_execution_store(
                Arc::new(UnusedWorkflowResolver),
                Arc::new(AcceptingWorktreeResolver),
                Arc::new(ExecutionStore::new_in_memory_for_tests()),
                Arc::new(FailingWorkflowAgentSessions),
                worktree_ledger,
                Arc::new(MissingRepoWorktreeInventory),
            );
            host.reconcile_startup(app.handle()).await.unwrap();
            let before = workflow_fact_log::read_tree_records(&store, TREE_ID)
                .unwrap()
                .len();

            let error = host
                .resume_workflow_execution(app.handle(), TREE_ID)
                .await
                .unwrap_err();

            assert!(
                matches!(
                    &error,
                    WorkflowRuntimeError::InvalidState(reason)
                        if reason
                            == &format!(
                                "isolated worktree is missing: /repo-worktrees/.releash-isolated/{TREE_ID}-session-a1"
                            )
                ),
                "unexpected error: {error}"
            );
            let records = workflow_fact_log::read_tree_records(&store, TREE_ID).unwrap();
            assert_eq!(records.len(), before);
            assert!(!records
                .iter()
                .any(|record| matches!(record.fact, NodeFact::ResumeRequested)));
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
