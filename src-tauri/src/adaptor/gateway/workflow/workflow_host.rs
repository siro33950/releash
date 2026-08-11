//! Workflow execution host gateway.
//!
//! The domain aggregate owns lifecycle transitions and decisions, while
//! `usecase::workflow::runtime_driver` owns their application procedure and
//! transaction ordering. This gateway retains the aggregates, delegates
//! decisions to them, and connects event storage, agent sessions, processes,
//! and notifications.

use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;

mod activation;
pub(crate) mod approval_runtime;
mod command_preparation;
pub(crate) mod execution_registry;
pub(crate) mod execution_state;
pub(crate) mod fanout_runtime;
mod lifecycle_commands;
pub(crate) mod node_settings;
pub(crate) mod output_limit;
pub(crate) mod prompt_rendering;
mod resume_orchestration;
pub(crate) mod resume_projection;
pub(crate) mod runtime_commit;
pub(crate) mod runtime_session;
pub(crate) mod turn_completion;
mod turn_completion_recovery;

use activation::{run_runtime_activation, RuntimeActivationGate};
use command_preparation::{command_execution_input_is_current, CommandExecutionInput};

use crate::adaptor::gateway::workflow::event_log_writer as workflow_event_log_writer;
use crate::adaptor::gateway::workflow::execution_store::{
    ExecutionOrigin, ExecutionStatus, ExecutionStore, ExecutionStoreError,
    WorkflowExecutionMetadata,
};
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
#[cfg(test)]
use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
use crate::adaptor::gateway::workflow::node_session_boundary::{
    NodeSessionDeps, ProviderWorkflowAgentSessionPort, RealNodeSessionDeps,
    WorkflowAgentSessionPort,
};
use crate::adaptor::gateway::workflow::secret_source;
use crate::domain::agent_session::PermissionMode;
use crate::domain::local_event::CommitOperationKind;
use crate::domain::workflow::entities::workflow_execution::CanonicalNodeFact;
use crate::domain::workflow::entities::workflow_execution::{
    NodeStallObservation, RuntimeNodeExecution as NodeExecution,
    RuntimeNodeExecutionStatus as NodeExecutionStatus,
};
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::transition as workflow_transition;
#[cfg(test)]
use crate::domain::workflow::NodeDefinition;
#[cfg(test)]
use crate::domain::workflow::NodeHistoryEntry;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::WorkflowFacetContents;
#[cfg(test)]
use crate::domain::workflow::{
    CommandSpec, FacetRefs, FanoutSpec, NodeKind, SessionGate, SessionSpec,
};
use crate::domain::workflow::{
    ContractValidationResult, FailureClassification, FailureDisposition, NodeExecutionFailureKind,
    SchemaDef as DomainSchemaDef,
};
use crate::domain::workflow::{NodeKindName, WorkflowDefinition};
use crate::domain::workflow::{RuntimeArtifact, RuntimeExecutionState, TokenUsage};
use crate::infrastructure::process::command_runner::{
    self as workflow_command_runner, ActiveCommandHandle, CommandRunOutput, CommandRunnerError,
};
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
#[cfg(test)]
use crate::usecase::agent_session::session::OpenTabRegistry;
use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::status::current_timestamp;
use crate::usecase::agent_session::{
    ProviderAgentInitialInstructionUsecase, ProviderAgentSessionLaunchUsecase,
};
#[cfg(test)]
use crate::usecase::workflow::output_submission as workflow_output_submission;
use crate::usecase::workflow::runtime_driver::{
    self as workflow_runtime_driver, NodeOutcome, PreparedWorkflowTransaction,
    WorkflowRuntimeEffect, WorkflowTransactionCommitError,
};
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_events as workflow_runtime_events;
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolver, WorkflowDefinitionResolver,
};
#[cfg(test)]
use crate::usecase::workflow::runtime_resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;
use crate::usecase::workflow::runtime_start_guard as workflow_runtime_start_guard;
use approval_runtime as workflow_approval_runtime;
use execution_registry::{find_any_by_worktree, find_by_worktree, find_by_worktree_mut};
#[cfg(test)]
use execution_state::NextNodeDecision;
use execution_state::{DomainWorkflowExecution, FanoutChildRuntimeState, SessionWorkflowRef};
#[cfg(test)]
use execution_state::{FanoutChildRuntime, FanoutRuntimeState};
use fanout_runtime as workflow_fanout_runtime;
use node_settings::WorkflowDefaults;
use output_limit as workflow_output_limit;
use prompt_rendering as workflow_prompt;
use resume_projection as workflow_resume_projection;
#[cfg(test)]
use runtime_commit::CommandMutationRollback;
use runtime_commit::{
    self as workflow_runtime_commit, AbortOutcome, AbortTargetLookup, RequiredEventCommit,
};
use runtime_session as workflow_runtime_session;

#[cfg(test)]
#[derive(Default)]
struct TestWorkflowAgentSessionPort;

#[cfg(test)]
#[async_trait::async_trait]
impl WorkflowAgentSessionPort for TestWorkflowAgentSessionPort {
    fn is_provider_available(
        &self,
        _provider: crate::domain::provider_lifecycle::ProviderKind,
    ) -> bool {
        true
    }

    async fn prepare_workflow_agent_session(
        &self,
        _worktree_path: &str,
        provider: crate::domain::provider_lifecycle::ProviderKind,
        _workflow_execution_id: &str,
        node_execution_id: &str,
        _initial_instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        Ok(NodeSessionInfo {
            id: format!(
                "provider-agent-session-{}-{node_execution_id}",
                match provider {
                    crate::domain::provider_lifecycle::ProviderKind::Claude => "claude",
                    crate::domain::provider_lifecycle::ProviderKind::Codex => "codex",
                }
            ),
        })
    }

    async fn activate_workflow_agent_session(
        &self,
        _node_session_id: &str,
        _node_execution_id: &str,
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

    async fn rollback_workflow_agent_session(
        &self,
        _node_session_id: &str,
        _node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }
}

fn fanout_child_failure_kind(
    exit_code: i64,
    failure_signal: Option<workflow_transition::SessionFailureSignal>,
) -> NodeExecutionFailureKind {
    workflow_transition::classify_session_error(exit_code, failure_signal)
}

#[cfg(test)]
struct TestWorkflowDefinitionResolver;

#[cfg(test)]
#[async_trait::async_trait]
impl WorkflowDefinitionResolver for TestWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<WorkflowDefinition, WorkflowDefinitionResolverError> {
        let workflow_name = workflow_name.to_string();
        tokio::task::spawn_blocking(move || {
            let dir = crate::adaptor::gateway::workflow::storage::workflows_dir();
            let facets_base = crate::adaptor::gateway::workflow::facet::facets_base_dir();
            crate::adaptor::gateway::workflow::runtime_resolver::resolve_workflow_by_name(
                &dir,
                &facets_base,
                &workflow_name,
            )
        })
        .await
        .map_err(|e| {
            WorkflowDefinitionResolverError::Infrastructure(format!("task join error: {e}"))
        })?
    }
}

#[cfg(test)]
struct PassthroughManagedWorktreeResolver;

#[cfg(test)]
#[async_trait::async_trait]
impl ManagedWorktreeResolver for PassthroughManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}

#[cfg(test)]
type AbortAfterLookupGate =
    Arc<Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>>;

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
    /// Confirmed children of an interrupted fanout, consumed exactly once by its resumed parent.
    fanout_resume_checkpoints: Arc<Mutex<HashMap<String, FanoutResumeCheckpoint>>>,
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
    #[cfg(test)]
    fail_next_required_event_append: Arc<AtomicBool>,
    #[cfg(test)]
    abort_after_lookup_gate: AbortAfterLookupGate,
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

struct FanoutChildCompletionCommit {
    all_completed: bool,
    outcome: Option<NodeOutcome>,
    snapshot_before: DomainWorkflowExecution,
    progress_events: Vec<WorkflowEvent>,
    required_progress_events: bool,
    failure_telemetry: Option<FailureClassification>,
}

struct FanoutChildFailureCommit {
    completion: FanoutChildCompletionCommit,
    interrupted_session_ids: Vec<String>,
    interrupted_command_ids: Vec<String>,
}

struct FanoutChildFailureInput {
    child_node_execution_id: String,
    child_failure_reason: String,
    failure_kind: NodeExecutionFailureKind,
    failure_disposition: FailureDisposition,
    retry_count: Option<u32>,
    timestamp: f64,
    record_child_token_usage: bool,
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
struct FanoutResumeChild {
    node_name: String,
    item_index: Option<usize>,
    child_index: usize,
    reusable: workflow_fanout_runtime::ReusableFanoutChild,
}

#[derive(Clone)]
struct FanoutResumeCheckpoint {
    parent_node_name: String,
    children: Vec<FanoutResumeChild>,
}

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

fn is_still_current_execution(
    exec: &DomainWorkflowExecution,
    node_name: &str,
    attempt: u32,
) -> bool {
    if !exec.is_active() {
        return false;
    }
    let current_node = &exec.workflow.nodes[exec.current_node_index];
    if current_node.name != node_name {
        return false;
    }
    exec.node_execution_counts
        .get(node_name)
        .copied()
        .unwrap_or(1)
        == attempt
}

fn commit_snapshot_is_current(
    exec: &DomainWorkflowExecution,
    snapshot: &RuntimeCommitSnapshot,
) -> bool {
    exec.id == snapshot.execution_id
        && exec.updated_at == snapshot.updated_at
        && exec.state() == &snapshot.state
        && exec.current_node_index == snapshot.current_node_index
        && exec.current_session_id == snapshot.current_session_id
        && exec.node_executions == snapshot.node_executions
}

#[cfg(test)]
fn resolve_active_node_execution_index(
    exec: &DomainWorkflowExecution,
    node_name: &str,
    node_execution_id: Option<&str>,
) -> Result<usize, WorkflowRuntimeError> {
    let candidates = exec
        .node_executions
        .iter()
        .enumerate()
        .filter(|(_, execution)| execution.node_name == node_name && execution.status.is_active())
        .collect::<Vec<_>>();
    if let Some(node_execution_id) = node_execution_id {
        return candidates
            .into_iter()
            .find(|(_, execution)| execution.id == node_execution_id)
            .map(|(index, _)| index)
            .ok_or_else(|| {
                WorkflowRuntimeError::InvalidState(format!(
                    "active node execution '{node_execution_id}' for node '{node_name}' was not found"
                ))
            });
    }
    match candidates.as_slice() {
        [(index, _)] => Ok(*index),
        [] => Err(WorkflowRuntimeError::InvalidState(format!(
            "node '{node_name}' has no active execution"
        ))),
        candidates => {
            let candidate_ids = candidates
                .iter()
                .map(|(_, execution)| execution.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(WorkflowRuntimeError::InvalidState(format!(
                "node '{node_name}' has {} active executions; node_execution_id is required; candidates: [{candidate_ids}]",
                candidates.len()
            )))
        }
    }
}

#[cfg(test)]
fn resolve_fanout_approval_target_node_execution_id(
    exec: &DomainWorkflowExecution,
    node_name: &str,
    node_execution_id: Option<&str>,
) -> Result<Option<String>, WorkflowRuntimeError> {
    let active_candidates = exec
        .node_executions
        .iter()
        .filter(|candidate| {
            candidate.node_name == node_name
                && candidate.fanout_parent.is_some()
                && candidate.status.is_active()
        })
        .collect::<Vec<_>>();

    if let Some(requested_id) = node_execution_id {
        return Ok(active_candidates
            .into_iter()
            .find(|candidate| {
                candidate.id == requested_id
                    && candidate.status == NodeExecutionStatus::WaitingApproval
            })
            .map(|candidate| candidate.id.clone()));
    }

    if active_candidates.len() > 1 {
        let candidate_ids = active_candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(WorkflowRuntimeError::InvalidState(format!(
            "node '{node_name}' has {} active fanout executions; node_execution_id is required; candidates: [{candidate_ids}]",
            active_candidates.len(),
        )));
    }

    Ok(active_candidates
        .into_iter()
        .find(|candidate| candidate.status == NodeExecutionStatus::WaitingApproval)
        .map(|candidate| candidate.id.clone()))
}

fn complete_fanout_parent_after_all_children(
    exec: &mut DomainWorkflowExecution,
    snapshot_before: DomainWorkflowExecution,
    mut progress_events: Vec<WorkflowEvent>,
    required_progress_events: bool,
    failure_telemetry: Option<FailureClassification>,
) -> Result<FanoutChildCompletionCommit, WorkflowRuntimeError> {
    let Some(fanout_runtime) = exec.fanout_runtime.as_ref() else {
        return Err(WorkflowRuntimeError::InvalidState(
            "fanout parent completion requires an active fanout runtime".to_string(),
        ));
    };
    let parent_node_name = fanout_runtime.parent_node_name.clone();
    let parent_node_execution_id = fanout_runtime.parent_node_execution_id.clone();
    let parent_attempt = exec
        .node_execution_counts
        .get(&parent_node_name)
        .copied()
        .unwrap_or(1);
    let completed_at = current_timestamp();
    let completion_plan = workflow_fanout_runtime::plan_fanout_parent_completion(
        &parent_node_name,
        parent_attempt,
        &fanout_runtime.children,
        completed_at,
    );
    let parent_artifact = completion_plan
        .parent_artifact
        .artifact
        .clone()
        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
    progress_events.push(WorkflowEvent::ArtifactProduced {
        execution_id: exec.id.clone(),
        node_execution_id: parent_node_execution_id.clone(),
        node_name: parent_node_name.clone(),
        contract: None,
        value: parent_artifact.clone(),
        request_id: None,
        submitted_at: None,
        timestamp: completed_at,
    });

    let _ = exec.finalize_fanout_parent(
        &parent_node_execution_id,
        completion_plan.parent_artifact,
        completion_plan.history_entry,
        completed_at,
    );

    let outcome =
        workflow_runtime_driver::apply_advance(exec, new_node_execution_id(), completed_at)?;

    Ok(FanoutChildCompletionCommit {
        all_completed: true,
        outcome: Some(outcome),
        snapshot_before,
        progress_events,
        required_progress_events,
        failure_telemetry,
    })
}

fn finalize_child_terminal_state(
    exec: &mut DomainWorkflowExecution,
    snapshot_before: DomainWorkflowExecution,
    progress_events: Vec<WorkflowEvent>,
    required_progress_events: bool,
    failure_telemetry: Option<FailureClassification>,
) -> Result<FanoutChildCompletionCommit, WorkflowRuntimeError> {
    let Some(fanout_runtime) = exec.fanout_runtime.as_ref() else {
        return Err(WorkflowRuntimeError::InvalidState(
            "fanout child terminal state requires an active fanout runtime".to_string(),
        ));
    };
    let all_done = fanout_runtime.children.iter().all(|c| {
        matches!(
            c.state,
            FanoutChildRuntimeState::Completed | FanoutChildRuntimeState::Failed
        )
    });

    if !all_done {
        exec.touch(current_timestamp());
        let snapshot = RuntimeCommitSnapshot::from_execution(exec)?;
        return Ok(FanoutChildCompletionCommit {
            all_completed: false,
            outcome: Some(NodeOutcome::Persist(snapshot)),
            snapshot_before,
            progress_events,
            required_progress_events,
            failure_telemetry,
        });
    }

    complete_fanout_parent_after_all_children(
        exec,
        snapshot_before,
        progress_events,
        required_progress_events,
        failure_telemetry,
    )
}

fn record_fanout_child_successful_completion(
    execution: &mut DomainWorkflowExecution,
    child_node_name: &str,
) {
    execution.record_successful_node_completion(child_node_name, current_timestamp());
}

fn finalize_fanout_child_failure_state(
    exec: &mut DomainWorkflowExecution,
    snapshot_before: DomainWorkflowExecution,
    input: FanoutChildFailureInput,
) -> Result<FanoutChildFailureCommit, WorkflowRuntimeError> {
    let execution_id = exec.id.clone();
    let failure_kind = input.failure_kind;
    let failure_disposition = input.failure_disposition;
    let retry_count = input.retry_count;
    let timestamp = input.timestamp;
    let (child_name, child_attempt, child_session_id, child_token_usage) = {
        let Some(fanout_runtime) = exec.fanout_runtime.as_ref() else {
            return Err(WorkflowRuntimeError::InvalidState(
                "fanout child failure requires an active fanout runtime".to_string(),
            ));
        };
        let Some(failed_child) = fanout_runtime
            .children
            .iter()
            .find(|child| child.node_execution_id == input.child_node_execution_id)
        else {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "fanout child failure references unknown child '{}'",
                input.child_node_execution_id
            )));
        };
        let child_name = failed_child.node_name.clone();
        let child_attempt = failed_child.attempt;
        let child_session_id = failed_child.session_id.clone();
        let child_token_usage = input
            .record_child_token_usage
            .then(|| failed_child.token_usage.clone());
        (
            child_name,
            child_attempt,
            child_session_id,
            child_token_usage,
        )
    };

    let _ = exec.fail_fanout_child_execution(
        &input.child_node_execution_id,
        input.child_failure_reason.clone(),
        failure_kind,
        failure_disposition,
        timestamp,
    );
    if let Some(child_token_usage) = child_token_usage {
        let _ = exec.record_node_token_usage(
            &input.child_node_execution_id,
            child_token_usage,
            timestamp,
        );
    }
    if !child_session_id.is_empty() {
        exec.clear_stalls_for_session(&child_session_id, timestamp);
    }
    let progress_events = vec![WorkflowEvent::NodeFailed {
        execution_id,
        node_execution_id: input.child_node_execution_id,
        node_name: child_name,
        attempt: child_attempt,
        reason: input.child_failure_reason,
        failure_kind,
        retry_count,
        timestamp,
    }];

    Ok(FanoutChildFailureCommit {
        completion: FanoutChildCompletionCommit {
            all_completed: false,
            outcome: Some(NodeOutcome::Persist(RuntimeCommitSnapshot::from_execution(
                exec,
            )?)),
            snapshot_before,
            progress_events,
            required_progress_events: true,
            failure_telemetry: Some(FailureClassification::with_disposition(
                failure_kind,
                failure_disposition,
            )),
        },
        interrupted_session_ids: Vec::new(),
        interrupted_command_ids: Vec::new(),
    })
}

fn current_node_for_stall_observation(
    exec: &DomainWorkflowExecution,
    session_id: &str,
) -> Option<(String, String, u32)> {
    if let Some(fanout_runtime) = exec.fanout_runtime.as_ref() {
        if let Some(child) = fanout_runtime.children.iter().find(|child| {
            child.session_id == session_id && child.state == FanoutChildRuntimeState::Running
        }) {
            return Some((
                child.node_execution_id.clone(),
                child.node_name.clone(),
                child.attempt,
            ));
        }
    }
    if exec.current_session_id.as_deref() != Some(session_id) {
        return None;
    }
    let node_name = exec
        .workflow
        .nodes
        .get(exec.current_node_index)?
        .name
        .clone();
    let attempt = exec
        .node_execution_counts
        .get(&node_name)
        .copied()
        .unwrap_or(1);
    let node_execution_id = exec
        .node_executions
        .iter()
        .rev()
        .find(|node| {
            node.node_name == node_name && node.attempt == attempt && node.fanout_parent.is_none()
        })?
        .id
        .clone();
    Some((node_execution_id, node_name, attempt))
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
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowRuntimeError> {
        self.finish_control_plane_commit(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            snapshot,
            outcome,
        )
        .await
    }

    pub(crate) async fn finish_retried_fanout_control_plane_commit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.finish_control_plane_commit(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            snapshot,
            None,
        )
        .await?;
        if let Err(error) = self
            .start_retried_fanout_child(
                app,
                session_store,
                agent_runtime,
                worktree_path,
                node_execution_id,
            )
            .await
        {
            if let Err(settle_error) = self
                .settle_runtime_failure_for_node(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    &snapshot.execution_id,
                    node_execution_id,
                    &error,
                )
                .await
            {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "{error}; NodeFailed settlement failed: {settle_error}"
                )));
            }
            return Ok(());
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn new(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    ) -> Self {
        Self::with_execution_store(
            workflow_resolver,
            worktree_resolver,
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            Arc::new(TestWorkflowAgentSessionPort),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_canonical(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        data_dir: Option<std::path::PathBuf>,
        repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
        installation_id: String,
        provider_agent_session_launch: Arc<ProviderAgentSessionLaunchUsecase>,
        provider_agent_initial_instruction: Arc<ProviderAgentInitialInstructionUsecase>,
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
                provider_agent_session_launch,
                provider_agent_initial_instruction,
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
            fanout_resume_checkpoints: Arc::new(Mutex::new(HashMap::new())),
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
            #[cfg(test)]
            fail_next_required_event_append: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            abort_after_lookup_gate: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::new(
            Arc::new(TestWorkflowDefinitionResolver),
            Arc::new(PassthroughManagedWorktreeResolver),
        )
    }

    #[cfg(test)]
    fn new_for_test_with_workflow_agent_sessions(
        workflow_agent_sessions: Arc<dyn WorkflowAgentSessionPort>,
    ) -> Self {
        Self::with_execution_store(
            Arc::new(TestWorkflowDefinitionResolver),
            Arc::new(PassthroughManagedWorktreeResolver),
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            workflow_agent_sessions,
        )
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

    #[cfg(test)]
    pub(crate) async fn seed_active_execution_for_test(
        &self,
        execution_id: String,
        workflow: WorkflowDefinition,
        state: RuntimeExecutionState,
        worktree_path: String,
        created_from: ExecutionOrigin,
    ) {
        assert!(
            matches!(
                state,
                RuntimeExecutionState::Running | RuntimeExecutionState::WaitingApproval
            ),
            "seed_active_execution_for_test only accepts active states"
        );
        let current_node = workflow.nodes[0].name.clone();
        let current_node_kind = workflow.nodes[0].kind_name();
        let node_execution_status = if matches!(state, RuntimeExecutionState::WaitingApproval) {
            NodeExecutionStatus::WaitingApproval
        } else {
            NodeExecutionStatus::Running
        };
        let execution_status = if matches!(state, RuntimeExecutionState::WaitingApproval) {
            ExecutionStatus::WaitingApproval
        } else {
            ExecutionStatus::Running
        };
        let now = 1000.0;
        let node_execution_id = uuid::Uuid::new_v4().to_string();
        let domain_execution = crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: execution_id.clone(),
            workflow: workflow.clone(),
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(state.clone()),
            current_node_index: 0,
            node_execution_counts: HashMap::from([(current_node.clone(), 1)]),
            loop_guard_reset_baselines: Default::default(),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: crate::domain::agent_session::PermissionMode::EDIT.to_string(),
            },
            worktree_path: worktree_path.clone(),
            created_from,
            error_reason: None,
            started_at: now,
            updated_at: now,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: node_execution_id.clone(),
                execution_id: execution_id.clone(),
                node_name: current_node.clone(),
                kind: current_node_kind,
                attempt: 1,
                status: node_execution_status,
                session_id: None,
                display_command: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                completion_signals: Default::default(),
                started_at: now,
                completed_at: None,
            }],
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        };
        self.execution_store
            .register_active_execution(WorkflowExecutionMetadata {
                execution_id: execution_id.clone(),
                workflow_name: workflow.name.clone(),
                status: execution_status,
                worktree_path: worktree_path.clone(),
                current_node: Some(current_node.clone()),
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
            .unwrap();
        if let Some(data_dir) = self.execution_store.data_dir_for_test().await {
            let mut events = vec![
                WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.clone(),
                    created_from,
                    request: String::new(),
                    permission_mode: PermissionMode::EDIT.to_string(),
                    definition: workflow.clone(),
                    timestamp: now,
                },
                WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: current_node.clone(),
                    kind: current_node_kind,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: now,
                },
            ];
            if matches!(state, RuntimeExecutionState::WaitingApproval) {
                events.push(WorkflowEvent::ApprovalRequested {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: current_node.clone(),
                    timestamp: now,
                });
            }
            if let Some((repository, installation_id)) =
                self.execution_store.local_event_authority().await
            {
                let initial_mutations = self
                    .execution_store
                    .prepare_atomic_initial_snapshot_mutations(
                        &RuntimeCommitSnapshot::from_execution(&domain_execution).unwrap(),
                    )
                    .await
                    .unwrap();
                WorkflowEventLog::with_authority(repository, installation_id)
                    .append_batch_durable_with_mutations_blocking_as(
                        CommitOperationKind::Workflow,
                        &events,
                        initial_mutations,
                    )
                    .unwrap();
            } else {
                WorkflowEventLog::new(&data_dir)
                    .append_batch(&events)
                    .unwrap();
            }
        }
        self.executions
            .lock()
            .await
            .insert(execution_id, domain_execution);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_required_event_append_for_test(&self) {
        self.fail_next_required_event_append
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) async fn pause_abort_after_lookup_for_test(
        &self,
        lookup_completed: Arc<tokio::sync::Notify>,
        continue_precommit: Arc<tokio::sync::Notify>,
    ) {
        *self.abort_after_lookup_gate.lock().await = Some((lookup_completed, continue_precommit));
    }

    #[cfg(test)]
    async fn wait_abort_after_lookup_for_test(&self) {
        let gate = self.abort_after_lookup_gate.lock().await.take();
        if let Some((lookup_completed, continue_precommit)) = gate {
            lookup_completed.notify_one();
            continue_precommit.notified().await;
        }
    }

    #[cfg(test)]
    pub(crate) async fn contains_execution_for_test(&self, execution_id: &str) -> bool {
        self.executions.lock().await.contains_key(execution_id)
    }

    #[cfg(test)]
    pub(crate) async fn executions_len_for_test(&self) -> usize {
        self.executions.lock().await.len()
    }

    /// テスト専用: 指定 execution の `current_node_index` を移動させて stale 状態を作る。
    #[cfg(test)]
    pub(crate) async fn force_current_node_index_for_test(&self, execution_id: &str, index: usize) {
        if let Some(exec) = self.executions.lock().await.get_mut(execution_id) {
            exec.current_node_index = index;
        }
    }

    /// Execution Store の参照（テスト専用）。production 経路では下記 facade メソッドを使用する。
    /// 公開 API は `list_active_executions` / `list_completed_executions` / `execution_id_for_worktree` /
    /// `resolve_worktree_by_execution` / `set_execution_store_data_dir` に集約する。
    #[cfg(test)]
    pub fn execution_store(&self) -> &Arc<ExecutionStore> {
        &self.execution_store
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
                current_node: workflow.nodes.first().map(|n| n.name.clone()),
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
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        let WorkflowExecutionInsert {
            execution_id,
            workflow,
            worktree_path,
            request,
            created_from,
            workflow_defaults,
            now,
        } = input;
        let request_text = request.clone().unwrap_or_default();
        let mut artifacts = HashMap::new();
        artifacts.insert(
            crate::domain::workflow::services::reference::REQUEST_ARTIFACT.to_string(),
            workflow_prompt::request_node_artifact(&request_text, now),
        );
        let mut execution = crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: execution_id.clone(),
            workflow: workflow.clone(),
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(RuntimeExecutionState::Running),
            current_node_index: 0,
            node_execution_counts: HashMap::new(),
            loop_guard_reset_baselines: Default::default(),
            node_history: Vec::new(),
            workflow_defaults,
            created_from,
            error_reason: None,
            started_at: now,
            updated_at: now,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            artifacts,
            node_executions: Vec::new(),
            request,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
            worktree_path: worktree_path.clone(),
        };

        let node_name = workflow.nodes[0].name.clone();
        let mut execs = self.executions.lock().await;
        DomainWorkflowExecution::validate_start(
            &workflow,
            find_any_by_worktree(&execs, &worktree_path),
        )?;
        execution.start_node_execution(
            node_name,
            workflow.nodes[0].kind_name(),
            1,
            None,
            new_node_execution_id(),
            now,
        );
        execs.insert(execution_id.clone(), execution);
        RuntimeCommitSnapshot::from_execution(execs.get(&execution_id).unwrap())
    }

    #[cfg(test)]
    async fn start_workflow_common_core_for_test(
        &self,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        now: f64,
    ) -> Result<String, WorkflowRuntimeError> {
        workflow_runtime_start_guard::validate_workflow_shape(&workflow)?;
        let execution_id = self
            .reserve_workflow_execution(
                &workflow,
                &worktree_path,
                request.clone(),
                created_from,
                now,
            )
            .await?;
        self.insert_workflow_execution(WorkflowExecutionInsert {
            execution_id: execution_id.clone(),
            workflow,
            worktree_path,
            request,
            created_from,
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: crate::domain::agent_session::PermissionMode::EDIT.to_string(),
            },
            now,
        })
        .await?;
        Ok(execution_id)
    }

    /// worktree_path から active execution_id を解決する。Execution Store の secondary index を参照する。
    #[cfg(test)]
    pub async fn execution_id_for_worktree(&self, worktree_path: &str) -> Option<String> {
        self.execution_store
            .resolve_execution_by_worktree(worktree_path)
            .await
    }

    /// execution_id から worktree_path を解決する test seam。SQLite authority 導入後は
    /// filesystem へ fallback せず authority projection から返す。
    /// Tauri command 経路で execution_id 主語の操作を内部 worktree_path に解決する際に使用する。
    #[cfg(test)]
    pub async fn resolve_worktree_by_execution(&self, execution_id: &str) -> Option<String> {
        self.execution_store
            .resolve_worktree_by_execution(execution_id)
            .await
    }

    /// テスト専用 facade: active な execution 一覧を取得する。
    /// production の read-only 経路は workflow QueryService を使う。
    #[cfg(test)]
    pub async fn list_active_executions(
        &self,
    ) -> Vec<crate::domain::workflow::WorkflowExecutionSummary> {
        self.execution_store
            .list_executions(crate::domain::workflow::ExecutionListFilter {
                status: Some(crate::domain::workflow::ExecutionStatusFilter::Active),
                worktree_path: None,
            })
            .await
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

    /// テスト専用 facade: terminal な execution 一覧を取得する。
    #[cfg(test)]
    pub async fn list_completed_executions(
        &self,
    ) -> Vec<crate::domain::workflow::WorkflowExecutionSummary> {
        self.execution_store
            .list_executions(crate::domain::workflow::ExecutionListFilter {
                status: Some(crate::domain::workflow::ExecutionStatusFilter::Terminal),
                worktree_path: None,
            })
            .await
    }

    /// テスト専用 facade: 単一 execution の summary を取得する。
    /// active map → terminal metadata file の順で lookup する。
    #[cfg(test)]
    pub async fn get_execution(
        &self,
        execution_id: &str,
    ) -> Option<crate::domain::workflow::WorkflowExecutionSummary> {
        self.execution_store.get_execution(execution_id).await
    }

    /// Execution Store の永続化ディレクトリを設定する（アプリ起動時の setup から呼ぶ）。
    #[cfg(test)]
    pub async fn set_execution_store_data_dir(&self, dir: std::path::PathBuf) {
        self.execution_store.set_data_dir(dir).await;
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
    #[cfg(test)]
    pub async fn recover_orphan_executions<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<(), WorkflowRuntimeError> {
        self.recover_orphan_executions_excluding(app, &std::collections::BTreeSet::new())
            .await
    }

    pub async fn recover_orphan_executions_excluding<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        unresolved_turn_completions: &std::collections::BTreeSet<String>,
    ) -> Result<(), WorkflowRuntimeError> {
        let _recovery_guard = self.startup_recovery_lock.lock().await;
        let orphans = self
            .execution_store
            .try_list_non_terminal_metadata()
            .await
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?
            .into_iter()
            .filter(|metadata| {
                !unresolved_turn_completions.contains(metadata.execution_id.as_str())
            })
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
                let checkpoint = workflow_resume_projection::project_turn_completion_checkpoint(
                    &execution_id,
                    &events,
                )
                .map_err(|error| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "restart reconciliation checkpoint failed for {}: {error}",
                        execution_id
                    ))
                })?;
                let mut live_execution =
                    turn_completion_recovery::hydrate_restart_execution(&checkpoint)?;
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
    #[allow(clippy::too_many_arguments)]
    async fn start_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        permission_mode: PermissionMode,
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

        // parent ChatSession 機構撤去後は session を driver が作らない。
        // workflow_defaults は StartExecution の permission_mode 引数を workflow 全体の継承
        // デフォルトとして capture する（schema 境界 [02]: 各 node は NodeDefinition.model
        // 必須で個別解決される）。
        let _ = data_dir; // unused after parent session removal
        let workflow_defaults = WorkflowDefaults {
            backend_id: None,
            permission_mode: permission_mode.as_str().to_string(),
        };

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
        let snapshot = match snapshot_result {
            Ok(s) => s,
            Err(e) => {
                self.release_execution_facet_contents(&execution_id).await;
                rollback_reservation(format!("validate_start failed: {e}")).await;
                return Err(e);
            }
        };

        // [04] commit point: ExecutionStarted と初回 NodeStarted を同一の required batch で
        // append する。初回 NodeExecution は insert 時点で採番済みなので、両 event を
        // atomic に記録しないと strict projection が execution の第一級 id を復元できない。
        let initial_node_started_event =
            match workflow_runtime_events::node_started_event_for_snapshot(&snapshot) {
                Ok(event) => event,
                Err(error) => {
                    self.executions.lock().await.remove(&execution_id);
                    self.release_execution_facet_contents(&execution_id).await;
                    rollback_reservation(format!(
                        "initial NodeStarted event construction failed: {error}"
                    ))
                    .await;
                    return Err(error);
                }
            };
        let required_start_events = vec![
            WorkflowEvent::ExecutionStarted {
                execution_id: snapshot.execution_id.clone(),
                workflow_name: snapshot.workflow_name.clone(),
                worktree_path: worktree_path.clone(),
                created_from,
                request: request.clone().unwrap_or_default(),
                permission_mode: permission_mode.as_str().to_string(),
                definition: workflow.clone(),
                timestamp: now,
            },
            initial_node_started_event,
        ];
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
        if let Err(e) = self
            .start_current_node_runtime(app, session_store, agent_runtime, &worktree_path)
            .await
        {
            if let Err(settle_error) = self
                .settle_runtime_failure(
                    app,
                    session_store,
                    agent_runtime,
                    &worktree_path,
                    &execution_id,
                    &e,
                )
                .await
            {
                log::error!(
                    "workflow {execution_id}: runtime start failed and NodeFailed settlement also failed: {settle_error}"
                );
            }
            log::warn!("workflow {execution_id}: post-commit node runtime start failed: {e}");
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_resolved_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        workflow: WorkflowDefinition,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        permission_mode: PermissionMode,
    ) -> Result<String, WorkflowRuntimeError> {
        self.start_workflow(
            app,
            session_store,
            agent_runtime,
            workflow,
            worktree_path,
            request,
            created_from,
            permission_mode,
        )
        .await
    }

    async fn restart_paused_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.restart_workflow_command_node(
            app,
            session_store,
            agent_runtime,
            execution_id,
            node_execution_id,
        )
        .await
    }

    async fn restart_workflow_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let timestamp = current_timestamp();
        let new_node_execution_id = new_node_execution_id();
        let (snapshot_before, mut candidate, worktree_path, is_fanout_child) = {
            let executions = self.executions.lock().await;
            let current = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            (
                current.clone(),
                current.clone(),
                current.worktree_path.clone(),
                current
                    .node_executions
                    .iter()
                    .find(|attempt| attempt.id == node_execution_id)
                    .is_some_and(|attempt| attempt.fanout_parent.is_some()),
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
        debug_assert_eq!(is_fanout_child, restarted.fanout_child);
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
                fanout_parent: new_attempt.fanout_parent.clone(),
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
        if is_fanout_child {
            self.finish_control_plane_commit(
                app,
                session_store,
                agent_runtime,
                &worktree_path,
                &snapshot,
                None,
            )
            .await?;
            if let Err(error) = self
                .start_retried_fanout_child(
                    app,
                    session_store,
                    agent_runtime,
                    &worktree_path,
                    &new_attempt.id,
                )
                .await
            {
                self.settle_runtime_failure_for_node(
                    app,
                    session_store,
                    agent_runtime,
                    &worktree_path,
                    execution_id,
                    &new_attempt.id,
                    &error,
                )
                .await?;
                log::warn!(
                    "workflow {execution_id}: retried fanout NodeExecution '{}' failed to activate: {error}",
                    new_attempt.id
                );
            }
        } else {
            self.finish_control_plane_commit(
                app,
                session_store,
                agent_runtime,
                &worktree_path,
                &snapshot,
                Some(NodeOutcome::RetryCurrentNode(snapshot.clone())),
            )
            .await?;
        }
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
        #[cfg(test)]
        if self
            .fail_next_required_event_append
            .swap(false, Ordering::AcqRel)
        {
            return Err(WorkflowRuntimeError::SessionStore(
                "injected required event append failure".to_string(),
            ));
        }
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
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowRuntimeError> {
        if let Some(outcome) = outcome {
            self.finalize_after_commit(app, snapshot, worktree_path)
                .await;
            self.dispatch_node_outcome_side_effects(
                app,
                session_store,
                agent_runtime,
                worktree_path,
                outcome,
            )
            .await
        } else {
            workflow_runtime_session::broadcast_state(app, worktree_path, snapshot.clone()).await;
            Ok(())
        }
    }

    /// AgentSession の無出力 timeout 到達を非終端 signal として workflow state に反映する。
    pub async fn on_agent_stall_observed<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_id: &str,
        turn_phase: String,
        idle_secs: u64,
        signal_count: u32,
        cap_reached: bool,
    ) -> Result<(), WorkflowRuntimeError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        let (snapshot, snapshot_before, worktree_path, execution_id, stall_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&session_ref.execution_id) else {
                return Ok(());
            };
            if !matches!(
                exec.observe_stall(),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Ok(());
            }

            let Some((node_execution_id, node_name, attempt)) =
                current_node_for_stall_observation(exec, session_id)
            else {
                return Ok(());
            };
            let snapshot_before = exec.clone();
            let observed_at = current_timestamp();
            let observation = NodeStallObservation {
                session_id: session_id.to_string(),
                node_name,
                attempt,
                turn_phase,
                idle_secs,
                signal_count,
                cap_reached,
                observed_at,
            };
            let _ = exec.observe_node_stall(observation.clone());
            let stall_event = WorkflowEvent::StallObserved {
                execution_id: exec.id.clone(),
                node_execution_id,
                session_id: observation.session_id,
                node_name: observation.node_name,
                attempt: observation.attempt,
                turn_phase: observation.turn_phase,
                idle_secs: observation.idle_secs,
                signal_count: observation.signal_count,
                cap_reached: observation.cap_reached,
                timestamp: observation.observed_at,
            };
            (
                RuntimeCommitSnapshot::from_execution(exec)?,
                snapshot_before,
                exec.worktree_path.clone(),
                exec.id.clone(),
                stall_event,
            )
        };

        self.commit_stall_event(
            app,
            &execution_id,
            snapshot,
            snapshot_before,
            self.execution_store
                .active_execution_snapshot(&execution_id)
                .await,
            worktree_path,
            stall_event,
            "workflow stall observed event append failed",
        )
        .await
    }

    /// AgentSession の無出力 timeout 観測が backend progress により解消されたことを workflow state に反映する。
    pub async fn on_agent_stall_cleared<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        let (snapshot, snapshot_before, worktree_path, execution_id, clear_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&session_ref.execution_id) else {
                return Ok(());
            };
            if !matches!(
                exec.observe_stall(),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let Some((node_execution_id, _, _)) =
                current_node_for_stall_observation(exec, session_id)
            else {
                return Ok(());
            };
            let cleared_at = current_timestamp();
            if !exec.clear_stalls_for_session(session_id, cleared_at) {
                return Ok(());
            }
            let clear_event = WorkflowEvent::StallCleared {
                execution_id: exec.id.clone(),
                node_execution_id,
                session_id: session_id.to_string(),
                timestamp: cleared_at,
            };
            (
                RuntimeCommitSnapshot::from_execution(exec)?,
                snapshot_before,
                exec.worktree_path.clone(),
                exec.id.clone(),
                clear_event,
            )
        };

        self.commit_stall_event(
            app,
            &execution_id,
            snapshot,
            snapshot_before,
            self.execution_store
                .active_execution_snapshot(&execution_id)
                .await,
            worktree_path,
            clear_event,
            "workflow stall cleared event append failed",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_stall_event<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        snapshot: RuntimeCommitSnapshot,
        snapshot_before: DomainWorkflowExecution,
        _execution_store_snapshot_before: Option<WorkflowExecutionMetadata>,
        worktree_path: String,
        event: WorkflowEvent,
        append_error_context: &'static str,
    ) -> Result<(), WorkflowRuntimeError> {
        let mutations = self
            .execution_store
            .prepare_atomic_existing_snapshot_mutations(&snapshot)
            .await
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        if let Err(error) = self.write_log_required_batch_with_mutations(app, &[event], mutations) {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(execution_id) {
                *exec = snapshot_before;
            }
            drop(execs);
            return Err(WorkflowRuntimeError::SessionStore(format!(
                "{append_error_context}: {error}"
            )));
        }
        if let Err(error) = workflow_runtime_commit::sync_execution_store_from_snapshot(
            &self.execution_store,
            execution_id,
            &snapshot,
        )
        .await
        {
            log::warn!(
                "workflow {execution_id}: derived projection refresh failed after stall commit: {error}"
            );
        }
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        Ok(())
    }

    /// Provider turnの異常終了を対応するNode AttemptのFailureへ写像する。
    /// 正常終了はSubmit / Stop handshakeを迂回してNodeを進行させない。
    #[allow(clippy::too_many_arguments)]
    pub async fn on_turn_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        session_id: &str,
        exit_code: i64,
        failure_signal: Option<workflow_transition::SessionFailureSignal>,
        _final_parts: &[crate::usecase::agent_session::session::MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowRuntimeError> {
        // session_id からSessionWorkflowRefを解決（ワークフロー既終了なら何もしない）
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        // parent ChatSession 機構撤去後は node session のみが登録されるため種別分岐なし。
        // 逐次 node / 並列子 node の区別は DomainWorkflowExecution.fanout_runtime に当該 session_id が
        // 含まれるかで判定する（Spec issues-929）。

        // SessionWorkflowRef.execution_id から exec を直接引き、属性として worktree_path を取得する
        // （Spec issues-1011: driver 内部キーも execution_id）。下流の handle_* は worktree_path を
        // 引数に取るため、ここで派生取得する。
        let (worktree_path, fanout_parent): (String, Option<String>) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(&session_ref.execution_id) else {
                return Ok(());
            };
            let context_authorized = exec.current_session_id.as_deref() == Some(session_id)
                || exec.fanout_runtime.as_ref().is_some_and(|fanout| {
                    fanout
                        .children
                        .iter()
                        .any(|child| child.session_id == session_id)
                });
            if !matches!(
                exec.admit_workflow_turn(context_authorized),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "execution {} does not admit WorkflowTurn for session {session_id}",
                    session_ref.execution_id
                )));
            }
            let canonical_fact = if exit_code == 0 && failure_signal.is_none() {
                CanonicalNodeFact::Completed
            } else {
                CanonicalNodeFact::Failed {
                    reason: format!("workflow-owned session failed (exit_code: {exit_code})"),
                    kind: fanout_child_failure_kind(exit_code, failure_signal),
                }
            };
            let _ = exec.apply_turn_completion(canonical_fact);
            let wt = exec.worktree_path.clone();
            let pp = exec.fanout_runtime.as_ref().and_then(|pr| {
                pr.children
                    .iter()
                    .find(|c| c.session_id == session_id)
                    .map(|_| pr.parent_node_name.clone())
            });
            (wt, pp)
        };

        if exit_code == 0 && failure_signal.is_none() {
            return Ok(());
        }

        if let Some(parent_node_name) = fanout_parent {
            return self
                .handle_fanout_child_complete(
                    app,
                    session_store,
                    agent_runtime,
                    &session_ref.execution_id,
                    &worktree_path,
                    session_id,
                    &parent_node_name,
                    exit_code,
                    failure_signal,
                    token_usage,
                )
                .await;
        }

        struct TurnCommit {
            outcome: NodeOutcome,
            required_events: Vec<WorkflowEvent>,
            rollback_snapshot: (String, DomainWorkflowExecution),
        }

        // 判定 + 状態変更を原子的に実行（AutoEvaluate以外）
        let action_or_outcome = {
            let mut execs = self.executions.lock().await;
            let exec = execs.get_mut(&session_ref.execution_id).ok_or_else(|| {
                WorkflowRuntimeError::ExecutionNotFound(session_ref.execution_id.clone())
            })?;

            // 現行ステップのセッション以外からの完了通知は無視
            if exec.current_session_id.as_deref() != Some(session_id) {
                return Ok(());
            }

            // トークン使用量を現在のステップに累計
            if let Some((input, output)) = token_usage {
                exec.add_current_token_usage(&TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
            let plan = exec
                .plan_turn_complete_mutation(exit_code, failure_signal)
                .map_err(
                    crate::usecase::workflow::runtime_error::workflow_error_to_runtime_error,
                )?;

            match plan {
                workflow_transition::TurnCompleteMutationPlan::NotRunning => return Ok(()),
                workflow_transition::TurnCompleteMutationPlan::SessionError {
                    node_name,
                    failure_reason,
                    kind,
                    ..
                } => {
                    if !exec.is_active() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let node_execution_id = exec
                        .active_current_node_execution_id()
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            WorkflowRuntimeError::InvalidState(format!(
                                "active NodeExecution for failed node '{node_name}' was not found"
                            ))
                        })?;
                    let attempt = exec
                        .node_executions
                        .iter()
                        .find(|node| node.id == node_execution_id)
                        .map(|node| node.attempt)
                        .ok_or_else(|| {
                            WorkflowRuntimeError::InvalidState(format!(
                                "failed NodeExecution '{node_execution_id}' disappeared"
                            ))
                        })?;
                    let timestamp = current_timestamp();
                    if exec.fail_node_execution(
                        &node_execution_id,
                        failure_reason.clone(),
                        kind,
                        timestamp,
                    ) != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                    {
                        return Ok(());
                    }
                    let outcome =
                        NodeOutcome::Persist(RuntimeCommitSnapshot::from_execution(exec)?);
                    let required_events = vec![WorkflowEvent::NodeFailed {
                        execution_id: exec.id.clone(),
                        node_execution_id,
                        node_name,
                        attempt,
                        reason: failure_reason,
                        failure_kind: kind,
                        retry_count: None,
                        timestamp,
                    }];
                    Ok(TurnCommit {
                        outcome,
                        required_events,
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                workflow_transition::TurnCompleteMutationPlan::RequestApproval { node_name } => {
                    if !exec.is_active() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let node_execution_id = exec
                        .active_current_node_execution_id()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            WorkflowRuntimeError::InvalidState(format!(
                                "active NodeExecution for approval-gated node '{node_name}' was not found"
                            ))
                        })?;
                    let timestamp = current_timestamp();
                    let _ = exec.mark_node_waiting_approval(&node_execution_id, timestamp);
                    exec.touch(timestamp);
                    Ok(TurnCommit {
                        outcome: NodeOutcome::Persist(RuntimeCommitSnapshot::from_execution(exec)?),
                        required_events: vec![WorkflowEvent::ApprovalRequested {
                            execution_id: exec.id.clone(),
                            node_execution_id,
                            node_name,
                            timestamp: exec.updated_at,
                        }],
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                workflow_transition::TurnCompleteMutationPlan::UnexpectedNodeKind {
                    failure_reason,
                    ..
                } => {
                    return Err(WorkflowRuntimeError::InvalidState(failure_reason));
                }
                workflow_transition::TurnCompleteMutationPlan::AutoEvaluate { node_name } => {
                    Err(node_name)
                }
            }
        };

        match action_or_outcome {
            Ok(commit) => {
                let (_, snapshot_before) = commit.rollback_snapshot.clone();
                if commit.required_events.is_empty() {
                    self.execute_outcome(
                        app,
                        session_store,
                        agent_runtime,
                        &worktree_path,
                        commit.outcome,
                        snapshot_before,
                    )
                    .await
                } else {
                    self.commit_required_turn_events_and_execute_outcome(
                        app,
                        session_store,
                        agent_runtime,
                        &worktree_path,
                        commit.outcome,
                        commit.required_events,
                        Some(commit.rollback_snapshot),
                    )
                    .await
                }
            }
            Err(_) => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_required_turn_events_and_execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        outcome: NodeOutcome,
        mut required_events: Vec<WorkflowEvent>,
        rollback_snapshot: Option<(String, DomainWorkflowExecution)>,
    ) -> Result<(), WorkflowRuntimeError> {
        let Some((execution_id, snapshot_before)) = rollback_snapshot else {
            return Err(WorkflowRuntimeError::SessionStore(
                "required turn event commit missing rollback snapshot".to_string(),
            ));
        };
        let snapshot_for_commit = outcome.snapshot().clone();
        let terminal_and_transition_events =
            match workflow_runtime_events::pre_commit_required_events_for_outcome(&outcome) {
                Ok(events) => events,
                Err(error) => {
                    let mut executions = self.executions.lock().await;
                    if let Some(execution) = executions.get_mut(&execution_id) {
                        *execution = snapshot_before;
                    }
                    return Err(error);
                }
            };
        required_events.extend(terminal_and_transition_events);
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(&execution_id)
            .await;

        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                operation_kind: CommitOperationKind::Workflow,
                execution_id: &execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                execution_store_snapshot_before,
                required_events,
                append_error_context: "turn_complete required event append failed",
            },
        )
        .await?;

        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path)
            .await;
        if let Err(e) = self
            .dispatch_node_outcome_side_effects(
                app,
                session_store,
                agent_runtime,
                worktree_path,
                outcome,
            )
            .await
        {
            log::warn!("workflow {execution_id}: post-commit turn side effects failed: {e}");
        }
        Ok(())
    }

    #[cfg(test)]
    fn apply_approval_application(
        exec: &mut DomainWorkflowExecution,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<NodeOutcome, WorkflowRuntimeError> {
        let timestamp = current_timestamp();
        let plan = exec
            .plan_approval_application(application)
            .map_err(crate::usecase::workflow::runtime_error::workflow_error_to_runtime_error)?;
        let completion = plan.completion;
        let entry = workflow_runtime_driver::make_node_history_entry(
            exec,
            Some(completion.result),
            completion.artifact,
            completion.contract,
            timestamp,
        );
        let completed_at = entry.completed_at;
        exec.record_history_entry(entry, completed_at);
        workflow_runtime_driver::apply_advance(exec, new_node_execution_id(), timestamp)
    }

    /// approvalモードでのユーザー判定を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行し、
    /// ロック外では永続化・ブロードキャスト・AgentSession起動のみ行う。
    ///
    /// Spec issues-1011 finding 2: lookup は `executions.get(execution_id)` / `get_mut(execution_id)` で
    /// 直接行い、worktree_path 経由の find は使用しない。同一 worktree に terminal/active
    /// 共存があっても execution_id 主語で取り違えない。worktree_path は exec から派生取得して
    /// 下流 (`fetch_current_output` / `execute_outcome`) に渡す。
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) async fn resolve_workflow_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        comment: Option<String>,
        expected_node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<(), WorkflowRuntimeError> {
        self.handle_approval(
            app,
            session_store,
            agent_runtime,
            execution_id,
            comment,
            expected_node_name,
            node_execution_id,
        )
        .await
    }

    /// [04] 内部 typed boundary: approval mutation の handler 実体。Tauri / local API は
    /// `resolve_workflow_approval*` からここに合流する。
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    async fn handle_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        comment: Option<String>,
        expected_node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<(), WorkflowRuntimeError> {
        let fanout_target_node_execution_id = {
            let executions = self.executions.lock().await;
            let execution = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            resolve_fanout_approval_target_node_execution_id(
                execution,
                expected_node_name,
                node_execution_id,
            )?
        };
        if let Some(fanout_target_node_execution_id) = fanout_target_node_execution_id {
            return self
                .handle_fanout_child_approval(
                    app,
                    session_store,
                    agent_runtime,
                    execution_id,
                    comment,
                    expected_node_name,
                    &fanout_target_node_execution_id,
                )
                .await;
        }

        // target検証 + session_id + worktree_path + contract 提出状態を1回のロックで取得
        let (
            current_session_id,
            worktree_path,
            approval_contract,
            approval_submitted_output,
            resolved_node_execution_id,
        ) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            workflow_approval_runtime::resolve_approval_target_snapshot(
                exec,
                Some(execution_id),
                Some(expected_node_name),
            )?;
            let execution_index =
                resolve_active_node_execution_index(exec, expected_node_name, node_execution_id)?;
            let resolved_node_execution_id = exec.node_executions[execution_index].id.clone();
            let node = &exec.workflow.nodes[exec.current_node_index];
            let contract = node.artifact.clone();
            let attempt = exec
                .node_execution_counts
                .get(&node.name)
                .copied()
                .unwrap_or(1);
            let submitted_output = contract.as_deref().and_then(|contract| {
                workflow_output_submission::submitted_node_artifact_for(
                    &exec.artifacts,
                    &node.name,
                    attempt,
                    contract,
                )
            });
            (
                exec.current_session_id.clone(),
                exec.worktree_path.clone(),
                contract,
                submitted_output,
                resolved_node_execution_id,
            )
        };

        workflow_approval_runtime::validate_approve_comment(comment.as_deref())?;
        let turn_phase = if let Some(ref sid) = current_session_id {
            agent_runtime.turn_phase(sid).await
        } else {
            None
        };
        workflow_approval_runtime::validate_approval_turn_phase(turn_phase)?;

        let approve_submitted_output = approval_submitted_output;

        let (artifact, contract_result): (Option<serde_json::Value>, Option<String>) =
            approve_submitted_output
                .as_ref()
                .map(|output| (output.artifact.clone(), output.result.clone()))
                .unwrap_or((None, None));

        let application_contract: Option<String> = if approve_submitted_output.is_some() {
            approval_contract.clone()
        } else {
            None
        };
        let effective_result = contract_result.unwrap_or_else(|| "approve".to_string());

        // [04] atomic mutation 境界: mutation 直前の DomainWorkflowExecution 全体を snapshot に
        // 保持し、ApprovalResolved event append / persist のいずれかが失敗した場合は
        // `*exec = snapshot` で全フィールド（履歴・変数・state・current_node_index 等）を
        // 一括復元する。部分 rollback helper は使わない。
        let (mut outcome, exec_snapshot_before, node_name_for_event) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            workflow_approval_runtime::resolve_approval_target_snapshot(
                exec,
                Some(execution_id),
                Some(expected_node_name),
            )?;
            let _execution_index = resolve_active_node_execution_index(
                exec,
                expected_node_name,
                Some(&resolved_node_execution_id),
            )?;
            let node_name = exec.workflow.nodes[exec.current_node_index].name.clone();
            let snapshot_before = exec.clone();
            let mut outcome = Self::apply_approval_application(
                exec,
                workflow_transition::ApprovalApplication {
                    effective_result,
                    artifact: artifact.clone(),
                    contract: application_contract,
                },
            )?;
            let _ = exec.complete_node_execution(
                &resolved_node_execution_id,
                artifact.clone(),
                None,
                current_timestamp(),
            );
            // `apply_approval_application` builds the NodeOutcome snapshot before the
            // NodeExecution read model is finalized above. Refresh it while still holding the
            // execution lock so the durable commit snapshot and the live mutation are identical;
            // otherwise the commit CAS correctly rejects this command as stale.
            *outcome.snapshot_mut() = RuntimeCommitSnapshot::from_execution(exec)?;
            (outcome, snapshot_before, node_name)
        };

        let snapshot_for_commit = outcome.snapshot().clone();
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(execution_id)
            .await;

        // [04] commit point: ApprovalResolved と、同じ受理サイクルで確定した
        // NodeCompleted / NodeStarted / terminal event を同一 batch で必須 append する。
        // append / persist 失敗時は snapshot で全フィールド一括復元する。
        // approve コメントには設定済み secret を含む可能性があるため、event log に
        // 保存する前に mask_sensitive_text() で redaction する。
        let event_comment = if let Some(raw) = comment {
            let secrets = secret_source::collect_configured_secret_values(app);
            Some(workflow_secret_masker::mask_sensitive_text(&raw, &secrets))
        } else {
            None
        };
        let approval_timestamp = current_timestamp();
        let approval_event = WorkflowEvent::ApprovalResolved {
            execution_id: execution_id.to_string(),
            node_execution_id: resolved_node_execution_id,
            node_name: node_name_for_event.clone(),
            comment: event_comment,
            timestamp: approval_timestamp,
        };
        // [05] silent error の禁止: required event 組立中に
        // `dispatch_internal_node_command` の ValidationError 等が発生した場合は
        // approval commit 境界として失敗扱いし、snapshot_before で driver state /
        // Execution Store / ChatSession を一括復元してから Err を返す。
        let commit_events = match workflow_runtime_events::required_events_for_approval_commit(
            approval_event,
            &mut outcome,
        ) {
            Ok(events) => events,
            Err(e) => {
                let _ = self
                    .rollback_command_mutation(
                        app,
                        session_store,
                        CommandMutationRollback {
                            execution_id,
                            snapshot_before: exec_snapshot_before,
                            execution_store_snapshot_before,
                            context: "approval required event build failed",
                        },
                    )
                    .await;
                return Err(e);
            }
        };
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                operation_kind: CommitOperationKind::UserMutation,
                execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before: exec_snapshot_before,
                execution_store_snapshot_before,
                required_events: commit_events,
                append_error_context: "approval commit batch append failed",
            },
        )
        .await?;

        // [04] post-commit: required event append 済みのため、ここから先の失敗は
        // command failure に射影しない（spec [04] post-commit 境界）。broadcast /
        // terminal log / cleanup / 次 node 起動 / auto-approve primitive は
        // ここで実行する。
        self.finalize_after_commit(app, &snapshot_for_commit, &worktree_path)
            .await;
        if let Err(e) = self
            .dispatch_node_outcome_side_effects(
                app,
                session_store,
                agent_runtime,
                &worktree_path,
                outcome,
            )
            .await
        {
            log::warn!("workflow {execution_id}: post-commit side effects failed: {e}");
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    async fn handle_fanout_child_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        comment: Option<String>,
        expected_node_name: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        workflow_approval_runtime::validate_approve_comment(comment.as_deref())?;
        let (worktree_path, session_id, attempt, contract, submitted_artifact) = {
            let executions = self.executions.lock().await;
            let execution = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let node_execution = execution
                .node_executions
                .iter()
                .find(|candidate| {
                    candidate.id == node_execution_id
                        && candidate.node_name == expected_node_name
                        && candidate.fanout_parent.is_some()
                        && candidate.status == NodeExecutionStatus::WaitingApproval
                })
                .ok_or_else(|| {
                    WorkflowRuntimeError::UnauthorizedApprovalTarget(format!(
                        "fanout approval NodeExecution '{node_execution_id}' is not waiting"
                    ))
                })?;
            let node = execution
                .workflow
                .nodes
                .iter()
                .find(|node| node.name == expected_node_name && node.is_approval_session())
                .ok_or_else(|| {
                    WorkflowRuntimeError::UnauthorizedApprovalTarget(
                        "node is not an approval session".to_string(),
                    )
                })?;
            (
                execution.worktree_path.clone(),
                node_execution.session_id.clone(),
                node_execution.attempt,
                node.artifact.clone(),
                node_execution.artifact.clone(),
            )
        };
        let turn_phase = if let Some(session_id) = session_id.as_deref() {
            agent_runtime.turn_phase(session_id).await
        } else {
            None
        };
        workflow_approval_runtime::validate_approval_turn_phase(turn_phase)?;

        let event_comment = comment.map(|raw| {
            let secrets = secret_source::collect_configured_secret_values(app);
            workflow_secret_masker::mask_sensitive_text(&raw, &secrets)
        });
        let completed_at = current_timestamp();
        let completion = {
            let mut executions = self.executions.lock().await;
            let execution = executions
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            if !matches!(
                execution.admit_fanout_approval(),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "execution {execution_id} does not accept fanout approval"
                )));
            }
            let snapshot_before = execution.clone();
            let _ = execution
                .node_executions
                .iter()
                .position(|candidate| {
                    candidate.id == node_execution_id
                        && candidate.status == NodeExecutionStatus::WaitingApproval
                })
                .ok_or_else(|| {
                    WorkflowRuntimeError::UnauthorizedApprovalTarget(format!(
                        "fanout approval NodeExecution '{node_execution_id}' is no longer waiting"
                    ))
                })?;
            let child = execution
                .fanout_runtime
                .as_ref()
                .and_then(|fanout| {
                    fanout
                        .children
                        .iter()
                        .find(|child| child.node_execution_id == node_execution_id)
                })
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "fanout approval child '{node_execution_id}' disappeared"
                    ))
                })?;
            let child_result = child.result.clone().or_else(|| Some("approve".to_string()));
            let child_tokens = child.token_usage.clone();
            let child_contract = child.contract.clone();
            let _ = execution.complete_fanout_child_execution(
                node_execution_id,
                child_result.clone(),
                submitted_artifact.clone(),
                child_contract,
                child_tokens.clone(),
                completed_at,
            );
            record_fanout_child_successful_completion(execution, expected_node_name);
            let mut progress_events = vec![WorkflowEvent::ApprovalResolved {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: expected_node_name.to_string(),
                comment: event_comment,
                timestamp: completed_at,
            }];
            if let Some(value) = submitted_artifact.clone() {
                progress_events.push(WorkflowEvent::ArtifactProduced {
                    execution_id: execution_id.to_string(),
                    node_execution_id: node_execution_id.to_string(),
                    node_name: expected_node_name.to_string(),
                    contract: contract.clone(),
                    value,
                    request_id: None,
                    submitted_at: None,
                    timestamp: completed_at,
                });
            }
            progress_events.push(WorkflowEvent::NodeCompleted {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: expected_node_name.to_string(),
                attempt,
                result_summary: child_result,
                token_usage: Some(child_tokens),
                timestamp: completed_at,
            });
            finalize_child_terminal_state(execution, snapshot_before, progress_events, true, None)?
        };

        if let Some(outcome) = completion.outcome {
            self.commit_required_fanout_progress_events_and_execute_outcome(
                app,
                session_store,
                agent_runtime,
                &worktree_path,
                CommitOperationKind::UserMutation,
                outcome,
                completion.snapshot_before,
                completion.progress_events,
                completion.failure_telemetry,
            )
            .await?;
        }
        Ok(())
    }

    async fn request_fanout_child_approval_if_needed<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        execution_id: &str,
        worktree_path: &str,
        session_id: &str,
        parent_node_name: &str,
    ) -> Result<bool, WorkflowRuntimeError> {
        let mutation = {
            let mut executions = self.executions.lock().await;
            let execution = executions
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            let Some(fanout) = execution.fanout_runtime.as_ref() else {
                return Ok(false);
            };
            if fanout.parent_node_name != parent_node_name {
                return Ok(false);
            }
            let Some(child) = fanout
                .children
                .iter()
                .find(|child| child.session_id == session_id)
            else {
                return Ok(false);
            };
            let Some(node) = execution
                .workflow
                .nodes
                .iter()
                .find(|node| node.name == child.node_name)
            else {
                return Ok(false);
            };
            if !node.is_approval_session() {
                return Ok(false);
            }
            let node_execution_id = child.node_execution_id.clone();
            let node_name = child.node_name.clone();
            let Some(node_execution_index) = execution
                .node_executions
                .iter()
                .position(|candidate| candidate.id == node_execution_id)
            else {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "fanout approval NodeExecution '{node_execution_id}' was not found"
                )));
            };
            if execution.node_executions[node_execution_index].status
                == NodeExecutionStatus::WaitingApproval
            {
                return Ok(true);
            }
            if execution.node_executions[node_execution_index].status
                != NodeExecutionStatus::Running
            {
                return Ok(true);
            }
            let snapshot_before = execution.clone();
            let timestamp = current_timestamp();
            let _ = execution.mark_node_waiting_approval(&node_execution_id, timestamp);
            let snapshot = RuntimeCommitSnapshot::from_execution(execution)?;
            let event = WorkflowEvent::ApprovalRequested {
                execution_id: execution_id.to_string(),
                node_execution_id,
                node_name,
                timestamp,
            };
            (snapshot_before, snapshot, event)
        };

        let (snapshot_before, snapshot, event) = mutation;
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(execution_id)
            .await;
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                operation_kind: CommitOperationKind::Workflow,
                execution_id,
                snapshot_for_commit: &snapshot,
                snapshot_before,
                execution_store_snapshot_before,
                required_events: vec![event],
                append_error_context: "fanout ApprovalRequested append failed",
            },
        )
        .await?;
        workflow_runtime_session::broadcast_state(app, worktree_path, snapshot).await;
        Ok(true)
    }

    /// 並列子ステップの完了を処理する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_fanout_child_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        worktree_path: &str,
        session_id: &str,
        parent_node_name: &str,
        exit_code: i64,
        failure_signal: Option<workflow_transition::SessionFailureSignal>,
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowRuntimeError> {
        // [08] fanout child の構造化出力は CLI / Tauri 経由の `SubmitOutput` で確定する。
        // contract がある child は、提出済み output が無い限り Completed にしない。
        let child_failed = exit_code != 0 || failure_signal.is_some();
        if !child_failed
            && self
                .request_fanout_child_approval_if_needed(
                    app,
                    session_store,
                    execution_id,
                    worktree_path,
                    session_id,
                    parent_node_name,
                )
                .await?
        {
            return Ok(());
        }
        let (child_result, child_artifact) = if !child_failed {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            if !exec.is_active() {
                return Ok(());
            }
            let Some(pr) = exec.fanout_runtime.as_ref() else {
                return Ok(());
            };
            if pr.parent_node_name != parent_node_name {
                return Ok(());
            }
            let Some(child) = pr.children.iter().find(|c| c.session_id == session_id) else {
                return Ok(());
            };
            let artifact = exec
                .node_executions
                .iter()
                .find(|execution| execution.id == child.node_execution_id)
                .and_then(|execution| execution.artifact.clone());
            (child.result.clone(), artifact)
        } else {
            (None, None)
        };
        // ロック内: 子ステップの状態更新 + 全完了チェック
        let (completion_commit, interrupted_session_ids, interrupted_command_ids) = 'state_update: {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;

            if !matches!(
                exec.complete_fanout_child(),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Ok(());
            }
            // [05] commit 境界: 子ステップ失敗 → workflow 全体 Failed の terminal event は
            // pre-commit batch で append し、失敗時は driver state を snapshot_before で
            // 一括復元する（post-persist warn 廃止）。snapshot は mutation 前にここで取得する。
            let exec_snapshot_before = exec.clone();
            let turn_usage = token_usage.map(|(input, output)| TokenUsage {
                input_tokens: input,
                output_tokens: output,
            });
            let Some(child) =
                exec.record_fanout_child_turn_usage(parent_node_name, session_id, turn_usage)
            else {
                return Ok(());
            };

            if child_failed {
                let failure_kind = fanout_child_failure_kind(exit_code, failure_signal);
                let child_name = child.node_name.clone();
                let child_node_execution_id = child.node_execution_id.clone();
                let reason = format!(
                    "fanout child '{}' failed (exit_code: {})",
                    child_name, exit_code
                );
                let failure_commit = finalize_fanout_child_failure_state(
                    exec,
                    exec_snapshot_before,
                    FanoutChildFailureInput {
                        child_node_execution_id,
                        child_failure_reason: reason,
                        failure_kind,
                        failure_disposition: if failure_kind.default_disposition()
                            == FailureDisposition::Partial
                        {
                            FailureDisposition::Partial
                        } else {
                            FailureDisposition::Terminal
                        },
                        retry_count: None,
                        timestamp: current_timestamp(),
                        record_child_token_usage: true,
                    },
                )?;
                break 'state_update (
                    failure_commit.completion,
                    failure_commit.interrupted_session_ids,
                    failure_commit.interrupted_command_ids,
                );
            }

            // 成功
            let child_name = child.node_name.clone();
            let child_node_execution_id = child.node_execution_id.clone();
            let child_token_usage = child.token_usage.clone();
            let child_attempt = child.attempt;
            let child_contract = child.contract.clone();
            let completed_at = current_timestamp();
            let _ = exec.complete_fanout_child_execution(
                &child_node_execution_id,
                child_result.clone(),
                child_artifact.clone(),
                child_contract.clone(),
                child_token_usage.clone(),
                completed_at,
            );
            record_fanout_child_successful_completion(exec, &child_name);

            // [08] child の RuntimeArtifact は CLI / Tauri 経由の SubmitOutput でのみ確定する。
            // ここでは artifacts slot に触れず、SubmitOutput 済みの値を保持したまま
            // fanout child も通常の NodeExecution として NodeCompleted を記録する。
            let mut progress_events = Vec::new();
            if let Some(value) = child_artifact.clone() {
                progress_events.push(WorkflowEvent::ArtifactProduced {
                    execution_id: exec.id.clone(),
                    node_execution_id: child_node_execution_id.clone(),
                    node_name: child_name.clone(),
                    contract: child_contract,
                    value,
                    request_id: None,
                    submitted_at: None,
                    timestamp: completed_at,
                });
            }
            progress_events.push(WorkflowEvent::NodeCompleted {
                execution_id: exec.id.clone(),
                node_execution_id: child_node_execution_id,
                node_name: child_name,
                attempt: child_attempt,
                result_summary: child_result.clone(),
                token_usage: Some(crate::domain::workflow::TokenUsage {
                    input_tokens: child_token_usage.input_tokens,
                    output_tokens: child_token_usage.output_tokens,
                }),
                timestamp: completed_at,
            });
            exec.clear_stalls_for_session(session_id, completed_at);

            (
                finalize_child_terminal_state(
                    exec,
                    exec_snapshot_before,
                    progress_events,
                    true,
                    None,
                )?,
                Vec::new(),
                Vec::new(),
            )
        };

        let FanoutChildCompletionCommit {
            all_completed,
            outcome,
            snapshot_before,
            progress_events,
            required_progress_events,
            failure_telemetry,
        } = completion_commit;

        if required_progress_events || !progress_events.is_empty() {
            let Some(outcome) = outcome else {
                let mut executions = self.executions.lock().await;
                if let Some(execution) = find_by_worktree_mut(&mut executions, worktree_path) {
                    *execution = snapshot_before;
                }
                return Err(WorkflowRuntimeError::InvalidState(
                    "fanout progress events require a durable outcome snapshot".to_string(),
                ));
            };
            self.commit_required_fanout_progress_events_and_execute_outcome(
                app,
                session_store,
                agent_runtime,
                worktree_path,
                CommitOperationKind::Workflow,
                outcome,
                snapshot_before,
                progress_events,
                failure_telemetry,
            )
            .await?;
            let recovery_suppressed = self.recovery_effects_suppressed(execution_id).await;
            if !recovery_suppressed {
                workflow_runtime_session::interrupt_agents(agent_runtime, &interrupted_session_ids)
                    .await?;
                if !interrupted_command_ids.is_empty() {
                    let handles = self.active_commands.lock().await;
                    for node_execution_id in interrupted_command_ids {
                        if let Some(handle) = handles.get(&node_execution_id) {
                            handle.request_shutdown();
                        }
                    }
                }
            }
            return Ok(());
        }

        if let Some(outcome) = outcome {
            if all_completed {
                self.execute_outcome(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    outcome,
                    snapshot_before,
                )
                .await?;
            } else {
                // まだ完了していない → Persistのみ
                if let NodeOutcome::Persist(snapshot) = outcome {
                    self.persist_and_broadcast(app, worktree_path, snapshot)
                        .await?;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_required_fanout_progress_events_and_execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        operation_kind: CommitOperationKind,
        outcome: NodeOutcome,
        snapshot_before: DomainWorkflowExecution,
        mut required_events: Vec<WorkflowEvent>,
        failure_telemetry: Option<FailureClassification>,
    ) -> Result<(), WorkflowRuntimeError> {
        let execution_id = outcome.snapshot().execution_id.clone();
        let snapshot_for_commit = outcome.snapshot().clone();
        let mut pre_commit_events =
            match workflow_runtime_events::pre_commit_required_events_for_outcome(&outcome) {
                Ok(events) => events,
                Err(e) => {
                    let mut execs = self.executions.lock().await;
                    if let Some(exec) = execs.get_mut(&execution_id) {
                        *exec = snapshot_before;
                    }
                    return Err(e);
                }
            };
        required_events.append(&mut pre_commit_events);

        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(&execution_id)
            .await;
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                operation_kind,
                execution_id: &execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                execution_store_snapshot_before,
                required_events,
                append_error_context: "fanout child progress event append failed",
            },
        )
        .await?;

        if let Some(classification) = failure_telemetry {
            crate::other::telemetry::record_workflow_node_failure(classification, None);
        }

        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path)
            .await;
        self.dispatch_node_outcome_side_effects(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            outcome,
        )
        .await
    }

    /// `execution_id` を直接指定して session_workflow_refs を掃除する。
    /// Spec issues-1011 finding 1: 同一 worktree に terminal/active 両方の execution が共存する
    /// 状況で、worktree 主語でクリーンアップすると別 execution の refs まで削除し得る。
    /// 全 cleanup 経路はこの execution_id 主語のメソッドを使う。
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

    /// 状態取得。`worktree_path` 属性で in-memory 実行表を検索する。
    pub async fn get_state(&self, worktree_path: &str) -> Option<RuntimeCommitSnapshot> {
        let execs = self.executions.lock().await;
        find_by_worktree(&execs, worktree_path)
            .and_then(|(_, execution)| RuntimeCommitSnapshot::from_execution(execution).ok())
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

    async fn release_fanout_resume_checkpoint(&self, execution_id: &str) {
        self.fanout_resume_checkpoints
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
            self.release_fanout_resume_checkpoint(execution_id).await;
        }
    }

    /// session_idがワークフロー実行中かどうか。
    pub async fn is_running(&self, session_id: &str) -> bool {
        let Some(worktree_path) = self.resolve_worktree_path(session_id).await else {
            return false;
        };
        let execs = self.executions.lock().await;
        find_by_worktree(&execs, &worktree_path).is_some_and(|(_, e)| e.is_active())
    }

    /// `execution_id` から approval 用 chat session（current node session）と worktree_path を解決する。
    /// Spec issues-1011 line 121: 起動以外の workflow 操作 API は execution_id を主語に取り、
    /// 内部の chat_session_id / worktree_path は driver が解決する。
    ///
    /// Spec issues-1011 finding 3: 任意 node session への注入経路を塞ぐため、resolve 時点で
    /// 以下を全て必須化する:
    ///   - 対象 execution が active であること
    ///   - state が `WaitingApproval` であること
    ///   - current node の `node_type` が `Approval` であること
    ///   - `current_session_id` が存在すること
    ///
    /// いずれかが不成立なら approval ターゲット解決を拒否する。
    pub async fn resolve_chat_session_for_approval(
        &self,
        execution_id: &str,
    ) -> Result<(String, String), WorkflowRuntimeError> {
        let execs = self.executions.lock().await;
        let exec = execs
            .get(execution_id)
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        let session_id = workflow_approval_runtime::resolve_chat_session_for_approval(exec)?;
        Ok((session_id, exec.worktree_path.clone()))
    }

    pub async fn validate_approval_chat_instruction(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        // parent ChatSession 機構撤去後は node session のみが session_workflow_refs に登録される。

        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(&session_ref.execution_id) else {
            return Ok(());
        };
        workflow_approval_runtime::validate_approval_chat_instruction(exec, session_id, content)
    }

    #[cfg(test)]
    pub async fn validate_approval_target(
        &self,
        worktree_path: &str,
        expected_execution_id: Option<&str>,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowRuntimeError> {
        let execs = self.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path)
            .ok_or_else(|| WorkflowRuntimeError::UnauthorizedWorktree(worktree_path.to_string()))?;
        workflow_approval_runtime::validate_approval_target_snapshot(
            exec,
            expected_execution_id,
            expected_node_name,
        )
    }

    /// セッションIDからworktree_pathを解決する。
    /// session_workflow_refsに登録されていない場合はNoneを返す。
    /// SessionWorkflowRef は execution_id を保持するため、executions から exec.worktree_path を
    /// 取得して返す（Spec issues-1011: driver 内部キーも execution_id）。
    pub async fn resolve_worktree_path(&self, session_id: &str) -> Option<String> {
        let execution_id = {
            let map = self.session_workflow_refs.lock().await;
            map.get(session_id).map(|r| r.execution_id.clone())?
        };
        let execs = self.executions.lock().await;
        execs.get(&execution_id).map(|e| e.worktree_path.clone())
    }

    /// セッションIDからSessionWorkflowRefを解決する。
    async fn resolve_session_ref(&self, session_id: &str) -> Option<SessionWorkflowRef> {
        let map = self.session_workflow_refs.lock().await;
        map.get(session_id).cloned()
    }

    // ---- 内部メソッド ----

    #[cfg(test)]
    async fn rollback_command_mutation<R: tauri::Runtime>(
        &self,
        _app: &tauri::AppHandle<R>,
        _session_store: &Arc<SessionStore>,
        rollback: CommandMutationRollback<'_>,
    ) -> Result<(), WorkflowRuntimeError> {
        let CommandMutationRollback {
            execution_id,
            snapshot_before,
            execution_store_snapshot_before,
            context,
        } = rollback;
        let execution_store_result =
            workflow_runtime_commit::restore_execution_store_active_snapshot(
                &self.execution_store,
                execution_store_snapshot_before,
            )
            .await;
        if let Err(ref rollback_err) = execution_store_result {
            log::warn!(
                "workflow {execution_id}: Execution Store rollback failed after {context}: {rollback_err}"
            );
        }
        let mut execs = self.executions.lock().await;
        if let Some(exec) = execs.get_mut(execution_id) {
            *exec = snapshot_before;
        }
        execution_store_result
    }

    async fn start_current_node_runtime<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let kind = {
            let execs = self.executions.lock().await;
            let (_, exec) = find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
            })?;
            exec.workflow.nodes[exec.current_node_index].kind_name()
        };
        match kind {
            NodeKindName::Command => {
                self.run_current_command_node(app, session_store, agent_runtime, worktree_path)
                    .await
            }
            NodeKindName::Session => self.start_node_session(app, worktree_path).await,
            NodeKindName::Fanout => {
                self.start_fanout_children(app, session_store, agent_runtime, worktree_path)
                    .await
            }
        }
    }

    async fn run_current_command_node<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let input = self.command_execution_input(worktree_path).await?;
        self.spawn_command_execution(app, session_store, agent_runtime, input)
            .await
    }

    async fn spawn_command_execution<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
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
        let observer_session_store = session_store.clone();
        let observer_agent_runtime = agent_runtime.clone();
        let node_execution_id = input.node_execution_id.clone();
        let still_current = self.command_execution_still_current(&input).await;
        let observer_node_execution_id = node_execution_id.clone();
        let runtime_handle = tokio::runtime::Handle::current();
        let observer = tokio::task::spawn_blocking(move || {
            runtime_handle.block_on(async move {
                driver
                    .observe_command_completion(
                        &observer_app,
                        &observer_session_store,
                        &observer_agent_runtime,
                        input,
                        running,
                    )
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
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
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
                if let Err(error) = self
                    .commit_command_output(app, session_store, agent_runtime, input, output)
                    .await
                {
                    let reason = format!("command completion failed: {error}");
                    log::warn!("{reason}");
                    if self.command_execution_still_current(&failure_input).await {
                        if let Err(settle_error) = self
                            .settle_runtime_failure(
                                app,
                                session_store,
                                agent_runtime,
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
                    .fail_current_command_node(
                        app,
                        session_store,
                        agent_runtime,
                        &input,
                        reason.clone(),
                    )
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

    async fn command_execution_input(
        &self,
        worktree_path: &str,
    ) -> Result<CommandExecutionInput, WorkflowRuntimeError> {
        let execs = self.executions.lock().await;
        let (execution_id, exec) = find_by_worktree(&execs, worktree_path)
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string()))?;
        let node = &exec.workflow.nodes[exec.current_node_index];
        let Some(command) = node.command() else {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "current node '{}' is not a command",
                node.name
            )));
        };
        let artifacts = workflow_prompt::artifact_values(&exec.artifacts, exec.request.as_deref());
        let rendered_command =
            workflow_prompt::render_artifact_references(command, &artifacts, None);
        let attempt = exec
            .node_execution_counts
            .get(&node.name)
            .copied()
            .unwrap_or(1);
        let node_execution_id = exec
            .node_executions
            .iter()
            .rev()
            .find(|node_execution| {
                node_execution.node_name == node.name
                    && node_execution.attempt == attempt
                    && node_execution.fanout_parent.is_none()
                    && node_execution.status.is_active()
            })
            .map(|node_execution| node_execution.id.clone())
            .ok_or_else(|| {
                WorkflowRuntimeError::InvalidState(format!(
                    "active NodeExecution for '{}' attempt {} is unavailable",
                    node.name, attempt
                ))
            })?;
        Ok(CommandExecutionInput {
            execution_id: execution_id.clone(),
            node_execution_id,
            node_name: node.name.clone(),
            attempt,
            worktree_path: exec.worktree_path.clone(),
            raw_command: Some(rendered_command),
            contract: node.artifact.clone(),
            schemas: exec.workflow.schemas.clone(),
            fanout_parent: None,
            session_id: exec.current_session_id.clone().or_else(|| {
                exec.node_executions
                    .iter()
                    .rev()
                    .find_map(|node_execution| node_execution.session_id.clone())
            }),
        })
    }

    async fn command_execution_still_current(&self, input: &CommandExecutionInput) -> bool {
        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(&input.execution_id) else {
            return false;
        };
        command_execution_input_is_current(exec, input)
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_command_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        input: CommandExecutionInput,
        output: CommandRunOutput,
    ) -> Result<(), WorkflowRuntimeError> {
        if input.fanout_parent.is_some() {
            return self
                .commit_fanout_command_output(app, session_store, agent_runtime, input, output)
                .await;
        }
        let secrets = secret_source::collect_configured_secret_values(app);
        let artifact =
            build_command_artifact(&input.schemas, input.contract.as_deref(), output, &secrets);
        let artifact_value = artifact.value.clone();
        let artifact_event_contract = artifact.event_contract.clone();
        let result_summary = Some(artifact.result_summary.clone());
        let timestamp = current_timestamp();

        let (outcome, snapshot_before, snapshot_for_commit, worktree_path) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&input.execution_id) else {
                return Ok(());
            };
            if !is_still_current_execution(exec, &input.node_name, input.attempt) {
                return Ok(());
            }

            let snapshot_before = exec.clone();
            let entry = workflow_runtime_driver::make_node_history_entry(
                exec,
                result_summary,
                None,
                None,
                timestamp,
            );
            let completed_at = entry.completed_at;
            let attempt = entry.attempt;
            exec.record_history_entry(entry, completed_at);
            exec.upsert_artifact(
                RuntimeArtifact {
                    node_name: input.node_name.clone(),
                    attempt,
                    session_id: None,
                    result: Some(artifact.result_summary),
                    artifact: Some(artifact_value.clone()),
                    contract: artifact_event_contract.clone(),
                    token_usage: None,
                    completed_at,
                },
                completed_at,
            );
            let _ = exec.complete_node_execution(
                &input.node_execution_id,
                Some(artifact_value.clone()),
                None,
                timestamp,
            );
            let outcome =
                workflow_runtime_driver::apply_advance(exec, new_node_execution_id(), timestamp)?;
            (
                outcome,
                snapshot_before,
                RuntimeCommitSnapshot::from_execution(exec)?,
                exec.worktree_path.clone(),
            )
        };

        let artifact_event = WorkflowEvent::ArtifactProduced {
            execution_id: input.execution_id.clone(),
            node_execution_id: input.node_execution_id.clone(),
            node_name: input.node_name.clone(),
            contract: artifact_event_contract,
            value: artifact_value,
            request_id: None,
            submitted_at: None,
            timestamp,
        };
        let mut required_events = vec![artifact_event];
        match workflow_runtime_events::pre_commit_required_events_for_outcome(&outcome) {
            Ok(events) => required_events.extend(events),
            Err(error) => {
                let mut execs = self.executions.lock().await;
                if let Some(exec) = execs.get_mut(&input.execution_id) {
                    *exec = snapshot_before;
                }
                return Err(error);
            }
        }
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(&input.execution_id)
            .await;
        self.commit_required_events(
            app,
            session_store,
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
        Box::pin(self.dispatch_node_outcome_side_effects(
            app,
            session_store,
            agent_runtime,
            &worktree_path,
            outcome,
        ))
        .await
    }

    async fn commit_fanout_command_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        input: CommandExecutionInput,
        output: CommandRunOutput,
    ) -> Result<(), WorkflowRuntimeError> {
        let secrets = secret_source::collect_configured_secret_values(app);
        let artifact =
            build_command_artifact(&input.schemas, input.contract.as_deref(), output, &secrets);
        let artifact_value = artifact.value;
        let result_summary = artifact.result_summary;
        let event_contract = artifact.event_contract;
        let completed_at = current_timestamp();

        let completion = {
            let mut executions = self.executions.lock().await;
            let Some(execution) = executions.get_mut(&input.execution_id) else {
                return Ok(());
            };
            if !self.command_input_is_active_fanout_child(execution, &input) {
                return Ok(());
            }
            let snapshot_before = execution.clone();
            let child = execution
                .fanout_runtime
                .as_ref()
                .and_then(|fanout| {
                    fanout
                        .children
                        .iter()
                        .find(|child| child.node_execution_id == input.node_execution_id)
                })
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "active fanout command child '{}' was not found",
                        input.node_execution_id
                    ))
                })?;
            let child_contract = child.contract.clone();
            let child_token_usage = child.token_usage.clone();
            let _ = execution.complete_fanout_child_execution(
                &input.node_execution_id,
                Some(result_summary.clone()),
                Some(artifact_value.clone()),
                child_contract,
                child_token_usage,
                completed_at,
            );
            record_fanout_child_successful_completion(execution, &input.node_name);
            let progress_events = vec![
                WorkflowEvent::ArtifactProduced {
                    execution_id: input.execution_id.clone(),
                    node_execution_id: input.node_execution_id.clone(),
                    node_name: input.node_name.clone(),
                    contract: event_contract,
                    value: artifact_value.clone(),
                    request_id: None,
                    submitted_at: None,
                    timestamp: completed_at,
                },
                WorkflowEvent::NodeCompleted {
                    execution_id: input.execution_id.clone(),
                    node_execution_id: input.node_execution_id.clone(),
                    node_name: input.node_name.clone(),
                    result_summary: Some(result_summary),
                    token_usage: None,
                    attempt: input.attempt,
                    timestamp: completed_at,
                },
            ];
            finalize_child_terminal_state(execution, snapshot_before, progress_events, true, None)?
        };

        if let Some(outcome) = completion.outcome {
            self.commit_required_fanout_progress_events_and_execute_outcome(
                app,
                session_store,
                agent_runtime,
                &input.worktree_path,
                CommitOperationKind::Workflow,
                outcome,
                completion.snapshot_before,
                completion.progress_events,
                completion.failure_telemetry,
            )
            .await?;
        }
        Ok(())
    }

    fn command_input_is_active_fanout_child(
        &self,
        execution: &DomainWorkflowExecution,
        input: &CommandExecutionInput,
    ) -> bool {
        execution.is_active()
            && execution.node_executions.iter().any(|node_execution| {
                node_execution.id == input.node_execution_id
                    && node_execution.status == NodeExecutionStatus::Running
            })
            && execution.fanout_runtime.as_ref().is_some_and(|fanout| {
                input.fanout_parent.as_deref() == Some(fanout.parent_node_name.as_str())
                    && fanout.children.iter().any(|child| {
                        child.node_execution_id == input.node_execution_id
                            && child.state == FanoutChildRuntimeState::Running
                    })
            })
    }

    async fn fail_current_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
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
                session_store,
                agent_runtime,
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

    /// 現在のSession Node用AgentSessionを起動し、初期指示を送信する。
    async fn start_node_session<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let deps = RealNodeSessionDeps {
            app,
            agent_sessions: self.workflow_agent_sessions.as_ref(),
            execution_store: &self.execution_store,
        };
        self.start_node_session_with_deps(&deps, worktree_path)
            .await
    }

    /// `start_node_session` のコアロジック。副作用境界は `NodeSessionDeps` 経由で注入する。
    ///
    /// プロンプト合成、AgentSession起動、Workflow所有関係の永続化、初期指示の順で実行する。
    async fn start_node_session_with_deps<D: NodeSessionDeps + ?Sized>(
        &self,
        deps: &D,
        worktree_path: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let activation_execution_id = {
            let executions = self.executions.lock().await;
            let (execution_id, execution) = find_by_worktree(&executions, worktree_path)
                .ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
                })?;
            if !matches!(
                execution.start_fanout_child(),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "execution {execution_id} does not admit fanout expansion"
                )));
            }
            execution_id.clone()
        };
        let activation_gate = self.runtime_activation_gate(&activation_execution_id).await;
        let _activation_guard = activation_gate.lock.lock().await;
        let (
            execution_id_for_ref,
            node_clone,
            artifacts_clone,
            task_clone,
            node_execution_id,
            workflow_clone,
        ) = {
            let execs = self.executions.lock().await;
            let (execution_id, exec) =
                find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
                })?;
            if execution_id != &activation_execution_id {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "worktree {worktree_path} changed execution before session activation"
                )));
            }
            let node = &exec.workflow.nodes[exec.current_node_index];
            let node_attempt = exec
                .node_execution_counts
                .get(&node.name)
                .copied()
                .unwrap_or(1);
            let node_execution_id = exec
                .node_executions
                .iter()
                .rev()
                .find(|node_execution| {
                    node_execution.node_name == node.name
                        && node_execution.attempt == node_attempt
                        && node_execution.fanout_parent.is_none()
                        && node_execution.status.is_active()
                })
                .map(|node_execution| node_execution.id.clone())
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "active NodeExecution for '{}' attempt {} is unavailable",
                        node.name, node_attempt
                    ))
                })?;
            (
                execution_id.clone(),
                node.clone(),
                exec.artifacts.clone(),
                exec.request.clone(),
                node_execution_id,
                exec.workflow.clone(),
            )
        };
        let facet_contents = self
            .facet_contents_for_execution(&execution_id_for_ref, &workflow_clone)
            .await?;
        let node_facet_contents = facet_contents.for_node(&node_clone.name);

        // プロンプト合成に失敗した場合はAgentSessionもPTYも作成しない。
        let (system_prompt, prompt) = workflow_prompt::build_node_prompt(
            &node_clone,
            node_facet_contents,
            &execution_id_for_ref,
            task_clone.as_deref(),
            &artifacts_clone,
            &workflow_clone.schemas,
        )?;
        let initial_instruction =
            crate::domain::workflow::services::prompt_composition::provider_tui_initial_instruction(
                system_prompt.as_deref(),
                &prompt,
            );
        let provider = node_clone
            .session()
            .ok_or_else(|| {
                WorkflowRuntimeError::InvalidWorkflow(format!(
                    "Node '{}' is not a Session Node",
                    node_clone.name
                ))
            })?
            .provider;
        let node_session = deps
            .prepare_workflow_agent_session(
                worktree_path,
                provider,
                &execution_id_for_ref,
                &node_execution_id,
                &initial_instruction,
            )
            .await?;
        let node_session_id = node_session.id.clone();

        // SessionAttached とその projection が durable になるまで、候補 aggregate を
        // live registry へ公開しない。これにより、UI/Hook が session_id を観測して
        // 完了処理を開始し、添付 event の commit を追い越すことを防ぐ。
        let snapshot = {
            let mut execs = self.executions.lock().await;
            let execution = execs.get_mut(&execution_id_for_ref).ok_or_else(|| {
                WorkflowRuntimeError::ExecutionNotFound(execution_id_for_ref.clone())
            })?;
            let mut candidate = execution.clone();
            if !matches!(
                candidate.attach_node_session(
                    &node_execution_id,
                    node_session_id.clone(),
                    current_timestamp(),
                ),
                crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            ) {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "NodeExecution '{node_execution_id}' does not admit AgentSession attachment"
                )));
            }
            let snapshot = RuntimeCommitSnapshot::from_execution(&candidate)?;
            if let Err(append_error) = deps.append_node_session_started(&snapshot).await {
                deps.rollback_workflow_agent_session(&node_session_id, &node_execution_id)
                    .await
                    .map_err(|rollback_error| {
                        WorkflowRuntimeError::AgentSession(format!(
                            "{append_error}; rollback failed: {rollback_error}"
                        ))
                    })?;
                return Err(append_error);
            }
            *execution = candidate;
            snapshot
        };

        // durable attachmentだけをAgentSessionからWorkflowへ解決可能にする。
        {
            let mut map = self.session_workflow_refs.lock().await;
            map.insert(
                node_session_id.clone(),
                SessionWorkflowRef {
                    execution_id: execution_id_for_ref.clone(),
                },
            );
        }
        deps.broadcast_state(worktree_path, snapshot).await;

        // AgentSession/Workflow双方のdurable stateを確定した後に、初回指示を起動引数として
        // Provider TUIをspawnする。PTY起動直後の入力競合は作らない。
        run_runtime_activation(
            &activation_gate,
            &execution_id_for_ref,
            "session",
            deps.activate_workflow_agent_session(&node_session_id, &node_execution_id),
        )
        .await
    }

    /// [08] prose 抽出経路は driver から完全除去された（spec [08] Rule 4 構造化出力の
    /// 確定経路は明示的提出のみ）。本 helper は ChatSession 表示など event log と無関係な
    /// 経路で「最後の Agent メッセージ本文」を取り出すテスト用 fixture としてのみ残す。
    #[cfg(test)]
    fn extract_last_assistant_text_from_session(
        session: &crate::usecase::agent_session::session::ChatSession,
    ) -> Option<String> {
        let agent_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::usecase::agent_session::session::MessageRole::Agent)?;

        let text = if let Some(ref parts) = agent_msg.parts {
            turn_completion::extract_text_from_parts(parts)
        } else {
            agent_msg.content.clone()
        };

        if text.is_empty() {
            return None;
        }

        Some(text)
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
        _session_store: &Arc<SessionStore>,
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

    /// non-command 経路のcanonical projectionを同期する。
    async fn sync_projection<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError> {
        let execution_id = snapshot.execution_id.clone();
        let mutations = self
            .execution_store
            .prepare_atomic_existing_snapshot_mutations(snapshot)
            .await
            .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
        workflow_event_log_writer::commit_projection_with_mutations_for_app(
            app,
            &execution_id,
            mutations,
        )
        .map_err(WorkflowRuntimeError::SessionStore)?;
        if let Err(e) = workflow_runtime_commit::sync_execution_store_from_snapshot(
            &self.execution_store,
            &execution_id,
            snapshot,
        )
        .await
        {
            log::warn!(
                "workflow {execution_id}: derived execution projection refresh failed after canonical projection commit: {e}"
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

    /// 既存呼び出し元（on_turn_complete 等）から使う一括 helper。pre-commit と post-commit
    /// を順に呼ぶだけで、外部 contract は変えない。
    #[allow(clippy::too_many_arguments)]
    async fn persist_and_broadcast<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        snapshot: RuntimeCommitSnapshot,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        self.sync_projection(app, &snapshot).await?;
        self.finalize_after_commit(app, &snapshot, worktree_path)
            .await;
        Ok(snapshot)
    }

    /// ロック外でNodeOutcomeに応じた副作用（永続化・ブロードキャスト・AgentSession起動）を実行する。
    ///
    /// 本 helper は non-command 経路（NodeCompleted / NodeFailed 等）から呼ばれる。
    ///
    /// [05] commit 境界: spec [04] commit_required_events を基盤に、NodeOutcome から
    /// `NodeCompleted` / `NodeFailed` / `ExecutionCompleted` / `ExecutionFailed` の必須 event を
    /// 組み立て、ExecutionStore sync → ChatSession persist → event log append の順で commit
    /// する。いずれかの phase で失敗した場合は driver state と Execution Store snapshot を
    /// `snapshot_before` で一括復元することで、event log と driver state / ExecutionStore /
    /// ChatSession の分離を防ぐ（spec [05]: state mutation と event log の分離を防ぐ
    /// rollback 境界 / atomic mutation 境界）。
    ///
    /// 必須 event が空の場合はcanonical projectionだけを同期する。
    async fn execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        outcome: NodeOutcome,
        snapshot_before: DomainWorkflowExecution,
    ) -> Result<(), WorkflowRuntimeError> {
        let snapshot_for_commit = outcome.snapshot().clone();
        let execution_id = snapshot_for_commit.execution_id.clone();

        // [05] pre-commit phase: 必須 event の生成。`dispatch_internal_node_command` の
        // ValidationError は driver state を snapshot_before で復元して伝播する
        // （spec [05] silent error 禁止）。
        let pre_commit_events =
            match workflow_runtime_events::pre_commit_required_events_for_outcome(&outcome) {
                Ok(events) => events,
                Err(e) => {
                    let mut execs = self.executions.lock().await;
                    if let Some(exec) = execs.get_mut(&execution_id) {
                        *exec = snapshot_before;
                    }
                    return Err(e);
                }
            };

        if !pre_commit_events.is_empty() {
            // [05] commit_required_events 基盤: 順序と rollback 方針を一箇所に集約。
            // 失敗時は driver state と Execution Store snapshot を一括復元する。
            let execution_store_snapshot_before = self
                .execution_store
                .active_execution_snapshot(&execution_id)
                .await;
            self.commit_required_events(
                app,
                session_store,
                RequiredEventCommit {
                    operation_kind: CommitOperationKind::Workflow,
                    execution_id: &execution_id,
                    snapshot_for_commit: &snapshot_for_commit,
                    snapshot_before,
                    execution_store_snapshot_before,
                    required_events: pre_commit_events,
                    append_error_context: "execute_outcome required event append failed",
                },
            )
            .await?;
        } else {
            self.sync_projection(app, &snapshot_for_commit).await?;
        }

        // terminal / NodeCompleted are already part of the canonical commit.
        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path)
            .await;
        self.dispatch_node_outcome_side_effects(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            outcome,
        )
        .await
    }

    async fn settle_runtime_failure<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
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
                .active_current_node_execution_id()
                .map(str::to_owned)
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "workflow '{execution_id}' has no active node attempt to fail"
                    ))
                })?
        };
        self.settle_runtime_failure_for_node(
            app,
            session_store,
            agent_runtime,
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
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
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
                    session_store,
                    agent_runtime,
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
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        execution_id: &str,
        node_execution_id: &str,
        reason: String,
        failure_kind: NodeExecutionFailureKind,
    ) -> Result<(), WorkflowRuntimeError> {
        let timestamp = current_timestamp();
        let (snapshot_before, mut candidate, node_name, attempt, is_fanout_child) = {
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
                node.fanout_parent.is_some(),
            )
        };
        let transition = if is_fanout_child {
            candidate.fail_fanout_child_execution(
                node_execution_id,
                reason.clone(),
                failure_kind,
                FailureDisposition::Terminal,
                timestamp,
            )
        } else {
            candidate.fail_node_execution(
                node_execution_id,
                reason.clone(),
                failure_kind,
                timestamp,
            )
        };
        if transition
            != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        {
            return Ok(());
        }
        if !is_fanout_child {
            let history = candidate.make_failed_node_history_entry_at(
                Some(reason.clone()),
                None,
                None,
                timestamp,
            );
            candidate.record_history_entry(history, timestamp);
        }
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
                    operation_kind: CommitOperationKind::Workflow,
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
            session_store,
            agent_runtime,
            worktree_path,
            &snapshot,
            None,
        )
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
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
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
            NodeOutcome::RetryCurrentNode(snapshot) => {
                if let Err(e) = Box::pin(self.start_current_node_runtime(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                ))
                .await
                {
                    if let Err(settle_error) = Box::pin(self.settle_runtime_failure(
                        app,
                        session_store,
                        agent_runtime,
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
            NodeOutcome::TransitionAndStart(snapshot) => {
                if let Err(e) = Box::pin(self.start_current_node_runtime(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                ))
                .await
                {
                    if let Err(settle_error) = Box::pin(self.settle_runtime_failure(
                        app,
                        session_store,
                        agent_runtime,
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
            NodeOutcome::StartFanout(snapshot) => {
                if let Err(e) = Box::pin(self.start_fanout_children(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                ))
                .await
                {
                    if let Err(settle_error) = Box::pin(self.settle_runtime_failure(
                        app,
                        session_store,
                        agent_runtime,
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

    /// fanout の子 node execution を展開して起動する。
    #[allow(clippy::too_many_arguments)]
    async fn rollback_unattached_fanout_sessions(
        &self,
        child_setups: &[workflow_fanout_runtime::FanoutChildSessionSetup],
    ) -> Option<WorkflowRuntimeError> {
        {
            let mut refs = self.session_workflow_refs.lock().await;
            for setup in child_setups {
                refs.remove(&setup.session_id);
            }
        }
        let mut rollback_failure = None;
        for setup in child_setups {
            if let Err(error) = self
                .workflow_agent_sessions
                .rollback_workflow_agent_session(&setup.session_id, &setup.node_execution_id)
                .await
            {
                rollback_failure.get_or_insert(error);
            }
        }
        rollback_failure
    }

    async fn start_retried_fanout_child<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let (execution_id, fanout_start, prompt_inputs, workflow, node_kind, attempt) = {
            let executions = self.executions.lock().await;
            let (execution_id, execution) = find_by_worktree(&executions, worktree_path)
                .ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
                })?;
            let node_execution = execution
                .node_executions
                .iter()
                .find(|node| {
                    node.id == node_execution_id
                        && node.status.is_active()
                        && node.fanout_parent.is_some()
                })
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "retried fanout NodeExecution '{node_execution_id}' is not active"
                    ))
                })?;
            let fanout_parent = node_execution
                .fanout_parent
                .as_ref()
                .expect("fanout retry target must have a parent");
            let mut fanout_start =
                workflow_fanout_runtime::prepare_fanout_start_context(execution)?;
            let mut child = fanout_start
                .children
                .into_iter()
                .find(|child| {
                    child.node.name == node_execution.node_name
                        && child.item_index == fanout_parent.item_index
                        && child.child_index == fanout_parent.child_index
                })
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "fanout expansion no longer contains retried NodeExecution '{node_execution_id}'"
                    ))
                })?;
            child.node_execution_id = node_execution_id.to_string();
            child.attempt = node_execution.attempt;
            child.reused = None;
            fanout_start.children = vec![child];
            (
                execution_id.clone(),
                fanout_start,
                workflow_fanout_runtime::fanout_prompt_inputs(execution),
                execution.workflow.clone(),
                node_execution.kind,
                node_execution.attempt,
            )
        };
        let activation_gate = self.runtime_activation_gate(&execution_id).await;
        let _activation_guard = activation_gate.lock.lock().await;
        match node_kind {
            NodeKindName::Session => {
                let facet_contents = self
                    .facet_contents_for_execution(&execution_id, &workflow)
                    .await?;
                let mut plans = workflow_runtime_session::prepare_fanout_child_session_plans(
                    &fanout_start,
                    &prompt_inputs,
                    &facet_contents,
                    &workflow.schemas,
                )?;
                let plan = plans.pop().ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "retried Session NodeExecution '{node_execution_id}' has no activation plan"
                    ))
                })?;
                let session = self
                    .workflow_agent_sessions
                    .prepare_workflow_agent_session(
                        worktree_path,
                        plan.provider,
                        &execution_id,
                        node_execution_id,
                        &plan.initial_instruction,
                    )
                    .await?;
                let timestamp = current_timestamp();
                let attach_result = async {
                    let (snapshot_before, mut candidate) = {
                        let executions = self.executions.lock().await;
                        let execution = executions.get(&execution_id).ok_or_else(|| {
                            WorkflowRuntimeError::ExecutionNotFound(execution_id.clone())
                        })?;
                        (execution.clone(), execution.clone())
                    };
                    if candidate.attach_child_node_session(
                        node_execution_id,
                        session.id.clone(),
                        timestamp,
                    ) != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
                    {
                        return Err(WorkflowRuntimeError::InvalidState(format!(
                            "retried fanout NodeExecution '{node_execution_id}' rejected its AgentSession"
                        )));
                    }
                    let snapshot = self
                        .commit_control_plane_candidate(
                            app,
                            ControlPlaneCommitCandidate {
                                operation_kind: CommitOperationKind::Workflow,
                                execution_id: &execution_id,
                                snapshot_before,
                                candidate,
                                events: &[WorkflowEvent::SessionAttached {
                                    execution_id: execution_id.clone(),
                                    node_execution_id: node_execution_id.to_string(),
                                    session_id: session.id.clone(),
                                    timestamp,
                                }],
                                provider_events: Vec::new(),
                            },
                        )
                        .await?;
                    self.finish_control_plane_commit(
                        app,
                        session_store,
                        agent_runtime,
                        worktree_path,
                        &snapshot,
                        None,
					)
					.await?;
                    Ok::<(), WorkflowRuntimeError>(())
                }
                .await;
                if let Err(error) = attach_result {
                    self.workflow_agent_sessions
                        .rollback_workflow_agent_session(&session.id, node_execution_id)
                        .await
                        .map_err(|rollback_error| {
                            WorkflowRuntimeError::AgentSession(format!(
                                "{error}; rollback failed: {rollback_error}"
                            ))
                        })?;
                    return Err(error);
                }
                self.session_workflow_refs.lock().await.insert(
                    session.id.clone(),
                    SessionWorkflowRef {
                        execution_id: execution_id.clone(),
                    },
                );
                run_runtime_activation(
                    &activation_gate,
                    &execution_id,
                    "fanout retry",
                    self.workflow_agent_sessions
                        .activate_workflow_agent_session(&session.id, node_execution_id),
                )
                .await
            }
            NodeKindName::Command => {
                let child = fanout_start
                    .children
                    .first()
                    .expect("retried fanout context must contain one child");
                let command = child.node.command().ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "retried Command NodeExecution '{node_execution_id}' has no command"
                    ))
                })?;
                let artifacts = workflow_prompt::artifact_values(
                    &prompt_inputs.artifacts,
                    fanout_start.request.as_deref(),
                );
                self.spawn_command_execution(
                    app,
                    session_store,
                    agent_runtime,
                    CommandExecutionInput {
                        execution_id,
                        node_execution_id: node_execution_id.to_string(),
                        node_name: child.node.name.clone(),
                        attempt,
                        worktree_path: worktree_path.to_string(),
                        raw_command: Some(workflow_prompt::render_artifact_references(
                            command,
                            &artifacts,
                            child.item.as_ref(),
                        )),
                        contract: child.node.artifact.clone(),
                        schemas: workflow.schemas.clone(),
                        fanout_parent: Some(fanout_start.parent_node_name),
                        session_id: None,
                    },
                )
                .await
            }
            NodeKindName::Fanout => Err(WorkflowRuntimeError::InvalidState(format!(
                "nested fanout NodeExecution '{node_execution_id}' cannot be retried"
            ))),
        }
    }

    async fn start_fanout_children<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        let activation_execution_id = {
            let executions = self.executions.lock().await;
            find_by_worktree(&executions, worktree_path)
                .map(|(execution_id, _)| execution_id.clone())
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string()))?
        };
        let activation_gate = self.runtime_activation_gate(&activation_execution_id).await;
        let activation_guard = activation_gate.lock.lock().await;
        let workflow_runtime_session::FanoutStartRuntimeInputs {
            mut fanout_start,
            prompt_inputs,
        } = workflow_runtime_session::load_fanout_start_runtime_inputs(
            &self.executions,
            worktree_path,
        )
        .await?;
        if fanout_start.execution_id != activation_execution_id {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "worktree {worktree_path} changed execution before fanout activation"
            )));
        }
        // A resume checkpoint remains available until the copied child facts have been durably
        // committed. Consuming it here would lose confirmed children if prompt/session setup or
        // the child-event append failed and this parent had to be resumed again.
        let fanout_resume_checkpoint = self
            .fanout_resume_checkpoints
            .lock()
            .await
            .get(&fanout_start.execution_id)
            .cloned();
        if let Some(checkpoint) = fanout_resume_checkpoint.as_ref() {
            if checkpoint.parent_node_name != fanout_start.parent_node_name {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "fanout resume checkpoint targets '{}' but current parent is '{}'",
                    checkpoint.parent_node_name, fanout_start.parent_node_name
                )));
            }
            if checkpoint.children.iter().any(|confirmed| {
                !fanout_start.children.iter().any(|child| {
                    confirmed.node_name == child.node.name
                        && confirmed.item_index == child.item_index
                        && confirmed.child_index == child.child_index
                })
            }) {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "fanout resume checkpoint for '{}' does not match the workflow snapshot",
                    fanout_start.parent_node_name
                )));
            }
            for child in &mut fanout_start.children {
                child.reused = checkpoint
                    .children
                    .iter()
                    .find(|confirmed| {
                        confirmed.node_name == child.node.name
                            && confirmed.item_index == child.item_index
                            && confirmed.child_index == child.child_index
                    })
                    .map(|confirmed| confirmed.reusable.clone());
            }
        }
        let workflow_for_facets = {
            let execs = self.executions.lock().await;
            let (_execution_id, exec) =
                find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
                })?;
            exec.workflow.clone()
        };
        let facet_contents = self
            .facet_contents_for_execution(&fanout_start.execution_id, &workflow_for_facets)
            .await?;
        let command_artifacts = workflow_prompt::artifact_values(
            &prompt_inputs.artifacts,
            fanout_start.request.as_deref(),
        );
        let command_schemas = workflow_for_facets.schemas.clone();
        let command_inputs = fanout_start
            .children
            .iter()
            .filter_map(|child| {
                if child.reused.is_some() {
                    return None;
                }
                let command = child.node.command()?;
                Some(CommandExecutionInput {
                    execution_id: fanout_start.execution_id.clone(),
                    node_execution_id: child.node_execution_id.clone(),
                    node_name: child.node.name.clone(),
                    attempt: child.attempt,
                    worktree_path: worktree_path.to_string(),
                    raw_command: Some(workflow_prompt::render_artifact_references(
                        command,
                        &command_artifacts,
                        child.item.as_ref(),
                    )),
                    contract: child.node.artifact.clone(),
                    schemas: command_schemas.clone(),
                    fanout_parent: Some(fanout_start.parent_node_name.clone()),
                    session_id: None,
                })
            })
            .collect::<Vec<_>>();

        let child_session_plans = workflow_runtime_session::prepare_fanout_child_session_plans(
            &fanout_start,
            &prompt_inputs,
            &facet_contents,
            &workflow_for_facets.schemas,
        )?;
        let mut child_setups: Vec<workflow_fanout_runtime::FanoutChildSessionSetup> =
            Vec::with_capacity(child_session_plans.len());
        for plan in child_session_plans {
            let session = match self
                .workflow_agent_sessions
                .prepare_workflow_agent_session(
                    worktree_path,
                    plan.provider,
                    &fanout_start.execution_id,
                    &plan.node_execution_id,
                    &plan.initial_instruction,
                )
                .await
            {
                Ok(session) => session,
                Err(launch_error) => {
                    return match self
                        .rollback_unattached_fanout_sessions(&child_setups)
                        .await
                    {
                        Some(rollback_error) => Err(WorkflowRuntimeError::AgentSession(format!(
                            "{launch_error}; rollback failed: {rollback_error}"
                        ))),
                        None => Err(launch_error),
                    };
                }
            };
            self.session_workflow_refs.lock().await.insert(
                session.id.clone(),
                SessionWorkflowRef {
                    execution_id: fanout_start.execution_id.clone(),
                },
            );
            child_setups.push(workflow_fanout_runtime::FanoutChildSessionSetup {
                node_execution_id: plan.node_execution_id,
                session_id: session.id,
            });
        }

        let timestamp = current_timestamp();
        let (snapshot_before, snapshot) = {
            let mut executions = self.executions.lock().await;
            let execution =
                find_by_worktree_mut(&mut executions, worktree_path).ok_or_else(|| {
                    WorkflowRuntimeError::ExecutionNotFound(worktree_path.to_string())
                })?;
            let snapshot_before = execution.clone();
            workflow_fanout_runtime::apply_fanout_runtime_state(
                execution,
                &fanout_start,
                &child_setups,
                timestamp,
            )?;
            for child in fanout_start
                .children
                .iter()
                .filter(|child| child.reused.is_some())
            {
                record_fanout_child_successful_completion(execution, &child.node.name);
            }
            let snapshot = RuntimeCommitSnapshot::from_execution(execution)?;
            (snapshot_before, snapshot)
        };

        // Child IDs are allocated during expansion, after the parent transition commit. Commit
        // their NodeStarted and SessionAttached facts atomically with the expanded live state
        // before broadcasting or activating any runtime. Workspace queries replay this log, so
        // the first child-visible broadcast must never expose a Session Node whose attachment is
        // still absent from the durable projection.
        if !fanout_start.children.is_empty() {
            let session_ids_by_node_execution = child_setups
                .iter()
                .map(|setup| (setup.node_execution_id.as_str(), setup.session_id.as_str()))
                .collect::<HashMap<_, _>>();
            let mut started_events = Vec::new();
            for child in &fanout_start.children {
                started_events.push(WorkflowEvent::NodeStarted {
                    execution_id: fanout_start.execution_id.clone(),
                    node_execution_id: child.node_execution_id.clone(),
                    node_name: child.node.name.clone(),
                    kind: child.node.kind_name(),
                    attempt: child.attempt,
                    fanout_parent: Some(crate::domain::workflow::FanoutParentRef {
                        parent_node: fanout_start.parent_node_name.clone(),
                        parent_attempt: fanout_start.parent_attempt,
                        item_index: child.item_index,
                        child_index: child.child_index,
                    }),
                    timestamp,
                });
                if let Some(session_id) =
                    session_ids_by_node_execution.get(child.node_execution_id.as_str())
                {
                    started_events.push(WorkflowEvent::SessionAttached {
                        execution_id: fanout_start.execution_id.clone(),
                        node_execution_id: child.node_execution_id.clone(),
                        session_id: (*session_id).to_string(),
                        timestamp,
                    });
                }
            }
            // The live expansion installs every child NodeExecution before reused children are
            // completed. Keep the durable fact order identical so replay sees the same execution
            // counts at each reused child completion boundary.
            for child in &fanout_start.children {
                if let Some(reused) = child.reused.as_ref() {
                    if let Some(display_command) = reused.display_command.clone() {
                        started_events.push(WorkflowEvent::CommandPrepared {
                            execution_id: fanout_start.execution_id.clone(),
                            node_execution_id: child.node_execution_id.clone(),
                            display_command,
                            timestamp,
                        });
                    }
                    if let Some(value) = reused.artifact.clone() {
                        started_events.push(WorkflowEvent::ArtifactProduced {
                            execution_id: fanout_start.execution_id.clone(),
                            node_execution_id: child.node_execution_id.clone(),
                            node_name: child.node.name.clone(),
                            contract: reused.contract.clone(),
                            value,
                            request_id: None,
                            submitted_at: None,
                            timestamp,
                        });
                    }
                    started_events.push(WorkflowEvent::NodeCompleted {
                        execution_id: fanout_start.execution_id.clone(),
                        node_execution_id: child.node_execution_id.clone(),
                        node_name: child.node.name.clone(),
                        attempt: child.attempt,
                        result_summary: reused.result.clone(),
                        // Usage was already accounted by the confirmed prior child attempt.
                        token_usage: None,
                        timestamp,
                    });
                }
            }
            let execution_store_snapshot_before = self
                .execution_store
                .active_execution_snapshot(&fanout_start.execution_id)
                .await;
            if let Err(failure) = self
                .commit_required_events_with_phase(
                    app,
                    RequiredEventCommit {
                        operation_kind: CommitOperationKind::Workflow,
                        execution_id: &fanout_start.execution_id,
                        snapshot_for_commit: &snapshot,
                        snapshot_before: snapshot_before.clone(),
                        execution_store_snapshot_before,
                        required_events: started_events,
                        append_error_context: "fanout child start event append failed",
                    },
                )
                .await
            {
                return match failure {
                    RequiredEventCommitFailure::BeforeDurableAppend(error) => {
                        match self
                            .rollback_unattached_fanout_sessions(&child_setups)
                            .await
                        {
                            Some(rollback_error) => Err(WorkflowRuntimeError::AgentSession(
                                format!("{error}; rollback failed: {rollback_error}"),
                            )),
                            None => Err(error),
                        }
                    }
                    #[cfg(test)]
                    RequiredEventCommitFailure::AfterDurableAppend(error) => Err(error),
                };
            }
            if fanout_resume_checkpoint.is_some() {
                self.fanout_resume_checkpoints
                    .lock()
                    .await
                    .remove(&fanout_start.execution_id);
            }
        }

        let all_children_completed = {
            let executions = self.executions.lock().await;
            executions
                .get(&fanout_start.execution_id)
                .and_then(|execution| execution.fanout_runtime.as_ref())
                .is_some_and(|fanout| {
                    fanout
                        .children
                        .iter()
                        .all(|child| child.state == FanoutChildRuntimeState::Completed)
                })
        };
        if fanout_start.children.is_empty() || all_children_completed {
            // No child runtime remains to activate. Release the activation lock before completing
            // the parent because its outcome may synchronously start the next node.
            drop(activation_guard);
            drop(activation_gate);
            let completion = {
                let mut executions = self.executions.lock().await;
                let execution =
                    executions
                        .get_mut(&fanout_start.execution_id)
                        .ok_or_else(|| {
                            WorkflowRuntimeError::ExecutionNotFound(
                                fanout_start.execution_id.clone(),
                            )
                        })?;
                let snapshot_before_parent_completion = execution.clone();
                complete_fanout_parent_after_all_children(
                    execution,
                    snapshot_before_parent_completion,
                    Vec::new(),
                    true,
                    None,
                )?
            };
            if let Some(outcome) = completion.outcome {
                self.commit_required_fanout_progress_events_and_execute_outcome(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    CommitOperationKind::Workflow,
                    outcome,
                    completion.snapshot_before,
                    completion.progress_events,
                    completion.failure_telemetry,
                )
                .await?;
            }
            return Ok(());
        }

        workflow_runtime_session::broadcast_state(app, worktree_path, snapshot).await;
        for setup in &child_setups {
            if let Err(error) = run_runtime_activation(
                &activation_gate,
                &fanout_start.execution_id,
                "fanout",
                self.workflow_agent_sessions
                    .activate_workflow_agent_session(&setup.session_id, &setup.node_execution_id),
            )
            .await
            {
                self.settle_runtime_failure_for_node(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    &fanout_start.execution_id,
                    &setup.node_execution_id,
                    &error,
                )
                .await?;
                log::warn!(
                    "workflow {}: fanout NodeExecution '{}' failed to activate: {error}",
                    fanout_start.execution_id,
                    setup.node_execution_id
                );
            }
        }

        for input in command_inputs {
            let node_execution_id = input.node_execution_id.clone();
            if let Err(error) = self
                .spawn_command_execution(app, session_store, agent_runtime, input)
                .await
            {
                self.settle_runtime_failure_for_node(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    &fanout_start.execution_id,
                    &node_execution_id,
                    &error,
                )
                .await?;
                log::warn!(
                    "workflow {}: fanout Command NodeExecution '{}' failed to activate: {error}",
                    fanout_start.execution_id,
                    node_execution_id
                );
            }
        }

        Ok(())
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
        #[cfg(test)]
        if self
            .fail_next_required_event_append
            .swap(false, Ordering::AcqRel)
        {
            return Err("injected required event append failure".to_string());
        }
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
        #[cfg(test)]
        if self
            .fail_next_required_event_append
            .swap(false, Ordering::AcqRel)
        {
            return Err("injected required event append failure".to_string());
        }
        workflow_event_log_writer::append_required_events_with_mutations_for_app_as(
            app,
            operation_kind,
            events,
            mutations,
        )
    }

    #[cfg(test)]
    async fn handle_approval_with_output_for_test(
        &self,
        worktree_path: &str,
        expected_execution_id: Option<&str>,
        expected_node_name: Option<&str>,
    ) -> Result<NodeOutcome, WorkflowRuntimeError> {
        let execution_id = {
            let execs = self.executions.lock().await;
            let (execution_id, _) = find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                WorkflowRuntimeError::UnauthorizedWorktree(worktree_path.to_string())
            })?;
            execution_id.clone()
        };
        self.handle_approval_with_output_for_execution_for_test(
            &execution_id,
            expected_execution_id,
            expected_node_name,
        )
        .await
    }

    /// [05] Test-only: 既に `Failed` state に遷移した snapshot に対して
    /// `execute_outcome(NodeOutcome::Persist(snapshot))` を実行する production 経路の
    /// ショートカット。pre-commit append 失敗時に ExecutionStore / state が persist されない
    /// ことを検証するために用いる（spec [05] commit 境界の継承）。
    #[cfg(test)]
    async fn execute_outcome_persist_failed_for_test<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        snapshot: RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError> {
        // テスト helper の snapshot_before は driver.executions の現在状態を採用する。
        // production 経路では call site が mutation 前に capture するが、本 helper は
        // 既に mutated snapshot を直接渡すための短絡として、現在状態を rollback target
        // 扱いにする（pre-commit 失敗時の挙動を観測する用途のため）。
        let snapshot_before = {
            let execs = self.executions.lock().await;
            execs.get(&snapshot.execution_id).cloned().ok_or_else(|| {
                WorkflowRuntimeError::ExecutionNotFound(snapshot.execution_id.clone())
            })?
        };
        self.execute_outcome(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            NodeOutcome::Persist(snapshot),
            snapshot_before,
        )
        .await
    }

    #[cfg(test)]
    async fn handle_approval_with_output_for_execution_for_test(
        &self,
        execution_id: &str,
        expected_execution_id: Option<&str>,
        expected_node_name: Option<&str>,
    ) -> Result<NodeOutcome, WorkflowRuntimeError> {
        {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            workflow_approval_runtime::validate_approval_target_snapshot(
                exec,
                expected_execution_id,
                expected_node_name,
            )?;
        }

        workflow_approval_runtime::validate_approve_comment(None)?;
        workflow_approval_runtime::validate_approval_turn_phase(None)?;

        let contract = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
            exec.workflow.nodes[exec.current_node_index]
                .artifact
                .clone()
        };

        let artifact = None;
        let application_contract = contract;
        let effective_result = "approve".to_string();

        let mut execs = self.executions.lock().await;
        let exec = execs
            .get_mut(execution_id)
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        workflow_approval_runtime::validate_approval_target_snapshot(
            exec,
            expected_execution_id,
            expected_node_name,
        )?;
        Self::apply_approval_application(
            exec,
            workflow_transition::ApprovalApplication {
                effective_result,
                artifact,
                contract: application_contract,
            },
        )
    }

    #[cfg(test)]
    async fn execute_outcome_persist_auto_approve_for_test(
        &self,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<Option<NodeOutcome>, WorkflowRuntimeError> {
        if let Some((execution_id, node_name)) =
            workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(snapshot, true)
        {
            self.handle_approval_with_output_for_test(
                worktree_path,
                Some(&execution_id),
                Some(&node_name),
            )
            .await
            .map(Some)
        } else {
            Ok(None)
        }
    }

    #[cfg(test)]
    pub(crate) async fn insert_test_approval_execution(
        &self,
        worktree_path: &str,
        current_session_id: &str,
        state: RuntimeExecutionState,
    ) -> RuntimeCommitSnapshot {
        let workflow = WorkflowDefinition {
            name: "test-approval-workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            schemas: Default::default(),
            nodes: vec![NodeDefinition {
                name: "implementation_fix_policy".to_string(),
                kind: NodeKind::Session(SessionSpec {
                    gate: SessionGate::Approval,
                    facets: FacetRefs {
                        instruction: Some("Review fix policy".to_string()),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                artifact: Some("approved-fix-policy".to_string()),
                rules: vec![],
                ..Default::default()
            }],
        };
        let node_status = if matches!(state, RuntimeExecutionState::WaitingApproval) {
            NodeExecutionStatus::WaitingApproval
        } else {
            NodeExecutionStatus::Running
        };
        let exec = crate::adaptor::gateway::workflow::workflow_host::execution_state::domain_workflow_execution! {
            id: "exec-approval-chat".to_string(),
            workflow,
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(state),
            current_node_index: 0,
            node_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            loop_guard_reset_baselines: Default::default(),
            node_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(current_session_id.to_string()),
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "node-exec-approval".to_string(),
                execution_id: "exec-approval-chat".to_string(),
                node_name: "implementation_fix_policy".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: node_status,
                session_id: Some(current_session_id.to_string()),
                display_command: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                completion_signals: Default::default(),
                started_at: 1000.0,
                completed_at: None,
            }],
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
            worktree_path: worktree_path.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            error_reason: None,
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let snapshot =
            RuntimeCommitSnapshot::from_execution(&exec).expect("commit snapshot must be valid");
        let execution_id = exec.id.clone();
        self.executions
            .lock()
            .await
            .insert(execution_id.clone(), exec);
        self.session_workflow_refs.lock().await.insert(
            current_session_id.to_string(),
            SessionWorkflowRef { execution_id },
        );
        snapshot
    }
}

#[cfg(test)]
mod tests;
