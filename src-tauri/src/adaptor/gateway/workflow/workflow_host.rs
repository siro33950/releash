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
mod restart_recovery;
pub(crate) mod resume_projection;
pub(crate) mod runtime_commit;
pub(crate) mod runtime_session;

use activation::{run_runtime_activation, RuntimeActivationGate};
use command_preparation::{command_execution_input_is_current, CommandExecutionInput};

use crate::adaptor::gateway::workflow::event_log_writer as workflow_event_log_writer;
use crate::adaptor::gateway::workflow::execution_store::{
    ExecutionOrigin, ExecutionStatus, ExecutionStore, ExecutionStoreError,
    WorkflowExecutionMetadata,
};
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::adaptor::gateway::workflow::node_session_boundary::{
    ProviderWorkflowAgentSessionPort, WorkflowAgentSessionPort, WorkflowSessionLaunchConfig,
};
use crate::adaptor::gateway::workflow::secret_source;
use crate::domain::local_event::CommitOperationKind;
use crate::domain::workflow::entities::workflow_execution::{
    AppliedAdvance, LeafStart, RuntimeNodeExecutionStatus as NodeExecutionStatus, TransitionOutcome,
};
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::WorkflowFacetContents;
use crate::domain::workflow::{
    ContractValidationResult, FailureClassification, FailureDisposition, NodeExecutionFailureKind,
    SchemaDef as DomainSchemaDef,
};
use crate::domain::workflow::{NodeKindName, WorkflowDefinition};
use crate::infrastructure::process::command_runner::{
    self as workflow_command_runner, ActiveCommandHandle, CommandRunOutput, CommandRunnerError,
};
use crate::usecase::agent_session::{
    AgentSessionInitialInstructionUsecase, AgentSessionInterruptUsecase, AgentSessionLaunchUsecase,
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
use execution_registry::{find_any_by_worktree, find_by_worktree};
use execution_state::{DomainWorkflowExecution, SessionWorkflowRef};
use node_settings::WorkflowDefaults;
use output_limit as workflow_output_limit;
use prompt_rendering as workflow_prompt;
use resume_projection as workflow_resume_projection;
use runtime_commit::{
    self as workflow_runtime_commit, AbortOutcome, AbortTargetLookup, RequiredEventCommit,
};
use runtime_session as workflow_runtime_session;

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
    /// session_id（親・ステップ・並列子） → SessionWorkflowRef のマッピング
    session_workflow_refs: Arc<Mutex<HashMap<String, SessionWorkflowRef>>>,
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
}

enum RequiredEventCommitFailure {
    /// No event fact became visible; rollbackable resources may be discarded.
    BeforeDurableAppend(WorkflowRuntimeError),
    /// Legacy test authority committed the event but failed its separate file projection.
    #[cfg(test)]
    AfterDurableAppend(WorkflowRuntimeError),
}

impl RequiredEventCommitFailure {
    fn into_workflow_error(self) -> WorkflowRuntimeError {
        match self {
            Self::BeforeDurableAppend(error) => error,
            #[cfg(test)]
            Self::AfterDurableAppend(error) => error,
        }
    }
}

struct ControlPlaneCommitCandidate<'a> {
    operation_kind: CommitOperationKind,
    execution_id: &'a str,
    snapshot_before: DomainWorkflowExecution,
    candidate: DomainWorkflowExecution,
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

fn command_env(input: &CommandExecutionInput) -> Vec<(String, String)> {
    let mut env = vec![
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
    ];
    if let Some(session_id) = input.session_id.as_ref() {
        env.push(("RELEASH_SESSION_ID".to_string(), session_id.clone()));
    }
    env
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
    pub(crate) async fn load_control_plane_execution(
        &self,
        execution_id: &str,
    ) -> Option<DomainWorkflowExecution> {
        self.executions.lock().await.get(execution_id).cloned()
    }

    pub(crate) async fn commit_workflow_control_plane<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        commit: crate::usecase::workflow::control_plane::WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        self.commit_control_plane_candidate(
            app,
            ControlPlaneCommitCandidate {
                operation_kind: commit.operation_kind,
                execution_id: &commit.execution_id,
                snapshot_before: commit.before,
                candidate: commit.after,
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
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
        agent_session_launch: Arc<AgentSessionLaunchUsecase>,
        agent_session_initial_instruction: Arc<AgentSessionInitialInstructionUsecase>,
        agent_session_interrupt: Arc<AgentSessionInterruptUsecase>,
        provider_availability: Arc<dyn crate::domain::agent_session::ProviderAvailabilityReader>,
    ) -> Self {
        Self::with_execution_store(
            workflow_resolver,
            worktree_resolver,
            Arc::new(ExecutionStore::new_canonical(
                data_dir,
                repository,
                installation_id,
            )),
            Arc::new(ProviderWorkflowAgentSessionPort::new(
                agent_session_launch,
                agent_session_initial_instruction,
                agent_session_interrupt,
                provider_availability,
            )),
        )
    }

    fn with_execution_store(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        execution_store: Arc<ExecutionStore>,
        workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
    ) -> Self {
        Self {
            executions: Arc::new(Mutex::new(HashMap::new())),
            session_workflow_refs: Arc::new(Mutex::new(HashMap::new())),
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
        if self.execution_store.local_event_authority().await.is_some() {
            // The worktree-owner CAS is included in the same required SQLite
            // batch as ExecutionStarted/NodeStarted. The in-memory store is a
            // post-commit list projection and cannot pre-admit the command.
            return Ok(execution_id);
        }
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

    async fn durable_workflow_event_log(
        &self,
        _data_dir: &std::path::Path,
    ) -> Result<WorkflowEventLog, WorkflowRuntimeError> {
        let Some((repository, installation_id)) =
            self.execution_store.local_event_authority().await
        else {
            #[cfg(test)]
            return Ok(WorkflowEventLog::new(_data_dir));
            #[cfg(not(test))]
            return Err(WorkflowRuntimeError::SessionStore(
                "workflow SQLite event authority is not configured".to_string(),
            ));
        };
        Ok(WorkflowEventLog::with_authority(
            repository,
            installation_id,
        ))
    }

    /// 起動時 recovery: 前回プロセスで実行中だった Agent Node を同一 Attempt の Paused へ
    /// 遷移させる。受信済み completion signal は保持し、Stop 受信済みの Submit 待ちと
    /// WaitingApproval には Pause を重ねない。
    ///
    /// `<data_dir>/workflow_execution_logs/<execution_id>.ndjson` 末尾に
    /// 必要な `NodePaused` を append した後、event log 全体を replay し、active projection と
    /// live aggregateを同じ durable stateから復元する。
    ///
    /// 本メソッドは `set_execution_store_data_dir` 直後（in-memory `executions` map が空の状態）に
    /// 1 度だけ呼ばれる前提。canonical read / projection / commit のいずれかを確認できない場合は
    /// startup recovery 全体を失敗させる。呼び出し側は通常 activation を開始せず、同じ durable
    /// inventory から再試行する。
    pub async fn recover_orphan_executions<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<(), WorkflowRuntimeError> {
        let _recovery_guard = self.startup_recovery_lock.lock().await;
        let orphans = self
            .execution_store
            .try_list_non_terminal_metadata()
            .await
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?
            .into_iter()
            .collect::<Vec<_>>();
        if orphans.is_empty() {
            return Ok(());
        }
        let sqlite_authority = self.execution_store.local_event_authority().await.is_some();
        let data_dir = match self.execution_store.configured_data_dir().await {
            Some(data_dir) => data_dir,
            None => crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
                .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?,
        };
        for metadata in orphans {
            let execution_id = metadata.execution_id.clone();
            let log = self.durable_workflow_event_log(&data_dir).await?;
            let events = log.read_log_durable(&execution_id).await.map_err(|error| {
                WorkflowRuntimeError::SessionStore(format!(
                    "orphan recovery event read failed for {}: {error}",
                    execution_id
                ))
            })?;
            let projected_before =
                match crate::domain::workflow::services::event_replay::project_workflow_execution(
                    &execution_id,
                    &events,
                ) {
                    Ok(Some(projected)) => projected,
                    Ok(None) => {
                        if sqlite_authority {
                            let mutations = self
                            .execution_store
                            .prepare_atomic_stale_reservation_deletion_mutations(&metadata)
                            .await
                            .map_err(|error| {
                                WorkflowRuntimeError::SessionStore(format!(
                                    "orphan recovery stale reservation deletion preparation failed for {}: {error}",
                                    execution_id
                                ))
                            })?;
                            log.commit_projection_durable(&execution_id, mutations)
                            .await
                            .map_err(|error| {
                                WorkflowRuntimeError::SessionStore(format!(
                                    "orphan recovery stale reservation deletion failed for {}: {error}",
                                    execution_id
                                ))
                            })?;
                        } else {
                            self.execution_store
                                .cancel_reservation(&execution_id)
                                .await
                                .map_err(|error| {
                                    WorkflowRuntimeError::SessionStore(format!(
                                    "orphan recovery reservation cleanup failed for {}: {error}",
                                    execution_id
                                ))
                                })?;
                        }
                        continue;
                    }
                    Err(error) => {
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "orphan recovery event projection failed for {}: {error}",
                            execution_id
                        )));
                    }
                };

            // A terminal/interrupted event may already be durable while the previous process died
            // before metadata projection. Reconcile it without appending a contradictory orphan
            // interruption. Only an event-log-active execution is a real orphan.
            let projected = if projected_before.status.is_active() {
                if projected_before.worktree_path != metadata.worktree_path {
                    return Err(WorkflowRuntimeError::InvalidState(format!(
                        "orphan recovery worktree mismatch for {}",
                        execution_id
                    )));
                }
                let checkpoint =
                    workflow_resume_projection::project_restart_checkpoint(&execution_id, &events)
                        .map_err(|error| {
                            WorkflowRuntimeError::InvalidState(format!(
                                "restart reconciliation checkpoint failed for {}: {error}",
                                execution_id
                            ))
                        })?;
                let mut live_execution = restart_recovery::hydrate_restart_execution(&checkpoint)?;
                let pause_timestamp = current_timestamp();
                let pause_targets = live_execution
                    .node_executions
                    .iter()
                    .filter(|node| {
                        matches!(node.kind, NodeKindName::Session | NodeKindName::Command)
                            && node.status == NodeExecutionStatus::Running
                            && node.completion_signals
                                != crate::domain::workflow::NodeCompletionSignalState::StopReceived
                    })
                    .map(|node| node.id.clone())
                    .collect::<Vec<_>>();
                let mut reconciliation_events = Vec::with_capacity(pause_targets.len());
                for node_execution_id in pause_targets {
                    if live_execution.pause_node_execution(&node_execution_id, pause_timestamp)
                        == crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                    {
                        reconciliation_events.push(WorkflowEvent::NodePaused {
                            execution_id: execution_id.clone(),
                            node_execution_id,
                            timestamp: pause_timestamp,
                        });
                    }
                }
                let mut candidate_events = events.clone();
                candidate_events.extend(reconciliation_events.iter().cloned());
                let reconciled = match crate::domain::workflow::services::event_replay::project_workflow_execution(
                    &execution_id,
                    &candidate_events,
                ) {
                    Ok(Some(projected)) if projected.status.is_active() => projected,
                    Ok(Some(projected)) => {
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "orphan recovery projection for {} has unexpected status {}",
                            execution_id,
                            projected.status.as_str()
                        )));
                    }
                    Ok(None) => {
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "restart reconciliation projection for {} is empty",
                            execution_id
                        )));
                    }
                    Err(error) => {
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "restart reconciliation projection failed for {}: {error}",
                            execution_id
                        )));
                    }
                };
                let mutations = match self
                    .execution_store
                    .prepare_atomic_event_reconciliation_metadata_mutations(&metadata, &reconciled)
                    .await
                {
                    Ok(mutations) => mutations,
                    Err(error) => {
                        return Err(WorkflowRuntimeError::SessionStore(format!(
                            "orphan recovery projection preparation failed for {}: {error}",
                            execution_id
                        )));
                    }
                };
                if reconciliation_events.is_empty() {
                    if sqlite_authority && !mutations.is_empty() {
                        log.commit_projection_durable(&execution_id, mutations)
                            .await
                            .map_err(|error| {
                                WorkflowRuntimeError::SessionStore(format!(
                                    "restart reconciliation metadata commit failed for {}: {error}",
                                    execution_id
                                ))
                            })?;
                    }
                } else {
                    self.write_log_required_batch_with_mutations(
                        app,
                        &reconciliation_events,
                        mutations,
                    )
                    .map_err(|error| {
                        WorkflowRuntimeError::SessionStore(format!(
                            "restart reconciliation atomic pause commit failed for {}: {error}",
                            execution_id
                        ))
                    })?;
                }
                {
                    let mut executions = self.executions.lock().await;
                    executions.insert(execution_id.clone(), live_execution.clone());
                }
                {
                    let mut refs = self.session_workflow_refs.lock().await;
                    for node in &live_execution.node_executions {
                        if let Some(session_id) = &node.session_id {
                            refs.insert(
                                session_id.clone(),
                                SessionWorkflowRef {
                                    execution_id: execution_id.clone(),
                                },
                            );
                        }
                    }
                }
                reconciled
            } else {
                if sqlite_authority {
                    let mutations = self
                        .execution_store
                        .prepare_atomic_event_reconciliation_metadata_mutations(
                            &metadata,
                            &projected_before,
                        )
                        .await
                        .map_err(|error| {
                            WorkflowRuntimeError::SessionStore(format!(
                                "orphan recovery durable event reconciliation preparation failed for {}: {error}",
                                execution_id
                            ))
                        })?;
                    if !mutations.is_empty() {
                        log.commit_projection_durable(&execution_id, mutations)
                            .await
                            .map_err(|error| {
                                WorkflowRuntimeError::SessionStore(format!(
                                    "orphan recovery durable event reconciliation failed for {}: {error}",
                                    execution_id
                                ))
                            })?;
                    }
                }
                projected_before
            };
            self.execution_store
                .reconcile_orphan_from_projection(metadata, &projected)
                .await
                .map_err(|error| {
                    WorkflowRuntimeError::SessionStore(format!(
                        "orphan recovery projection reconciliation failed for {}: {error}",
                        execution_id
                    ))
                })?;
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
        let sqlite_authority = self.execution_store.local_event_authority().await.is_some();
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
            if sqlite_authority {
                // Before the required batch commits there is no canonical
                // reservation to undo. Runtime maps are discarded by the
                // caller; a compensating ExecutionStore write would restore a
                // second admission authority.
                return;
            }
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
        let start_projection_mutations = match self
            .execution_store
            .prepare_atomic_initial_snapshot_mutations(&snapshot)
            .await
        {
            Ok(mutations) => mutations,
            Err(error) => {
                self.executions.lock().await.remove(&execution_id);
                self.release_execution_facet_contents(&execution_id).await;
                rollback_reservation(format!(
                    "initial workflow projection preparation failed: {error}"
                ))
                .await;
                return Err(WorkflowRuntimeError::SessionStore(error.to_string()));
            }
        };
        if let Err(e) = self.write_log_required_batch_with_mutations_as(
            app,
            CommitOperationKind::UserMutation,
            &required_start_events,
            start_projection_mutations,
        ) {
            let mut execs = self.executions.lock().await;
            execs.remove(&execution_id);
            drop(execs);
            self.release_execution_facet_contents(&execution_id).await;
            rollback_reservation(format!("initial workflow event batch failed: {e}")).await;
            return Err(WorkflowRuntimeError::SessionStore(format!(
                "write initial workflow event batch failed: {e}"
            )));
        }

        if sqlite_authority {
            if let Err(error) = self
                .execution_store
                .rebuild_active_projection_from_authority()
                .await
            {
                log::warn!(
                    "workflow {execution_id}: failed to refresh derived execution list after commit: {error}"
                );
            }
        }

        // [04] post-commit: broadcast。ExecutionStarted は append 済みのため command は既に受理。
        // session_workflow_refs への登録は node session 起動時（start_node_session /
        // start_fanout_children）で行う。
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;

        // [04] post-commit: ExecutionStarted append 済みのため start primitive は既に受理。
        //    初回 runtime 起動失敗は Failed 状態遷移として観測し、
        //    start primitive は Ok(execution_id) を返す（spec [04]『command 受理境界』Rule）。
        if let crate::domain::workflow::entities::workflow_execution::ExecutionAdvanceDecision::StartLeaves(leaves) =
            applied.decision
        {
            if let Err(e) = self.start_leaves(app, &worktree_path, leaves).await {
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
                    operation_kind: CommitOperationKind::UserMutation,
                    execution_id,
                    snapshot_before,
                    candidate,
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
            operation_kind,
            execution_id,
            snapshot_before,
            candidate,
            events,
            provider_events,
        } = commit;
        let snapshot = RuntimeCommitSnapshot::from_execution(&candidate)?;
        let transaction = PreparedWorkflowTransaction::capture_applied(
            snapshot_before,
            candidate,
            events.to_vec(),
            vec![WorkflowRuntimeEffect::BroadcastState],
        )
        .map_err(|error| {
            WorkflowRuntimeError::InvalidState(format!(
                "invalid control-plane transaction preparation: {error:?}"
            ))
        })?;
        let projection_mutations = self
            .execution_store
            .prepare_atomic_existing_snapshot_mutations(&snapshot)
            .await
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        let mut executions = self.executions.lock().await;
        let current = executions
            .get_mut(execution_id)
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        let durable = transaction
            .persist(current, |events| {
                if provider_events.is_empty() {
                    workflow_event_log_writer::append_required_events_with_mutations_for_app_as(
                        app,
                        operation_kind,
                        events,
                        projection_mutations,
                    )
                } else {
                    workflow_event_log_writer::append_provider_stop_for_app(
                        app,
                        execution_id,
                        events,
                        provider_events,
                        projection_mutations,
                    )
                }
            })
            .map_err(|error| match error {
                WorkflowTransactionCommitError::StaleCandidate => WorkflowRuntimeError::Conflict(
                    format!("execution '{execution_id}' changed before control-plane commit"),
                ),
                WorkflowTransactionCommitError::Persistence(error) => {
                    WorkflowRuntimeError::SessionStore(error)
                }
            })?;
        debug_assert_eq!(
            durable.into_effects(),
            vec![WorkflowRuntimeEffect::BroadcastState]
        );
        drop(executions);
        if let Err(error) = self.sync_state_after_required_event_commit(&snapshot).await {
            log::warn!(
                "workflow {execution_id}: derived execution projection refresh failed after control-plane commit: {error}"
            );
        }
        Ok(snapshot)
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

    async fn cleanup_session_workflow_refs_by_execution_id(&self, execution_id: &str) {
        let mut map = self.session_workflow_refs.lock().await;
        map.retain(|_, r| r.execution_id != execution_id);
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
        worktree_path: &str,
        leaves: Vec<LeafStart>,
    ) -> Result<(), WorkflowRuntimeError> {
        if leaves.is_empty() {
            return Ok(());
        }
        let (execution_id, workflow, attempts_by_id) = {
            let executions = self.executions.lock().await;
            let (execution_id, exec) =
                find_by_worktree(&executions, worktree_path).ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
                })?;
            let attempts_by_id: HashMap<String, u32> = exec
                .node_executions
                .iter()
                .map(|execution| (execution.id.clone(), execution.attempt))
                .collect();
            (execution_id.clone(), exec.workflow.clone(), attempts_by_id)
        };
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
                    let command = node.command().ok_or_else(|| {
                        WorkflowRuntimeError::InvalidState(format!(
                            "node '{}' is not a command",
                            leaf.node_name
                        ))
                    })?;
                    let rendered =
                        workflow_prompt::render_parameter_references(command, &leaf.bindings);
                    command_inputs.push(CommandExecutionInput {
                        execution_id: execution_id.clone(),
                        node_execution_id: leaf.node_execution_id.clone(),
                        node_name: leaf.node_name.clone(),
                        attempt: attempts_by_id
                            .get(&leaf.node_execution_id)
                            .copied()
                            .unwrap_or(1),
                        worktree_path: worktree_path.to_string(),
                        raw_command: Some(rendered),
                        contract: node.artifact.clone(),
                        schemas: workflow.schemas.clone(),
                        session_id: None,
                    });
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
            match self
                .workflow_agent_sessions
                .prepare_workflow_agent_session(
                    worktree_path,
                    launch_config,
                    &execution_id,
                    &node_execution_id,
                    &initial_instruction,
                )
                .await
            {
                Ok(session) => {
                    self.session_workflow_refs.lock().await.insert(
                        session.id.clone(),
                        SessionWorkflowRef {
                            execution_id: execution_id.clone(),
                        },
                    );
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
                        operation_kind: CommitOperationKind::Workflow,
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
        for (node_execution_id, session_id) in &session_setups {
            if let Err(error) = run_runtime_activation(
                &activation_gate,
                &execution_id,
                "session",
                self.workflow_agent_sessions
                    .activate_workflow_agent_session(session_id, node_execution_id),
            )
            .await
            {
                self.settle_runtime_failure_for_node(
                    app,
                    worktree_path,
                    &execution_id,
                    node_execution_id,
                    &error,
                )
                .await?;
                log::warn!(
                    "workflow {execution_id}: NodeExecution '{node_execution_id}' failed to activate: {error}"
                );
            }
        }
        drop(activation_guard);
        drop(activation_gate);
        for input in command_inputs {
            let node_execution_id = input.node_execution_id.clone();
            if let Err(error) = self.spawn_command_execution(app, input).await {
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
        {
            let mut refs = self.session_workflow_refs.lock().await;
            for (_, session_id) in session_setups {
                refs.remove(session_id);
            }
        }
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
        if !self.commit_command_prepared(app, &input).await? {
            return Ok(());
        }
        let raw_command = input.raw_command.take().ok_or_else(|| {
            WorkflowRuntimeError::InvalidState(format!(
                "raw command for node execution '{}' is unavailable",
                input.node_execution_id
            ))
        })?;
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
                command_env(&input),
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
                operation_kind: CommitOperationKind::Workflow,
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
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError> {
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
            operation_kind,
            execution_id,
            snapshot_for_commit,
            snapshot_before,
            execution_store_snapshot_before,
            required_events,
            append_error_context,
        } = commit;

        let projection_mutations = self
            .execution_store
            .prepare_atomic_existing_snapshot_mutations(snapshot_for_commit)
            .await
            .map_err(|error| {
                RequiredEventCommitFailure::BeforeDurableAppend(WorkflowRuntimeError::SessionStore(
                    format!("{append_error_context}: {error}"),
                ))
            })?;

        // Reacquire the execution mutex and keep it through the synchronous append. Every runtime
        // mutation uses this mutex, so a newer stop/completion cannot overtake this event commit.
        // If another mutation already won, this stale commit emits no fact and performs no
        // rollback. On append failure, restore the driver snapshot before releasing the mutex so
        // no concurrent mutation can observe or overwrite the failed pre-commit state.
        let append_result = {
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
            transaction
                .persist(current, |events| {
                    self.write_log_required_batch_with_mutations_as(
                        app,
                        operation_kind,
                        events,
                        projection_mutations,
                    )
                })
                .map(|durable| {
                    debug_assert_eq!(
                        durable.into_effects(),
                        vec![WorkflowRuntimeEffect::BroadcastState]
                    );
                })
                .map_err(|error| match error {
                    WorkflowTransactionCommitError::StaleCandidate => {
                        "workflow transaction candidate became stale".to_string()
                    }
                    WorkflowTransactionCommitError::Persistence(error) => error,
                })
        };
        if let Err(e) = append_result {
            let _ = workflow_runtime_commit::restore_execution_store_active_snapshot(
                &self.execution_store,
                execution_store_snapshot_before,
            )
            .await;
            return Err(RequiredEventCommitFailure::BeforeDurableAppend(
                WorkflowRuntimeError::SessionStore(format!("{append_error_context}: {e}")),
            ));
        }

        if let Err(e) = self
            .sync_state_after_required_event_commit(snapshot_for_commit)
            .await
        {
            #[cfg(test)]
            if self.execution_store.local_event_authority().await.is_none() {
                return Err(RequiredEventCommitFailure::AfterDurableAppend(
                    WorkflowRuntimeError::SessionStore(format!(
                        "required event projection failed: {e}"
                    )),
                ));
            }
            // Required events are the SQLite commit authority.  The
            // ExecutionStore/JSON view is a rebuildable post-commit
            // projection; its failure must not reverse an accepted command.
            log::warn!(
                "workflow {execution_id}: derived execution projection refresh failed after canonical commit: {e}"
            );
        }

        Ok(())
    }

    /// [04] post-commit phase: cleanup_refs + broadcast. Every required
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
        if is_finished {
            self.cleanup_session_workflow_refs_by_execution_id(&execution_id)
                .await;
        }
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
        let mut leaves = Vec::new();
        if let Some(treatment) = treatment {
            events.extend(treatment.events);
            leaves = treatment.leaves;
        }
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    operation_kind: CommitOperationKind::Workflow,
                    execution_id,
                    snapshot_before,
                    candidate,
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
                if let Err(e) = Box::pin(self.start_leaves(app, worktree_path, leaves)).await {
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

    /// 複数の必須 event を 1 つの atomic commit point として一括追記する。
    ///
    /// [04] spec『event 列と domain state の整合』Rule: 同一 command 受理サイクル内で
    /// 複数 required event を発行する場合は本 helper を使い、partial commit を構造的に
    /// 排除する。
    fn write_log_required_batch<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        events: &[WorkflowEvent],
    ) -> Result<(), String> {
        self.write_log_required_batch_as(
            app,
            crate::domain::local_event::CommitOperationKind::Workflow,
            events,
        )
    }

    fn write_log_required_batch_as<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        events: &[WorkflowEvent],
    ) -> Result<(), String> {
        workflow_event_log_writer::append_required_events_for_app_as(app, operation_kind, events)
    }

    fn write_log_required_batch_with_mutations<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        events: &[WorkflowEvent],
        mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        self.write_log_required_batch_with_mutations_as(
            app,
            crate::domain::local_event::CommitOperationKind::Workflow,
            events,
            mutations,
        )
    }

    fn write_log_required_batch_with_mutations_as<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        operation_kind: crate::domain::local_event::CommitOperationKind,
        events: &[WorkflowEvent],
        mutations: Vec<crate::domain::local_event::LocalStateMutation>,
    ) -> Result<(), String> {
        workflow_event_log_writer::append_required_events_with_mutations_for_app_as(
            app,
            operation_kind,
            events,
            mutations,
        )
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
            contract: None,
            schemas: BTreeMap::new(),
            session_id: None,
        };

        let env = command_env(&input);

        assert!(env.contains(&(
            "RELEASH_WORKTREE_PATH".to_string(),
            "/repo/worktree".to_string()
        )));
    }
}
