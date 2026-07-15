use std::collections::{BTreeMap, HashMap};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use super::runtime_session as workflow_runtime_session;
#[cfg(test)]
use super::runtime_session::resolve_step_model_with_registry;
#[cfg(test)]
use super::step_session_boundary::StepSessionInfo;
#[cfg(test)]
use super::step_session_boundary::{dispatch_session_start, SessionStartGate};
use super::step_session_boundary::{RealStepSessionDeps, StepSessionDeps};
use crate::adaptor::gateway::workflow::approval_runtime as workflow_approval_runtime;
use crate::adaptor::gateway::workflow::domain_mapping::{
    node_kind_to_domain, workflow_schemas_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::engine_start_guard as workflow_engine_start_guard;
use crate::adaptor::gateway::workflow::event::{ContractViolationRecord, WorkflowEvent};
use crate::adaptor::gateway::workflow::event_log_writer as workflow_event_log_writer;
use crate::adaptor::gateway::workflow::execution_registry::{
    find_any_by_worktree, find_by_worktree, find_by_worktree_mut, ExecutionStateTarget,
};
use crate::adaptor::gateway::workflow::execution_store::{
    ExecutionOrigin, ExecutionStatus, ExecutionStore, ExecutionStoreError, TerminalExecutionStatus,
    WorkflowExecutionMetadata,
};
use crate::adaptor::gateway::workflow::facet::WorkflowFacetContents;
use crate::adaptor::gateway::workflow::failure_wire::{
    self as workflow_failure_wire, SubmissionViolation,
};
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::adaptor::gateway::workflow::orphan_recovery as workflow_orphan_recovery;
use crate::adaptor::gateway::workflow::output_limit as workflow_output_limit;
use crate::adaptor::gateway::workflow::output_submission as workflow_output_submission;
use crate::adaptor::gateway::workflow::parallel_runtime as workflow_parallel_runtime;
use crate::adaptor::gateway::workflow::prompt_rendering as workflow_prompt;
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolver, WorkflowDefinitionResolver,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};
use crate::adaptor::gateway::workflow::runtime_commit::{
    self as workflow_runtime_commit, AbortOutcome, AbortTargetLookup, CommandMutationRollback,
    RequiredEventCommit, StepOutcome,
};
use crate::adaptor::gateway::workflow::runtime_events as workflow_runtime_events;
#[cfg(test)]
use crate::adaptor::gateway::workflow::runtime_state::NextStepDecision;
#[cfg(test)]
use crate::adaptor::gateway::workflow::runtime_state::{FanoutChildRuntime, FanoutRuntimeState};
use crate::adaptor::gateway::workflow::runtime_state::{
    FanoutChildRuntimeState, SessionWorkflowRef, WorkflowExecution,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::NodeDefinition;
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::{
    CommandSpec, FacetRefs, FanoutSpec, NodeKind, SessionGate, SessionSpec,
};
use crate::adaptor::gateway::workflow::schema::{NodeKindName, Workflow};
use crate::adaptor::gateway::workflow::secret_source;
#[cfg(test)]
use crate::adaptor::gateway::workflow::state::NodeHistoryEntry;
use crate::adaptor::gateway::workflow::state::{
    NodeExecution, NodeExecutionStatus, NodeStallObservation, RuntimeArtifact,
    RuntimeExecutionState, TokenUsage, WorkflowState,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::step_settings::resolve_step_settings;
#[cfg(test)]
use crate::adaptor::gateway::workflow::step_settings::ResolvedStepSettings;
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
#[cfg(test)]
use crate::adaptor::gateway::workflow::turn_completion;
use crate::domain::agent_session::PermissionMode;
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::contract_schema as workflow_contract_schema;
use crate::domain::workflow::services::failure_policy::{
    RepairDecision, RetryPolicy, StructuredOutputRepairPolicy,
};
use crate::domain::workflow::services::history::RuntimeStartFailureKind;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::spec_directory as workflow_spec_directory;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::OutcomeCommitMode;
use crate::domain::workflow::WorkflowNodeContext;
use crate::domain::workflow::{
    ContractValidationResult, FailureClassification, FailureDisposition, NodeExecutionFailureKind,
    SchemaDef as DomainSchemaDef, NODE_STATUS_FAILED, NODE_STATUS_INTERRUPTED,
};
use crate::infrastructure::process::command_runner::{
    self as workflow_command_runner, ActiveCommandHandle, CommandRunOutput, CommandRunnerError,
};
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{OpenTabRegistry, SessionStore};
use crate::usecase::agent_session::status::current_timestamp;

fn parallel_child_failure_kind(
    exit_code: i64,
    failure_signal: Option<workflow_transition::SessionFailureSignal>,
) -> NodeExecutionFailureKind {
    workflow_transition::classify_session_error(exit_code, failure_signal)
}

fn record_failed_snapshot_telemetry(snapshot: &WorkflowState) {
    if let RuntimeExecutionState::Failed {
        kind, retry_count, ..
    } = &snapshot.state
    {
        crate::other::telemetry::record_workflow_node_failure(
            FailureClassification::with_disposition(*kind, FailureDisposition::Terminal),
            *retry_count,
        );
    }
}

#[cfg(test)]
struct TestWorkflowDefinitionResolver;

#[cfg(test)]
#[async_trait::async_trait]
impl WorkflowDefinitionResolver for TestWorkflowDefinitionResolver {
    async fn resolve(
        &self,
        workflow_name: &str,
    ) -> Result<Workflow, WorkflowDefinitionResolverError> {
        let workflow_name = workflow_name.to_string();
        tokio::task::spawn_blocking(move || {
            let dir = crate::adaptor::gateway::workflow::storage::workflows_dir();
            let facets_base = crate::adaptor::gateway::workflow::facet::facets_base_dir();
            super::runtime_resolver::resolve_workflow_by_name(&dir, &facets_base, &workflow_name)
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

/// ワークフローのステップを順次実行するステートマシンエンジン。
#[derive(Clone)]
pub struct WorkflowRuntimeService {
    /// `execution_id` → `WorkflowExecution` の in-memory マッピング。
    /// HashMap キーは `WorkflowExecution.id`（= `execution_id`）と一致する。
    /// `worktree_path` は `WorkflowExecution.worktree_path` 属性として保持し、
    /// `worktree_path → execution_id` の補助解決は Execution Store の secondary index 経由で行う。
    executions: Arc<Mutex<HashMap<String, WorkflowExecution>>>,
    /// session_id（親・ステップ・並列子） → SessionWorkflowRef のマッピング
    session_workflow_refs: Arc<Mutex<HashMap<String, SessionWorkflowRef>>>,
    /// execution_id → 解決済み facet 本文。workflow state / event には含めない runtime-local read model。
    execution_facet_contents: Arc<Mutex<HashMap<String, WorkflowFacetContents>>>,
    /// node_execution_id → active command process shutdown handle.
    active_commands: Arc<Mutex<HashMap<String, ActiveCommandHandle>>>,
    /// node_execution_id → owning workflow execution_id.
    active_command_executions: Arc<Mutex<HashMap<String, String>>>,
    /// node_execution_id → command completion observer task owned by this workflow runtime.
    command_completion_observers: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
    /// node_execution_id → shutdown reason consumed by the completion observer.
    command_shutdown_reasons: Arc<Mutex<HashMap<String, ActiveCommandShutdownReason>>>,
    /// active な WorkflowExecutionMetadata の管理および execution metadata の永続化を担う Execution Store。
    /// worktree_path → active execution_id の secondary index は Execution Store 内で保持する。
    execution_store: Arc<ExecutionStore>,
    workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
    worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    open_tabs: Arc<OpenTabRegistry>,
    #[cfg(test)]
    fail_next_required_event_append: Arc<AtomicBool>,
    #[cfg(test)]
    abort_after_lookup_gate: AbortAfterLookupGate,
}

struct FanoutChildStartedLogObserver<'a, R: tauri::Runtime> {
    engine: &'a WorkflowRuntimeService,
    app: &'a tauri::AppHandle<R>,
    execution_id: &'a str,
}

struct FanoutChildCompletionCommit {
    all_completed: bool,
    outcome: Option<StepOutcome>,
    snapshot_before: WorkflowExecution,
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
    terminal_reason: String,
    failure_kind: NodeExecutionFailureKind,
    retry_count: Option<u32>,
    timestamp: f64,
    record_child_token_usage: bool,
}

struct WorkflowExecutionInsert {
    execution_id: String,
    workflow: Workflow,
    worktree_path: String,
    request: Option<String>,
    created_from: ExecutionOrigin,
    workflow_defaults: WorkflowDefaults,
    now: f64,
}

#[derive(Clone)]
struct CommandExecutionInput {
    execution_id: String,
    node_execution_id: String,
    node_name: String,
    attempt: u32,
    worktree_path: String,
    command: String,
    contract: Option<String>,
    schemas: BTreeMap<String, DomainSchemaDef>,
    fanout_parent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveCommandShutdownReason {
    AppExit,
}

struct CommandArtifact {
    value: serde_json::Value,
    event_contract: Option<String>,
    result_summary: String,
}

fn command_env(input: &CommandExecutionInput) -> Vec<(String, String)> {
    vec![
        (
            "RELEASH_WORKFLOW_EXECUTION_ID".to_string(),
            input.execution_id.clone(),
        ),
        (
            "RELEASH_NODE_EXECUTION_ID".to_string(),
            input.node_execution_id.clone(),
        ),
    ]
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
                let violations =
                    workflow_spec_directory::validate_contract_value(contract, &artifact);
                if violations.is_empty() {
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

fn is_still_current_execution(exec: &WorkflowExecution, node_name: &str, attempt: u32) -> bool {
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

fn resolve_active_node_execution_index(
    exec: &WorkflowExecution,
    node_name: &str,
    node_execution_id: Option<&str>,
) -> Result<usize, WorkflowEngineError> {
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
                WorkflowEngineError::InvalidState(format!(
                    "active node execution '{node_execution_id}' for node '{node_name}' was not found"
                ))
            });
    }
    match candidates.as_slice() {
        [(index, _)] => Ok(*index),
        [] => Err(WorkflowEngineError::InvalidState(format!(
            "node '{node_name}' has no active execution"
        ))),
        candidates => {
            let candidate_ids = candidates
                .iter()
                .map(|(_, execution)| execution.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(WorkflowEngineError::InvalidState(format!(
                "node '{node_name}' has {} active executions; node_execution_id is required; candidates: [{candidate_ids}]",
                candidates.len()
            )))
        }
    }
}

fn resolve_fanout_approval_node_execution_id(
    exec: &WorkflowExecution,
    node_name: &str,
    node_execution_id: Option<&str>,
) -> Result<Option<String>, WorkflowEngineError> {
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
        return Err(WorkflowEngineError::InvalidState(format!(
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
    exec: &mut WorkflowExecution,
    snapshot_before: WorkflowExecution,
    mut progress_events: Vec<WorkflowEvent>,
    required_progress_events: bool,
    failure_telemetry: Option<FailureClassification>,
) -> Result<FanoutChildCompletionCommit, WorkflowEngineError> {
    let Some(parallel_run) = exec.parallel_run.as_ref() else {
        return Err(WorkflowEngineError::InvalidState(
            "fanout parent completion requires an active fanout runtime".to_string(),
        ));
    };
    let parent_node_name = parallel_run.parent_node_name.clone();
    let parent_node_execution_id = parallel_run.parent_node_execution_id.clone();
    let parent_attempt = exec
        .node_execution_counts
        .get(&parent_node_name)
        .copied()
        .unwrap_or(1);
    let completed_at = current_timestamp();
    let completion_plan = workflow_parallel_runtime::plan_fanout_parent_completion(
        &parent_node_name,
        parent_attempt,
        &parallel_run.children,
        completed_at,
    );
    let parent_artifact = completion_plan
        .parent_step_output
        .artifact
        .clone()
        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
    let parent_token_usage = completion_plan.parent_step_output.token_usage.clone();
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

    exec.parallel_run = None;
    exec.updated_at = completed_at;
    exec.complete_node_execution(
        &parent_node_execution_id,
        Some(parent_artifact),
        parent_token_usage.clone(),
        completed_at,
    );
    exec.artifacts
        .insert(parent_node_name.clone(), completion_plan.parent_step_output);
    exec.current_step_token_usage = TokenUsage::default();
    exec.current_session_id = None;
    exec.node_history.push(completion_plan.history_entry);

    let outcome = exec.apply_advance();

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
    exec: &mut WorkflowExecution,
    snapshot_before: WorkflowExecution,
    progress_events: Vec<WorkflowEvent>,
    required_progress_events: bool,
    failure_telemetry: Option<FailureClassification>,
) -> Result<FanoutChildCompletionCommit, WorkflowEngineError> {
    let Some(parallel_run) = exec.parallel_run.as_ref() else {
        return Err(WorkflowEngineError::InvalidState(
            "fanout child terminal state requires an active fanout runtime".to_string(),
        ));
    };
    let all_done = parallel_run.children.iter().all(|c| {
        matches!(
            c.state,
            FanoutChildRuntimeState::Completed | FanoutChildRuntimeState::Failed
        )
    });

    if !all_done {
        exec.updated_at = current_timestamp();
        let snapshot = exec.to_workflow_state();
        return Ok(FanoutChildCompletionCommit {
            all_completed: false,
            outcome: Some(StepOutcome::Persist(snapshot)),
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

fn finalize_fanout_child_failure_state(
    exec: &mut WorkflowExecution,
    snapshot_before: WorkflowExecution,
    input: FanoutChildFailureInput,
) -> Result<FanoutChildFailureCommit, WorkflowEngineError> {
    let execution_id = exec.id.clone();
    let failure_kind = input.failure_kind;
    let retry_count = input.retry_count;
    let timestamp = input.timestamp;
    let (
        parent_node_execution_id,
        child_name,
        child_attempt,
        child_token_usage,
        interrupted_session_ids,
        interrupted_command_ids,
        interrupted_execution_ids,
    ) = {
        let Some(parallel_run) = exec.parallel_run.as_mut() else {
            return Err(WorkflowEngineError::InvalidState(
                "fanout child failure requires an active fanout runtime".to_string(),
            ));
        };
        let Some(child_index) = parallel_run
            .children
            .iter()
            .position(|child| child.node_execution_id == input.child_node_execution_id)
        else {
            return Err(WorkflowEngineError::InvalidState(format!(
                "fanout child failure references unknown child '{}'",
                input.child_node_execution_id
            )));
        };
        let parent_node_execution_id = parallel_run.parent_node_execution_id.clone();
        let mut child_name = String::new();
        let mut child_attempt = 1;
        let mut child_token_usage = None;
        let mut interrupted_session_ids = Vec::new();
        let mut interrupted_command_ids = Vec::new();
        let mut interrupted_execution_ids = Vec::new();

        for (index, child) in parallel_run.children.iter_mut().enumerate() {
            if index == child_index {
                child.state = FanoutChildRuntimeState::Failed;
                child.result = Some(failure_kind.as_str().to_string());
                child.artifact = None;
                child.failure_kind = Some(failure_kind);
                child.failure_disposition = None;
                child.completed_at = Some(timestamp);
                child_name = child.node_name.clone();
                child_attempt = child.attempt;
                if input.record_child_token_usage {
                    child_token_usage = Some(child.token_usage.clone());
                }
                continue;
            }
            if child.state != FanoutChildRuntimeState::Running {
                continue;
            }
            child.state = FanoutChildRuntimeState::Interrupted;
            child.completed_at = Some(timestamp);
            interrupted_execution_ids.push(child.node_execution_id.clone());
            if child.session_id.is_empty() {
                interrupted_command_ids.push(child.node_execution_id.clone());
            } else {
                interrupted_session_ids.push(child.session_id.clone());
            }
        }

        (
            parent_node_execution_id,
            child_name,
            child_attempt,
            child_token_usage,
            interrupted_session_ids,
            interrupted_command_ids,
            interrupted_execution_ids,
        )
    };

    exec.fail_node_execution(
        &input.child_node_execution_id,
        input.child_failure_reason.clone(),
        failure_kind,
        timestamp,
    );
    if let Some(child_token_usage) = child_token_usage {
        if let Some(node_execution) = exec
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == input.child_node_execution_id)
        {
            node_execution.token_usage = Some(child_token_usage);
        }
    }
    for execution_id in interrupted_execution_ids {
        if let Some(node_execution) = exec
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == execution_id)
        {
            node_execution.status = NodeExecutionStatus::Aborted;
            node_execution.completed_at = Some(timestamp);
        }
    }
    exec.fail_node_execution(
        &parent_node_execution_id,
        input.terminal_reason.clone(),
        failure_kind,
        timestamp,
    );
    exec.current_stall_observations.clear();
    exec.state = RuntimeExecutionState::Failed {
        reason: input.terminal_reason.clone(),
        kind: failure_kind,
        retry_count,
    };
    let history_entry =
        exec.make_node_history_entry(Some(input.terminal_reason.clone()), None, None);
    exec.node_history.push(history_entry);
    exec.parallel_run = None;
    exec.updated_at = timestamp;
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
            all_completed: true,
            outcome: Some(StepOutcome::Persist(exec.to_workflow_state())),
            snapshot_before,
            progress_events,
            required_progress_events: true,
            failure_telemetry: Some(FailureClassification::with_disposition(
                failure_kind,
                FailureDisposition::Terminal,
            )),
        },
        interrupted_session_ids,
        interrupted_command_ids,
    })
}

impl<R: tauri::Runtime> workflow_runtime_session::FanoutChildTurnObserver
    for FanoutChildStartedLogObserver<'_, R>
{
    fn child_turn_started(&self, started: workflow_runtime_session::FanoutChildStartedRuntime<'_>) {
        self.engine.write_log(
            self.app,
            WorkflowEvent::SessionAttached {
                execution_id: self.execution_id.to_string(),
                node_execution_id: started.node_execution_id.to_string(),
                session_id: started.session_id.to_string(),
                timestamp: current_timestamp(),
            },
        );
    }
}

fn current_step_for_stall_observation(
    exec: &WorkflowExecution,
    session_id: &str,
) -> Option<(String, String, u32)> {
    if let Some(parallel_run) = exec.parallel_run.as_ref() {
        if let Some(child) = parallel_run.children.iter().find(|child| {
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

fn clear_stall_observations_for_session(
    observations: &mut Vec<NodeStallObservation>,
    session_id: &str,
) -> bool {
    let before = observations.len();
    observations.retain(|observation| observation.session_id != session_id);
    observations.len() != before
}

fn upsert_stall_observation(
    observations: &mut Vec<NodeStallObservation>,
    observation: NodeStallObservation,
) {
    clear_stall_observations_for_session(observations, &observation.session_id);
    observations.push(observation);
}

// [08] `lookup_step_contract` は domain の contract service に移動済み。
// engine と CLI の双方が同じ domain service を参照するため、本モジュールではメモのみ残す。

impl WorkflowRuntimeService {
    pub(crate) fn new(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
        branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
        open_tabs: Arc<OpenTabRegistry>,
    ) -> Self {
        Self {
            executions: Arc::new(Mutex::new(HashMap::new())),
            session_workflow_refs: Arc::new(Mutex::new(HashMap::new())),
            execution_facet_contents: Arc::new(Mutex::new(HashMap::new())),
            active_commands: Arc::new(Mutex::new(HashMap::new())),
            active_command_executions: Arc::new(Mutex::new(HashMap::new())),
            command_completion_observers: Arc::new(Mutex::new(HashMap::new())),
            command_shutdown_reasons: Arc::new(Mutex::new(HashMap::new())),
            execution_store: Arc::new(ExecutionStore::new()),
            workflow_resolver,
            worktree_resolver,
            branch_diff_context,
            open_tabs,
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
            None,
            Arc::new(OpenTabRegistry::default()),
        )
    }

    #[cfg(test)]
    pub(crate) async fn seed_active_execution_for_test(
        &self,
        execution_id: String,
        workflow: Workflow,
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
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .unwrap();
        if let Some(data_dir) = self.execution_store.data_dir_for_test().await {
            let event_log = WorkflowEventLog::new(&data_dir);
            event_log
                .append(&WorkflowEvent::ExecutionStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow.name.clone(),
                    worktree_path: worktree_path.clone(),
                    created_from,
                    request: String::new(),
                    definition: workflow.clone(),
                    timestamp: now,
                })
                .unwrap();
            event_log
                .append(&WorkflowEvent::NodeStarted {
                    execution_id: execution_id.clone(),
                    node_execution_id: node_execution_id.clone(),
                    node_name: current_node.clone(),
                    kind: current_node_kind,
                    attempt: 1,
                    fanout_parent: None,
                    timestamp: now,
                })
                .unwrap();
        }
        self.executions.lock().await.insert(
            execution_id.clone(),
            WorkflowExecution {
                id: execution_id.clone(),
                workflow,
                state,
                current_node_index: 0,
                node_execution_counts: HashMap::from([(current_node.clone(), 1)]),
                node_history: Vec::new(),
                workflow_defaults: WorkflowDefaults {
                    backend_id: None,
                    permission_mode: crate::domain::agent_session::PermissionMode::EDIT.to_string(),
                },
                worktree_path,
                created_from,
                error_reason: None,
                started_at: now,
                updated_at: now,
                current_session_id: None,
                current_step_token_usage: TokenUsage::default(),
                artifacts: HashMap::new(),
                node_executions: vec![NodeExecution {
                    id: node_execution_id,
                    execution_id: execution_id,
                    node_name: current_node,
                    kind: current_node_kind,
                    attempt: 1,
                    status: node_execution_status,
                    session_id: None,
                    artifact: None,
                    token_usage: None,
                    failure: None,
                    fanout_parent: None,
                    started_at: now,
                    completed_at: None,
                }],
                request: None,
                parallel_run: None,
                current_stall_observations: Vec::new(),
            },
        );
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
        workflow: &Workflow,
        worktree_path: &str,
        _request: Option<String>,
        created_from: ExecutionOrigin,
        now: f64,
    ) -> Result<String, WorkflowEngineError> {
        let execution_id = uuid::Uuid::new_v4().to_string();
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
                total_token_usage: crate::domain::workflow::TokenUsage::default(),
            })
            .await
            .map_err(|e| match e {
                ExecutionStoreError::WorktreeAlreadyActive { .. } => {
                    WorkflowEngineError::AlreadyActive(workflow.name.clone())
                }
                other => WorkflowEngineError::SessionStore(format!(
                    "ExecutionStore register failed: {other}"
                )),
            })?;
        Ok(execution_id)
    }

    fn resolve_facet_contents_for_workflow(
        workflow: &Workflow,
    ) -> Result<WorkflowFacetContents, WorkflowEngineError> {
        crate::adaptor::gateway::workflow::storage::resolve_and_validate_workflow_facets(
            workflow,
            &crate::adaptor::gateway::workflow::facet::facets_base_dir(),
        )
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))
    }

    async fn facet_contents_for_execution(
        &self,
        execution_id: &str,
        workflow: &Workflow,
    ) -> Result<WorkflowFacetContents, WorkflowEngineError> {
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
    ) -> Result<WorkflowState, WorkflowEngineError> {
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
            workflow_prompt::request_step_output(&request_text, now),
        );
        let mut execution = WorkflowExecution {
            id: execution_id.clone(),
            workflow: workflow.clone(),
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            node_execution_counts: HashMap::new(),
            node_history: Vec::new(),
            workflow_defaults,
            created_from,
            error_reason: None,
            started_at: now,
            updated_at: now,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            artifacts,
            node_executions: Vec::new(),
            request,
            parallel_run: None,
            current_stall_observations: Vec::new(),
            worktree_path: worktree_path.clone(),
        };

        let node_name = workflow.nodes[0].name.clone();
        let mut execs = self.executions.lock().await;
        WorkflowExecution::validate_start(&workflow, find_any_by_worktree(&execs, &worktree_path))?;
        execution.node_execution_counts.insert(node_name.clone(), 1);
        execution.node_executions.push(NodeExecution {
            id: uuid::Uuid::new_v4().to_string(),
            execution_id: execution_id.clone(),
            node_name,
            kind: workflow.nodes[0].kind_name(),
            attempt: 1,
            status: NodeExecutionStatus::Running,
            session_id: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent: None,
            started_at: now,
            completed_at: None,
        });
        execs.insert(execution_id.clone(), execution);
        Ok(execs.get(&execution_id).unwrap().to_workflow_state())
    }

    #[cfg(test)]
    async fn start_workflow_common_core_for_test(
        &self,
        workflow: Workflow,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        now: f64,
    ) -> Result<String, WorkflowEngineError> {
        workflow_engine_start_guard::validate_workflow_shape(&workflow)?;
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

    /// execution_id から worktree_path を解決する。active な execution のみならず、終了済み execution も
    /// `workflow_executions/{execution_id}.json` から metadata を読み込んで返す。
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
    ) -> Vec<crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionSummary> {
        self.execution_store
            .list_executions(crate::adaptor::gateway::workflow::execution_store::ExecutionListFilter {
                status: Some(crate::adaptor::gateway::workflow::execution_store::ExecutionStatusFilter::Active),
                worktree_path: None,
            })
            .await
    }

    /// テスト専用 facade: terminal な execution 一覧を取得する。
    #[cfg(test)]
    pub async fn list_completed_executions(
        &self,
    ) -> Vec<crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionSummary> {
        self.execution_store
            .list_executions(crate::adaptor::gateway::workflow::execution_store::ExecutionListFilter {
                status: Some(crate::adaptor::gateway::workflow::execution_store::ExecutionStatusFilter::Terminal),
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
    ) -> Option<crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionSummary> {
        self.execution_store.get_execution(execution_id).await
    }

    /// Execution Store の永続化ディレクトリを設定する（アプリ起動時の setup から呼ぶ）。
    pub async fn set_execution_store_data_dir(&self, dir: std::path::PathBuf) {
        self.execution_store.set_data_dir(dir).await;
    }

    /// 起動時 recovery: 前回プロセスが terminal event を書かないまま終了した execution（metadata の
    /// status が non-terminal なまま残った execution）を、Aborted へ強制遷移させる。
    /// 既存 `event_projection` の `ExecutionAborted → Aborted` 判定をそのまま機能させるため、
    /// `<data_dir>/workflow_execution_logs/<execution_id>.ndjson` 末尾に `ExecutionAborted` event を append し、
    /// `workflow_executions/<execution_id>.json` の status を Aborted に更新する。
    ///
    /// 本メソッドは `set_execution_store_data_dir` 直後（in-memory `executions` map が空の状態）に
    /// 1 度だけ呼ばれる前提。append / persist が個別に失敗しても起動自体は止めない（warn
    /// のみ）。metadata の更新失敗時は次回起動で再試行される（idempotent）。
    pub async fn recover_orphan_executions<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        let orphans = self.execution_store.list_non_terminal_metadata().await;
        if orphans.is_empty() {
            return;
        }
        let recovery_items =
            workflow_orphan_recovery::orphan_execution_recovery_items(orphans, current_timestamp());
        for item in recovery_items {
            if let Err(e) = self.write_log_required(app, item.event) {
                log::warn!(
                    "recover_orphan_executions: append ExecutionAborted failed for {}: {e}",
                    item.execution_id
                );
                // metadata 更新は次回起動で再試行するため、ここで skip する。
                continue;
            }
            if let Err(e) = self
                .execution_store
                .force_complete_orphan_to_aborted(item.metadata, item.completed_at, None)
                .await
            {
                log::warn!(
                    "recover_orphan_executions: persist metadata failed for {}: {e}",
                    item.execution_id
                );
            }
        }
    }
}

impl WorkflowRuntimeService {
    /// ワークフローを開始する。
    /// ChatSessionは既に作成済みの前提で、最初のステップのプロンプトを送信する。
    ///
    /// 戻り値は新しく払い出された `execution_id`。
    /// `execution_id` を `execution_id` として「昇格」させた値であり、ここ以外で採番されることはない。
    /// state 変化の入口は resolved StartRun port からこの private handler に合流する。
    /// 外部入口としては公開せず、usecase/gateway が解決済み workflow を渡す境界にする。
    #[allow(clippy::too_many_arguments)]
    async fn start_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        workflow: Workflow,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        permission_mode: PermissionMode,
    ) -> Result<String, WorkflowEngineError> {
        // ===== Phase 1: 副作用なしの validation =====
        // parent ChatSession 作成・executions 登録・refs 登録の前で全 validation を実施する。
        // ここで弾けば、リトライ時に「孤立した parent session」「孤立した refs entry」
        // を残さない（Spec issues-1011: 起動順序のアトミック化）。
        //
        // 1) workflow 構造の事前検証（空 nodes などの実行不能形状の拒否）。
        workflow_engine_start_guard::validate_workflow_shape(&workflow)?;
        // 2) model 検証: 各 model から所属 backend を一意に解決する。
        //    registry 未登録自体を InvalidWorkflow として即時失敗にする（検証スキップを避ける）。
        let registry = agent_runtime.backend_registry();
        let workflow_definition =
            crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain(
                &workflow,
            );
        crate::domain::workflow::validation::validate_models(&workflow_definition, |model| {
            registry
                .resolve_model_entry(model)
                .map(|entry| Some(entry.backend))
        })
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
        let facet_contents = Self::resolve_facet_contents_for_workflow(&workflow)?;

        // ===== Phase 2: 副作用（Execution Store reservation 先取り → 親 session 作成 → executions 登録） =====
        // Spec issues-1011 finding 5/8: 並行起動でも parent ChatSession を孤立させないために
        // Execution Store reservation を「最初の副作用」にする。reservation が失敗（同一 worktree
        // への並行起動）した場合は AlreadyActive として返り、他の副作用は走らない。
        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
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
        // 残し、Execution Store と engine の状態スキューを抑える。
        // 撤回 helper は最終的な Result を返し、呼出側で start_workflow の Err に伝播させる。
        let rollback_reservation = |reason: String| async {
            if let Err(rs_err) = self.execution_store.cancel_reservation(&execution_id).await {
                log::warn!(
                    "ExecutionStore cancel_reservation failed during start rollback for {execution_id}: {rs_err}; reason={reason}"
                );
                // fallback として terminal metadata を残す（撤回より優先度低い）。
                if let Err(rs_err2) = self
                    .execution_store
                    .complete_execution(
                        &execution_id,
                        TerminalExecutionStatus::Failed,
                        current_timestamp(),
                        Some(reason),
                    )
                    .await
                {
                    log::warn!(
                        "ExecutionStore complete_execution failed during start rollback fallback for {execution_id}: {rs_err2}"
                    );
                }
            }
        };

        // parent ChatSession 機構撤去後は session を engine が作らない。
        // workflow_defaults は StartRun の permission_mode 引数を workflow 全体の継承
        // デフォルトとして capture する（schema 境界 [02]: 各 step は NodeDefinition.model
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
                definition: workflow.clone(),
                timestamp: now,
            },
            initial_node_started_event,
        ];
        if let Err(e) = self.write_log_required_batch(app, &required_start_events) {
            let mut execs = self.executions.lock().await;
            execs.remove(&execution_id);
            drop(execs);
            self.release_execution_facet_contents(&execution_id).await;
            rollback_reservation(format!("initial workflow event batch failed: {e}")).await;
            return Err(WorkflowEngineError::SessionStore(format!(
                "write initial workflow event batch failed: {e}"
            )));
        }

        // [04] post-commit: broadcast。ExecutionStarted は append 済みのため command は既に受理。
        // session_workflow_refs への登録は step session 起動時（start_step_session /
        // start_fanout_children）で行う。
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;

        let first_step_kind = workflow.nodes[0].kind_name();

        // [04] post-commit: ExecutionStarted append 済みのため start primitive は既に受理。
        //    初回 runtime 起動失敗は Failed 状態遷移として観測し、
        //    start primitive は Ok(execution_id) を返す（spec [04]『command 受理境界』Rule）。
        if let Err(e) = self
            .start_current_node_runtime(app, session_store, agent_runtime, &worktree_path)
            .await
        {
            match first_step_kind {
                NodeKindName::Session => {
                    workflow_runtime_session::record_node_session_start_failed_by_execution_id(
                        &self.executions,
                        &execution_id,
                        &e,
                    )
                    .await;
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            agent_runtime,
                            &worktree_path,
                            workflow_runtime_session::runtime_start_failed_state(
                                RuntimeStartFailureKind::StepSession,
                                &e,
                            ),
                        )
                        .await;
                }
                NodeKindName::Fanout => {
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            agent_runtime,
                            &worktree_path,
                            workflow_runtime_session::runtime_start_failed_state(
                                RuntimeStartFailureKind::ParallelChildren,
                                &e,
                            ),
                        )
                        .await;
                }
                NodeKindName::Command => {
                    let _ = self
                        .fail_current_command_node(
                            app,
                            session_store,
                            agent_runtime,
                            &execution_id,
                            &format!("Failed to start command runtime: {e}"),
                        )
                        .await;
                }
            }
            log::warn!("workflow {execution_id}: post-commit node runtime start failed: {e}");
        }
        Ok(execution_id)
    }

    pub(crate) async fn resolve_start_execution_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowEngineError> {
        self.worktree_resolver
            .resolve(worktree_path)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn resolve_start_execution_workflow(
        &self,
        workflow_name: &str,
    ) -> Result<Workflow, WorkflowEngineError> {
        crate::domain::workflow::validation::validate_name(workflow_name)
            .map_err(|e| WorkflowEngineError::ValidationError(format!("validation_error: {e}")))?;
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
        workflow: Workflow,
        worktree_path: String,
        request: Option<String>,
        created_from: ExecutionOrigin,
        permission_mode: PermissionMode,
    ) -> Result<String, WorkflowEngineError> {
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

    /// [08] node に対する構造化出力提出の単一トランザクション handler。
    ///
    /// 1. execution / node / contract の妥当性検証
    /// 2. `validate_contract_value` で contract 適合判定
    /// 3. 適合時のみ `artifacts` を更新し、
    ///    `ArtifactProduced` event を append
    /// 4. 不適合・stale step・不在 step・契約タイプ不一致は副作用なしで `Err` を返し、
    ///    `artifacts` / event log を一切変更しない。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn submit_workflow_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_name: String,
        node_execution_id: Option<String>,
        contract: String,
        artifact: serde_json::Value,
    ) -> Result<(), WorkflowEngineError> {
        self.handle_submit_output(
            app,
            session_store,
            agent_runtime,
            execution_id,
            node_name,
            node_execution_id,
            contract,
            artifact,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_submit_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_name: String,
        node_execution_id: Option<String>,
        contract: String,
        artifact: serde_json::Value,
    ) -> Result<(), WorkflowEngineError> {
        workflow_output_submission::validate_submit_output_request(
            execution_id,
            &node_name,
            node_execution_id.as_deref(),
            &contract,
        )?;

        // 1. contract 適合判定（pure validator、副作用なし）。ロック取得前に行い、
        //    無効入力は writer lock を取らずに弾く。
        //    [08] 機密値 redaction: caller (CLI / Tauri API) 入力に approve コメントや
        //    secret token が混入していても event log / artifacts に生で残らないよう、
        //    redaction 後の structured output を contract validation に通す。
        //    preflight (workflow_validate_output / CLI cmd_output_validate) と本 submit で
        //    同一の前処理 + validation を共有するため、`preprocess_and_validate_output`
        //    に集約する（spec [08] L169 / Rule 2）。
        let secrets = secret_source::collect_configured_secret_values(app);
        let workflow = {
            let execs = self.executions.lock().await;
            execs
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?
                .workflow
                .clone()
        };
        let workflow_output_submission::ValidatedSubmissionOutput {
            artifact: validated_output,
            result: validated_result,
        } = match workflow_output_submission::validate_submission_output_with_secrets(
            &workflow, &contract, artifact, &secrets,
        ) {
            Ok(validated) => validated,
            Err(validation_error) => {
                return self
                    .handle_invalid_submit_output(
                        app,
                        session_store,
                        agent_runtime,
                        execution_id,
                        &node_name,
                        node_execution_id.as_deref(),
                        &contract,
                        validation_error,
                    )
                    .await;
            }
        };

        // 2. writer lock 取得後に state / contract / accepting target / attempt を
        //    再検証し、snapshot 採取と mutation を同一 lock スコープで行う
        //    （spec [08] 境界: ArtifactProduced の append は適合判定および state 更新と
        //    同一トランザクション境界内。並行 dispatch によって stale step の output が
        //    確定されないよう、validation と mutation のあいだに lock を手放さない）。
        let timestamp = current_timestamp();
        let mutation = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            workflow_output_submission::apply_validated_submission(
                exec,
                execution_id,
                &node_name,
                node_execution_id.as_deref(),
                &contract,
                &validated_output,
                validated_result,
                timestamp,
            )?
        };

        // 3. ArtifactProduced event を append。append 失敗時は state を snapshot から
        //    一括復元することで「validation・state 更新・event append」を原子的に揃える
        //    （spec [08] 振る舞い定義 Rule 1: 適合しない場合 / 適合する場合いずれも
        //    state と event log が一致する）。
        let event = workflow_output_submission::artifact_produced_event(
            execution_id,
            &mutation.workflow_name,
            &mutation.node_execution_id,
            &node_name,
            contract,
            validated_output,
            None,
            None,
            timestamp,
        );
        if let Err(append_err) = self.write_log_required(app, event) {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(execution_id) {
                workflow_output_submission::rollback_validated_submission(
                    exec, &node_name, mutation,
                );
            }
            return Err(WorkflowEngineError::SessionStore(append_err));
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_invalid_submit_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        node_name: &str,
        node_execution_id: Option<&str>,
        contract: &str,
        validation_error: workflow_output_submission::SubmissionValidationError,
    ) -> Result<(), WorkflowEngineError> {
        let target = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            workflow_output_submission::validate_submission_target_context(
                exec,
                execution_id,
                node_name,
                node_execution_id,
                contract,
            )
        };
        let target = match target {
            Ok(target) => target,
            Err(_) => return Err(validation_error.into_engine_error()),
        };
        let schema_violations = validation_error.schema_violations().map(Vec::from);
        self.handle_missing_required_output(
            app,
            session_store,
            agent_runtime,
            &target.worktree_path,
            execution_id,
            &target.workflow_name,
            node_name,
            contract,
            target.attempt,
            target.session_id.as_deref(),
            None,
            Some(target.node_execution_id.as_str()),
            SubmissionViolation::InvalidSubmitOutput,
            schema_violations.as_deref(),
        )
        .await
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
    ) -> Result<(), WorkflowEngineError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        let (snapshot, snapshot_before, worktree_path, execution_id, stall_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&session_ref.execution_id) else {
                return Ok(());
            };
            if !exec.is_active() {
                return Ok(());
            }

            let Some((node_execution_id, node_name, attempt)) =
                current_step_for_stall_observation(exec, session_id)
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
            upsert_stall_observation(&mut exec.current_stall_observations, observation.clone());
            exec.updated_at = observed_at;
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
                exec.to_workflow_state(),
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
    ) -> Result<(), WorkflowEngineError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        let (snapshot, snapshot_before, worktree_path, execution_id, clear_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&session_ref.execution_id) else {
                return Ok(());
            };
            if !exec.is_active() {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let Some((node_execution_id, _, _)) =
                current_step_for_stall_observation(exec, session_id)
            else {
                return Ok(());
            };
            if !clear_stall_observations_for_session(
                &mut exec.current_stall_observations,
                session_id,
            ) {
                return Ok(());
            }
            let cleared_at = current_timestamp();
            exec.updated_at = cleared_at;
            let clear_event = WorkflowEvent::StallCleared {
                execution_id: exec.id.clone(),
                node_execution_id,
                session_id: session_id.to_string(),
                timestamp: cleared_at,
            };
            (
                exec.to_workflow_state(),
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
        snapshot: WorkflowState,
        snapshot_before: WorkflowExecution,
        execution_store_snapshot_before: Option<WorkflowExecutionMetadata>,
        worktree_path: String,
        event: WorkflowEvent,
        append_error_context: &'static str,
    ) -> Result<(), WorkflowEngineError> {
        if let Err(error) = workflow_runtime_commit::sync_execution_store_from_snapshot(
            &self.execution_store,
            execution_id,
            &snapshot,
        )
        .await
        {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(execution_id) {
                *exec = snapshot_before;
            }
            drop(execs);
            let _ = workflow_runtime_commit::restore_execution_store_active_snapshot(
                &self.execution_store,
                execution_store_snapshot_before,
            )
            .await;
            return Err(error);
        }
        if let Err(error) = self.write_log_required(app, event) {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(execution_id) {
                *exec = snapshot_before;
            }
            drop(execs);
            if let Err(rollback_error) =
                workflow_runtime_commit::restore_execution_store_active_snapshot(
                    &self.execution_store,
                    execution_store_snapshot_before,
                )
                .await
            {
                return Err(WorkflowEngineError::SessionStore(format!(
                    "{append_error_context}: {error}; {rollback_error}"
                )));
            }
            return Err(WorkflowEngineError::SessionStore(format!(
                "{append_error_context}: {error}"
            )));
        }
        record_failed_snapshot_telemetry(&snapshot);
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        Ok(())
    }

    /// turn_complete後に呼ばれるフック。
    /// autoモード→タグ検出で遷移、approvalモード→WaitingApproval、interactiveモード→何もしない。
    /// SessionError / WaitApproval は判定 + 状態変更を1回のロックで原子的に実行する。
    /// AutoEvaluate はタグ検出が必要なため handle_auto_complete に委譲する。
    #[allow(clippy::too_many_arguments)]
    pub async fn on_turn_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        session_id: &str,
        exit_code: i64,
        failure_signal: Option<workflow_transition::SessionFailureSignal>,
        final_parts: &[crate::usecase::agent_session::session::MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowEngineError> {
        // session_id からSessionWorkflowRefを解決（ワークフロー既終了なら何もしない）
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        // parent ChatSession 機構撤去後は step session のみが登録されるため種別分岐なし。
        // 逐次 step / 並列子 step の区別は WorkflowExecution.parallel_run に当該 session_id が
        // 含まれるかで判定する（Spec issues-929）。

        // SessionWorkflowRef.execution_id から exec を直接引き、属性として worktree_path を取得する
        // （Spec issues-1011: engine 内部キーも execution_id）。下流の handle_* は worktree_path を
        // 引数に取るため、ここで派生取得する。
        let (worktree_path, parallel_parent): (String, Option<String>) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(&session_ref.execution_id) else {
                return Ok(());
            };
            let wt = exec.worktree_path.clone();
            let pp = exec.parallel_run.as_ref().and_then(|pr| {
                pr.children
                    .iter()
                    .find(|c| c.session_id == session_id)
                    .map(|_| pr.parent_node_name.clone())
            });
            (wt, pp)
        };

        if let Some(parent_node_name) = parallel_parent {
            return self
                .handle_parallel_child_complete(
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
            outcome: StepOutcome,
            required_events: Vec<WorkflowEvent>,
            rollback_snapshot: (String, WorkflowExecution),
        }

        // 判定 + 状態変更を原子的に実行（AutoEvaluate以外）
        let action_or_outcome = {
            let mut execs = self.executions.lock().await;
            let exec = execs.get_mut(&session_ref.execution_id).ok_or_else(|| {
                WorkflowEngineError::ExecutionNotFound(session_ref.execution_id.clone())
            })?;

            // 現行ステップのセッション以外からの完了通知は無視
            if exec.current_session_id.as_deref() != Some(session_id) {
                return Ok(());
            }

            // トークン使用量を現在のステップに累計
            if let Some((input, output)) = token_usage {
                exec.current_step_token_usage.add(&TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
            let plan = exec.plan_turn_complete_mutation(exit_code, failure_signal)?;

            match plan {
                workflow_transition::TurnCompleteMutationPlan::NotRunning => return Ok(()),
                workflow_transition::TurnCompleteMutationPlan::SessionError {
                    node_name,
                    history_result,
                    mut failure_reason,
                    kind,
                    ..
                } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let retry_count = exec
                        .node_execution_counts
                        .get(&node_name)
                        .copied()
                        .unwrap_or(1)
                        .saturating_sub(1);
                    let retry_policy = RetryPolicy::default();
                    let retry_allowed = retry_policy.should_retry(kind, retry_count);
                    let outcome = if retry_allowed {
                        exec.retry_current_step()
                    } else {
                        let max_retries = retry_policy.max_retries(kind);
                        failure_reason.push_str(&format!(
                            "; retry policy did not retry failure_kind={} retry_count={} max_retries={}",
                            kind.as_str(),
                            retry_count,
                            max_retries
                        ));
                        let entry = exec.make_node_history_entry(Some(history_result), None, None);
                        exec.node_history.push(entry);
                        exec.state = RuntimeExecutionState::Failed {
                            reason: failure_reason,
                            kind,
                            retry_count: Some(retry_count),
                        };
                        exec.updated_at = current_timestamp();
                        StepOutcome::Persist(exec.to_workflow_state())
                    };
                    Ok(TurnCommit {
                        outcome,
                        required_events: Vec::new(),
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                workflow_transition::TurnCompleteMutationPlan::RequestApproval { node_name } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let node_execution_id = exec
                        .active_current_node_execution_id()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| {
                            WorkflowEngineError::InvalidState(format!(
                                "active NodeExecution for approval node '{node_name}' was not found"
                            ))
                        })?;
                    if let Some(execution) = exec
                        .node_executions
                        .iter_mut()
                        .find(|execution| execution.id == node_execution_id)
                    {
                        execution.status = NodeExecutionStatus::WaitingApproval;
                    }
                    exec.state = RuntimeExecutionState::WaitingApproval;
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
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
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let entry =
                        exec.make_node_history_entry(Some(failure_reason.clone()), None, None);
                    exec.node_history.push(entry);
                    exec.state = RuntimeExecutionState::Failed {
                        reason: failure_reason,
                        kind: NodeExecutionFailureKind::ValidationFailure,
                        retry_count: None,
                    };
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
                        required_events: Vec::new(),
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
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
            Err(node_name) => {
                self.handle_auto_complete(
                    app,
                    session_store,
                    agent_runtime,
                    &worktree_path,
                    final_parts,
                    &node_name,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_required_turn_events_and_execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        outcome: StepOutcome,
        required_events: Vec<WorkflowEvent>,
        rollback_snapshot: Option<(String, WorkflowExecution)>,
    ) -> Result<(), WorkflowEngineError> {
        let Some((execution_id, snapshot_before)) = rollback_snapshot else {
            return Err(WorkflowEngineError::SessionStore(
                "required turn event commit missing rollback snapshot".to_string(),
            ));
        };
        let completed_step_session_ids = outcome.completed_step_session_ids();
        let snapshot_for_commit = outcome.snapshot().clone();
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(&execution_id)
            .await;

        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                execution_id: &execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                execution_store_snapshot_before,
                required_events,
                append_error_context: "turn_complete required event append failed",
            },
        )
        .await?;

        workflow_runtime_session::release_completed_step_sessions(
            app,
            session_store,
            agent_runtime,
            &completed_step_session_ids,
        )
        .await;
        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path, true)
            .await;
        if let Err(e) = self
            .dispatch_step_outcome_side_effects(
                app,
                session_store,
                agent_runtime,
                worktree_path,
                outcome,
                OutcomeCommitMode::EmitProgressEvents,
            )
            .await
        {
            log::warn!("workflow {execution_id}: post-commit turn side effects failed: {e}");
        }
        Ok(())
    }

    fn apply_approval_application(
        exec: &mut WorkflowExecution,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let plan = exec.plan_approval_application(application)?;
        let completion = plan.completion;
        let entry = exec.make_node_history_entry(
            Some(completion.result),
            completion.artifact,
            completion.contract,
        );
        exec.node_history.push(entry);
        let outcome = exec.apply_advance();
        Ok(outcome)
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
    pub(crate) async fn resolve_workflow_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        comment: Option<String>,
        expected_node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
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
    async fn handle_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        comment: Option<String>,
        expected_node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        let fanout_node_execution_id = {
            let executions = self.executions.lock().await;
            let execution = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            resolve_fanout_approval_node_execution_id(
                execution,
                expected_node_name,
                node_execution_id,
            )?
        };
        if let Some(fanout_node_execution_id) = fanout_node_execution_id {
            return self
                .handle_fanout_child_approval(
                    app,
                    session_store,
                    agent_runtime,
                    execution_id,
                    comment,
                    expected_node_name,
                    &fanout_node_execution_id,
                )
                .await;
        }

        // target検証 + session_id + worktree_path + contract 提出状態を1回のロックで取得
        let (
            current_session_id,
            worktree_path,
            workflow_name_for_contract,
            node_name_for_contract,
            approval_contract,
            approval_attempt,
            approval_submitted_output,
            resolved_node_execution_id,
        ) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
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
                workflow_output_submission::submitted_step_output_for(
                    &exec.artifacts,
                    &node.name,
                    attempt,
                    contract,
                )
            });
            (
                exec.current_session_id.clone(),
                exec.worktree_path.clone(),
                exec.workflow.name.clone(),
                node.name.clone(),
                contract,
                attempt,
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

        let approve_submitted_output = if let Some(ref contract) = approval_contract {
            if let Some(output) = approval_submitted_output {
                Some(output)
            } else {
                self.handle_missing_required_output(
                    app,
                    session_store,
                    agent_runtime,
                    &worktree_path,
                    execution_id,
                    &workflow_name_for_contract,
                    &node_name_for_contract,
                    contract,
                    approval_attempt,
                    current_session_id.as_deref(),
                    None,
                    None,
                    SubmissionViolation::MissingSubmitOutput,
                    None,
                )
                .await?;
                return Err(WorkflowEngineError::ValidationError(
                    "required structured output has not been submitted".to_string(),
                ));
            }
        } else {
            None
        };

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

        // [04] atomic mutation 境界: mutation 直前の WorkflowExecution 全体を snapshot に
        // 保持し、ApprovalResolved event append / persist のいずれかが失敗した場合は
        // `*exec = snapshot` で全フィールド（履歴・変数・state・current_node_index 等）を
        // 一括復元する。部分 rollback helper は使わない。
        let (mut outcome, exec_snapshot_before, node_name_for_event) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            workflow_approval_runtime::resolve_approval_target_snapshot(
                exec,
                Some(execution_id),
                Some(expected_node_name),
            )?;
            let execution_index = resolve_active_node_execution_index(
                exec,
                expected_node_name,
                Some(&resolved_node_execution_id),
            )?;
            let node_name = exec.workflow.nodes[exec.current_node_index].name.clone();
            let snapshot_before = exec.clone();
            let outcome = Self::apply_approval_application(
                exec,
                workflow_transition::ApprovalApplication {
                    effective_result,
                    artifact: artifact.clone(),
                    contract: application_contract,
                },
            )?;
            if let Some(node_execution) = exec.node_executions.get_mut(execution_index) {
                node_execution.status = NodeExecutionStatus::Succeeded;
                node_execution.artifact = artifact.clone();
                node_execution.completed_at = Some(current_timestamp());
            }
            (outcome, snapshot_before, node_name)
        };

        let snapshot_for_commit = outcome.snapshot().clone();
        let completed_step_session_ids = outcome.completed_step_session_ids();
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
        // approval commit 境界として失敗扱いし、snapshot_before で engine state /
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
        // command failure に射影しない（spec [04] post-commit 境界）。session release /
        // broadcast / terminal log / cleanup / 次 step 起動 / auto-approve primitive は
        // ここで実行する。
        workflow_runtime_session::release_completed_step_sessions(
            app,
            session_store,
            agent_runtime,
            &completed_step_session_ids,
        )
        .await;
        self.finalize_after_commit(app, &snapshot_for_commit, &worktree_path, false)
            .await;
        if let Err(e) = self
            .dispatch_step_outcome_side_effects(
                app,
                session_store,
                agent_runtime,
                &worktree_path,
                outcome,
                OutcomeCommitMode::ProgressEventsAlreadyCommitted,
            )
            .await
        {
            log::warn!("workflow {execution_id}: post-commit side effects failed: {e}");
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_fanout_child_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        comment: Option<String>,
        expected_node_name: &str,
        node_execution_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        workflow_approval_runtime::validate_approve_comment(comment.as_deref())?;
        let (worktree_path, workflow_name, session_id, attempt, contract, submitted_artifact) = {
            let executions = self.executions.lock().await;
            let execution = executions
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
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
                    WorkflowEngineError::UnauthorizedApprovalTarget(format!(
                        "fanout approval NodeExecution '{node_execution_id}' is not waiting"
                    ))
                })?;
            let node = execution
                .workflow
                .nodes
                .iter()
                .find(|node| node.name == expected_node_name && node.is_approval_session())
                .ok_or_else(|| {
                    WorkflowEngineError::UnauthorizedApprovalTarget(
                        "node is not an approval session".to_string(),
                    )
                })?;
            (
                execution.worktree_path.clone(),
                execution.workflow.name.clone(),
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
        if let Some(contract) = contract.as_deref() {
            if submitted_artifact.is_none() {
                self.handle_missing_required_output(
                    app,
                    session_store,
                    agent_runtime,
                    &worktree_path,
                    execution_id,
                    &workflow_name,
                    expected_node_name,
                    contract,
                    attempt,
                    session_id.as_deref(),
                    None,
                    Some(node_execution_id),
                    SubmissionViolation::MissingSubmitOutput,
                    None,
                )
                .await?;
                return Err(WorkflowEngineError::ValidationError(
                    "required structured output has not been submitted".to_string(),
                ));
            }
        }

        let event_comment = comment.map(|raw| {
            let secrets = secret_source::collect_configured_secret_values(app);
            workflow_secret_masker::mask_sensitive_text(&raw, &secrets)
        });
        let completed_at = current_timestamp();
        let completion = {
            let mut executions = self.executions.lock().await;
            let execution = executions
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            let snapshot_before = execution.clone();
            let node_execution_index = execution
                .node_executions
                .iter()
                .position(|candidate| {
                    candidate.id == node_execution_id
                        && candidate.status == NodeExecutionStatus::WaitingApproval
                })
                .ok_or_else(|| {
                    WorkflowEngineError::UnauthorizedApprovalTarget(format!(
                        "fanout approval NodeExecution '{node_execution_id}' is no longer waiting"
                    ))
                })?;
            let child = execution
                .parallel_run
                .as_mut()
                .and_then(|run| {
                    run.children
                        .iter_mut()
                        .find(|child| child.node_execution_id == node_execution_id)
                })
                .ok_or_else(|| {
                    WorkflowEngineError::InvalidState(format!(
                        "fanout approval child '{node_execution_id}' disappeared"
                    ))
                })?;
            child.state = FanoutChildRuntimeState::Completed;
            child.result = child.result.clone().or_else(|| Some("approve".to_string()));
            child.artifact = submitted_artifact.clone();
            child.failure_kind = None;
            child.failure_disposition = None;
            child.completed_at = Some(completed_at);
            let child_result = child.result.clone();
            let child_tokens = child.token_usage.clone();
            let node_execution = &mut execution.node_executions[node_execution_index];
            node_execution.status = NodeExecutionStatus::Succeeded;
            node_execution.artifact = submitted_artifact.clone();
            node_execution.token_usage = Some(child_tokens.clone());
            node_execution.failure = None;
            node_execution.completed_at = Some(completed_at);
            execution.updated_at = completed_at;
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
            let completed_sessions = session_id.into_iter().collect::<Vec<_>>();
            self.commit_required_parallel_progress_events_and_execute_outcome(
                app,
                session_store,
                agent_runtime,
                &worktree_path,
                outcome,
                completion.snapshot_before,
                completion.progress_events,
                &completed_sessions,
                completion.failure_telemetry,
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn abort_workflow_execution<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        // execution 全体の Abort: NotFound / AlreadyTerminal は非受理として typed error
        // に射影する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
        match self
            .abort_workflow_by_execution_id(
                app,
                session_store,
                agent_runtime,
                execution_id,
                expected_node_name,
            )
            .await?
        {
            AbortOutcome::Aborted => Ok(()),
            AbortOutcome::NotFound => Err(WorkflowEngineError::ExecutionNotFound(
                execution_id.to_string(),
            )),
            AbortOutcome::AlreadyTerminal => Err(WorkflowEngineError::InvalidState(format!(
                "execution {execution_id} is already terminal"
            ))),
        }
    }

    /// ワークフローを中断する。
    /// `execution_id` を主語に workflow を中断する。
    ///
    /// Spec issues-1011 finding 2/10: 全経路で `executions.get_mut(execution_id)` を使い、
    /// worktree_path 経由の委譲を排除する。これにより、同一 worktree に terminal execution と
    /// active execution が共存しても誤って別 execution を中断する TOCTOU を構造的に排除する。
    ///
    /// Spec [04]: `AbortExecution` command handler の境界。
    /// - 対象 execution が存在しない場合は `AbortOutcome::NotFound` を返す（非受理）。
    /// - 既に terminal な execution の場合は `AbortOutcome::AlreadyTerminal` を返す（非受理）。
    /// - 実際に Aborted に遷移し ExecutionAborted event を必須 append できた場合のみ
    ///   `AbortOutcome::Aborted` を返す。
    ///
    /// ExecutionAborted event は `write_log_required` 経由で必須 append し、append 失敗時は
    /// mutation 直前 snapshot で `WorkflowExecution` 全体を一括復元する
    /// （Spec atomic mutation 境界）。
    ///
    /// 外部から直接呼ばれることはなく、`abort_workflow_execution*` runtime primitive 経路のみが
    /// 利用する（Spec [04]: 内部呼び出し元も engine の private method を直接叩かない）。
    async fn abort_workflow_by_execution_id<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<AbortOutcome, WorkflowEngineError> {
        // 1. 対象 execution の存在 + active 性を判定。
        //    非受理経路 (NotFound / AlreadyTerminal) ではどんな外部副作用も発生させない。
        let lookup = self.abort_target_lookup(execution_id).await;
        let (current_step_session_id, parallel_session_ids) = match lookup {
            AbortTargetLookup::NotFound => return Ok(AbortOutcome::NotFound),
            AbortTargetLookup::AlreadyTerminal => return Ok(AbortOutcome::AlreadyTerminal),
            AbortTargetLookup::Active {
                current_step_session_id,
                parallel_session_ids,
            } => (current_step_session_id, parallel_session_ids),
        };
        #[cfg(test)]
        self.wait_abort_after_lookup_for_test().await;

        // 2. [04] pre-commit (rollback 可能): mutation 直前 snapshot を取得し、
        //    state を Aborted に遷移させる。競合で terminal 化していた場合は
        //    AlreadyTerminal で返す。
        let timestamp = current_timestamp();
        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(execution_id)
            .await;
        let (snapshot_before, snapshot_state, aborted_node_for_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(execution_id) else {
                drop(execs);
                return Ok(if self.has_terminal_execution_record(execution_id).await {
                    AbortOutcome::AlreadyTerminal
                } else {
                    AbortOutcome::NotFound
                });
            };
            if !exec.is_active() {
                return Ok(AbortOutcome::AlreadyTerminal);
            }
            if let Some(expected_node_name) = expected_node_name {
                let current_node = exec
                    .workflow
                    .nodes
                    .get(exec.current_node_index)
                    .map(|node| node.name.as_str())
                    .ok_or_else(|| {
                        WorkflowEngineError::InvalidState(format!(
                            "execution {execution_id} has invalid current node"
                        ))
                    })?;
                if expected_node_name != current_node {
                    return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                        "node does not match".to_string(),
                    ));
                }
            }
            let snapshot_before = exec.clone();
            let aborted_node_for_event = exec
                .parallel_run
                .as_ref()
                .map(|fanout| fanout.parent_node_name.clone())
                .or_else(|| {
                    exec.workflow
                        .nodes
                        .get(exec.current_node_index)
                        .map(|node| node.name.clone())
                });

            // spec issues-1023: state を Aborted にする前に、中断時の current step /
            // parallel children を `node_history` に "aborted" entry として記録する。
            // これにより UI 側は既存 history 描画経路 + session_id を使って中断 step の
            // session log にアクセスできるようになる。`exec.parallel_run = None` を
            // 明示クリアして `to_workflow_state()` 経由の二重表示を防ぐ。
            if exec.parallel_run.is_some() {
                if let Some(entry) = exec.make_aborted_parallel_history_entry(timestamp) {
                    exec.node_history.push(entry);
                }
                exec.parallel_run = None;
            } else {
                let current_node_name = exec.workflow.nodes[exec.current_node_index].name.clone();
                let current_attempt = exec
                    .node_execution_counts
                    .get(&current_node_name)
                    .copied()
                    .unwrap_or(1);
                let already_in_history = exec.node_history.last().is_some_and(|e| {
                    e.node_name == current_node_name && e.attempt == current_attempt
                });
                if !already_in_history {
                    let entry = exec.make_aborted_history_entry(timestamp);
                    exec.node_history.push(entry);
                }
            }

            exec.state = RuntimeExecutionState::Aborted;
            exec.current_stall_observations.clear();
            exec.updated_at = timestamp;
            let snapshot_state = exec.to_workflow_state();
            (snapshot_before, snapshot_state, aborted_node_for_event)
        };

        // 3. [04] commit point: ExecutionAborted を必須 append。失敗時は
        //    WorkflowExecution / Execution Store / ChatSession を snapshot で一括復元する。
        //    interrupt_agent はこの時点ではまだ実行していないため、append 失敗時には
        //    rollback 不能な外部副作用が残らない。
        let aborted_event = WorkflowEvent::ExecutionAborted {
            execution_id: execution_id.to_string(),
            aborted_node: aborted_node_for_event,
            timestamp,
        };
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                execution_id,
                snapshot_for_commit: &snapshot_state,
                snapshot_before,
                execution_store_snapshot_before,
                required_events: vec![aborted_event],
                append_error_context: "ExecutionAborted log failed",
            },
        )
        .await?;
        crate::other::telemetry::record_workflow_node_failure(
            FailureClassification::new(NodeExecutionFailureKind::UserAbort),
            None,
        );

        // 4. [04] post-commit: interrupt_agent / cleanup / broadcast。
        //    ExecutionAborted event は append 済み。Execution Store / ChatSession は event 後の
        //    projection として同期済み、または warn として観測済み。
        self.shutdown_active_commands_for_execution(execution_id)
            .await;
        if let Some(ref step_sid) = current_step_session_id {
            workflow_runtime_session::interrupt_agent(agent_runtime, step_sid).await;
        }
        if let Some(ref session_ids) = parallel_session_ids {
            for sid in session_ids {
                workflow_runtime_session::interrupt_agent(agent_runtime, sid).await;
            }
        }
        self.finalize_terminal_transition_after_required_append(
            app,
            session_store,
            agent_runtime,
            execution_id,
        )
        .await;

        Ok(AbortOutcome::Aborted)
    }

    /// `abort_workflow_by_execution_id` の post-commit 区間。state は呼出し前に Aborted に
    /// 遷移済みで、`ExecutionAborted` event は必須 append 済み、かつ Execution Store sync も
    /// 完了済みである前提。ChatSession persist / step session release / refs cleanup /
    /// broadcast を実行する。
    ///
    /// [04] post-commit 失敗は warn ログのみで command 結果に伝播させない。観測可能な
    /// 事実は既に ExecutionAborted で確定しており、ここでの副作用失敗を command failure に
    /// 射影すると spec [04] の「post-commit 失敗は command failure として返さない」に
    /// 違反するため。
    async fn finalize_terminal_transition_after_required_append<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
    ) {
        let (snapshot, worktree_path) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(execution_id) else {
                return;
            };
            (exec.to_workflow_state(), exec.worktree_path.clone())
        };

        // terminal session の release と refs cleanup。
        let terminal_session_ids = workflow_runtime_commit::terminal_step_session_ids(&snapshot);
        workflow_runtime_session::release_completed_step_sessions(
            app,
            session_store,
            agent_runtime,
            &terminal_session_ids,
        )
        .await;
        self.cleanup_session_workflow_refs_by_execution_id(execution_id)
            .await;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        self.release_terminal_execution(execution_id).await;
    }

    async fn abort_target_lookup(&self, execution_id: &str) -> AbortTargetLookup {
        {
            let execs = self.executions.lock().await;
            if let Some(exec) = execs.get(execution_id) {
                if !exec.is_active() {
                    return AbortTargetLookup::AlreadyTerminal;
                }
                let current_step_session_id = exec.current_session_id.clone();
                let parallel_session_ids = exec.parallel_run.as_ref().map(|pr| {
                    pr.children
                        .iter()
                        .filter(|c| c.state == FanoutChildRuntimeState::Running)
                        .map(|c| c.session_id.clone())
                        .collect::<Vec<_>>()
                });
                return AbortTargetLookup::Active {
                    current_step_session_id,
                    parallel_session_ids,
                };
            }
        }
        if self.has_terminal_execution_record(execution_id).await {
            AbortTargetLookup::AlreadyTerminal
        } else {
            AbortTargetLookup::NotFound
        }
    }

    async fn has_terminal_execution_record(&self, execution_id: &str) -> bool {
        self.execution_store
            .get_execution_record(execution_id)
            .await
            .is_some_and(|run| run.status.is_terminal())
    }

    async fn request_fanout_child_approval_if_needed<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        execution_id: &str,
        worktree_path: &str,
        session_id: &str,
        parent_node_name: &str,
    ) -> Result<bool, WorkflowEngineError> {
        let mutation = {
            let mut executions = self.executions.lock().await;
            let execution = executions
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            let Some(run) = execution.parallel_run.as_ref() else {
                return Ok(false);
            };
            if run.parent_node_name != parent_node_name {
                return Ok(false);
            }
            let Some(child) = run
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
                return Err(WorkflowEngineError::InvalidState(format!(
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
            execution.node_executions[node_execution_index].status =
                NodeExecutionStatus::WaitingApproval;
            execution.updated_at = timestamp;
            let snapshot = execution.to_workflow_state();
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
    async fn handle_parallel_child_complete<R: tauri::Runtime>(
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
    ) -> Result<(), WorkflowEngineError> {
        // [08] parallel child の構造化出力は CLI / Tauri 経由の `SubmitOutput` で確定する。
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
        let (child_result, child_artifact, missing_child_output) = if !child_failed {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            if exec.is_terminal() {
                return Ok(());
            }
            let Some(pr) = exec.parallel_run.as_ref() else {
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
            if let Some(contract) = child.contract.clone() {
                let missing = if artifact.is_none() {
                    Some((
                        exec.workflow.name.clone(),
                        child.node_name.clone(),
                        child.node_execution_id.clone(),
                        child.attempt,
                        contract.clone(),
                    ))
                } else {
                    None
                };
                (child.result.clone(), artifact, missing)
            } else {
                (child.result.clone(), None, None)
            }
        } else {
            (None, None, None)
        };
        if let Some((workflow_name, child_name, child_node_execution_id, child_attempt, contract)) =
            missing_child_output
        {
            self.handle_missing_required_output(
                app,
                session_store,
                agent_runtime,
                worktree_path,
                execution_id,
                &workflow_name,
                &child_name,
                &contract,
                child_attempt,
                Some(session_id),
                None,
                Some(&child_node_execution_id),
                SubmissionViolation::MissingSubmitOutput,
                None,
            )
            .await?;
            return Ok(());
        }
        // ロック内: 子ステップの状態更新 + 全完了チェック
        let (completion_commit, interrupted_session_ids, interrupted_command_ids) = 'state_update: {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;

            if exec.is_terminal() {
                return Ok(());
            }
            // [05] commit 境界: 子ステップ失敗 → workflow 全体 Failed の terminal event は
            // pre-commit batch で append し、失敗時は engine state を snapshot_before で
            // 一括復元する（post-persist warn 廃止）。snapshot は mutation 前にここで取得する。
            let exec_snapshot_before = exec.clone();
            let Some(pr) = exec.parallel_run.as_mut() else {
                return Ok(());
            };
            if pr.parent_node_name != parent_node_name {
                return Ok(());
            }
            // 対象の子ステップを見つけて更新
            let Some(child) = pr.children.iter_mut().find(|c| c.session_id == session_id) else {
                return Ok(());
            };

            if let Some((input, output)) = token_usage {
                child.token_usage.add(&TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }

            if child_failed {
                let failure_kind = parallel_child_failure_kind(exit_code, failure_signal);
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
                        child_failure_reason: reason.clone(),
                        terminal_reason: reason,
                        failure_kind,
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
            child.state = FanoutChildRuntimeState::Completed;
            child.result = child_result.clone();
            child.artifact = child_artifact.clone();
            child.failure_kind = None;
            child.failure_disposition = None;
            let child_name = child.node_name.clone();
            let child_node_execution_id = child.node_execution_id.clone();
            let child_token_usage = child.token_usage.clone();
            let child_attempt = child.attempt;
            let child_contract = child.contract.clone();
            let completed_at = current_timestamp();
            child.completed_at = Some(completed_at);
            if let Some(execution) = exec
                .node_executions
                .iter_mut()
                .find(|execution| execution.id == child_node_execution_id)
            {
                execution.status = NodeExecutionStatus::Succeeded;
                execution.artifact = child_artifact.clone();
                execution.token_usage = Some(child_token_usage.clone());
                execution.failure = None;
                execution.completed_at = Some(completed_at);
            }

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
                token_usage: Some(crate::adaptor::gateway::workflow::event::TokenUsage {
                    input_tokens: child_token_usage.input_tokens,
                    output_tokens: child_token_usage.output_tokens,
                }),
                timestamp: completed_at,
            });
            clear_stall_observations_for_session(&mut exec.current_stall_observations, session_id);

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

        if required_progress_events {
            if let Some(outcome) = outcome {
                let mut completed_session_ids = interrupted_session_ids.clone();
                completed_session_ids.push(session_id.to_string());
                completed_session_ids.sort();
                completed_session_ids.dedup();
                self.commit_required_parallel_progress_events_and_execute_outcome(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    outcome,
                    snapshot_before,
                    progress_events,
                    &completed_session_ids,
                    failure_telemetry,
                )
                .await?;
            }
            for interrupted_session_id in interrupted_session_ids {
                workflow_runtime_session::interrupt_agent(agent_runtime, &interrupted_session_id)
                    .await;
            }
            if !interrupted_command_ids.is_empty() {
                let handles = self.active_commands.lock().await;
                for node_execution_id in interrupted_command_ids {
                    if let Some(handle) = handles.get(&node_execution_id) {
                        handle.request_shutdown();
                    }
                }
            }
            return Ok(());
        }

        for event in progress_events {
            self.write_log(app, event);
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
                if let StepOutcome::Persist(snapshot) = outcome {
                    self.persist_release_and_broadcast(
                        app,
                        session_store,
                        agent_runtime,
                        worktree_path,
                        snapshot,
                        &[session_id.to_string()],
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_required_parallel_progress_events_and_execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        outcome: StepOutcome,
        snapshot_before: WorkflowExecution,
        mut required_events: Vec<WorkflowEvent>,
        extra_completed_step_session_ids: &[String],
        failure_telemetry: Option<FailureClassification>,
    ) -> Result<(), WorkflowEngineError> {
        let execution_id = outcome.snapshot().execution_id.clone();
        let snapshot_for_commit = outcome.snapshot().clone();
        let mut completed_step_session_ids = outcome.completed_step_session_ids();
        completed_step_session_ids.extend(extra_completed_step_session_ids.iter().cloned());
        completed_step_session_ids.sort();
        completed_step_session_ids.dedup();

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
                execution_id: &execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                execution_store_snapshot_before,
                required_events,
                append_error_context: "parallel child progress event append failed",
            },
        )
        .await?;

        if let Some(classification) = failure_telemetry {
            crate::other::telemetry::record_workflow_node_failure(classification, None);
        }

        workflow_runtime_session::release_completed_step_sessions(
            app,
            session_store,
            agent_runtime,
            &completed_step_session_ids,
        )
        .await;
        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path, false)
            .await;
        self.dispatch_step_outcome_side_effects(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            outcome,
            OutcomeCommitMode::ProgressEventsAlreadyCommitted,
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

    /// 状態取得。`worktree_path` 属性で in-memory 実行表を検索する。
    pub async fn get_state(&self, worktree_path: &str) -> Option<WorkflowState> {
        let execs = self.executions.lock().await;
        find_by_worktree(&execs, worktree_path).map(|(_, e)| e.to_workflow_state())
    }

    /// `execution_id` から `WorkflowState` を取得する。
    #[cfg(test)]
    pub async fn get_state_by_execution_id(&self, execution_id: &str) -> Option<WorkflowState> {
        let execs = self.executions.lock().await;
        execs.get(execution_id).map(|exec| exec.to_workflow_state())
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

    fn contract_repair_attempt_count<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        execution_id: &str,
        node_execution_id: &str,
    ) -> Result<u32, WorkflowEngineError> {
        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
        let log = WorkflowEventLog::new(&data_dir);
        let events = log
            .read_log(execution_id)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    WorkflowEvent::ContractViolated {
                        node_execution_id: event_node_execution_id,
                        ..
                    } if event_node_execution_id == node_execution_id
                )
            })
            .count() as u32)
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_missing_required_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        execution_id: &str,
        _workflow_name: &str,
        node_name: &str,
        contract: &str,
        attempt: u32,
        session_id: Option<&str>,
        request_id: Option<&str>,
        node_execution_id: Option<&str>,
        violation: SubmissionViolation,
        schema_violations: Option<&[workflow_contract_schema::SchemaViolation]>,
    ) -> Result<(), WorkflowEngineError> {
        let projected_node_execution_id = if let Some(node_execution_id) = node_execution_id {
            node_execution_id.to_string()
        } else {
            let executions = self.executions.lock().await;
            executions
                .get(execution_id)
                .and_then(|execution| {
                    execution
                        .node_executions
                        .iter()
                        .rev()
                        .find(|node| {
                            node.node_name == node_name
                                && node.attempt == attempt
                                && node.fanout_parent.is_none()
                        })
                        .map(|node| node.id.clone())
                })
                .ok_or_else(|| {
                    WorkflowEngineError::InvalidState(format!(
                        "active NodeExecution for '{node_name}' attempt {attempt} was not found"
                    ))
                })?
        };
        let prior_attempts =
            self.contract_repair_attempt_count(app, execution_id, &projected_node_execution_id)?;
        let repair_policy = StructuredOutputRepairPolicy::default();
        let decision = repair_policy.decide(prior_attempts, session_id.is_some());
        let RepairDecision::Repair { attempt } = decision else {
            let reason = match (session_id.is_none(), violation) {
                (true, _) => {
                    "no active session is available for contract output repair".to_string()
                }
                (false, SubmissionViolation::InvalidSubmitOutput) => format!(
                    "submitted structured output did not satisfy contract after {} repair attempts",
                    repair_policy.max_attempts()
                ),
                (false, SubmissionViolation::MissingSubmitOutput) => format!(
                    "required structured output was not submitted after {} repair attempts",
                    repair_policy.max_attempts()
                ),
            };
            return self
                .fail_missing_required_output(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    execution_id,
                    node_name,
                    contract,
                    &reason,
                    node_execution_id,
                )
                .await;
        };
        let session_id = session_id.expect("repair policy requires a session for Repair");

        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
        let Some(session) = session_store
            .get_session_meta(&data_dir, session_id)
            .map_err(WorkflowEngineError::SessionStore)?
        else {
            return self
                .fail_missing_required_output(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    execution_id,
                    node_name,
                    contract,
                    &format!("node session not found for contract repair: {session_id}"),
                    node_execution_id,
                )
                .await;
        };

        self.write_log_required(
            app,
            WorkflowEvent::ContractViolated {
                execution_id: execution_id.to_string(),
                node_execution_id: projected_node_execution_id,
                node_name: node_name.to_string(),
                violations: schema_violations
                    .map(|violations| {
                        violations
                            .iter()
                            .map(|violation| ContractViolationRecord {
                                path: violation.path.clone(),
                                reason: violation.reason.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        vec![ContractViolationRecord {
                            path: "$".to_string(),
                            reason: workflow_failure_wire::submission_violation_reason(violation)
                                .to_string(),
                        }]
                    }),
                repair_attempt: attempt,
                request_id: request_id.map(ToOwned::to_owned),
                timestamp: current_timestamp(),
            },
        )
        .map_err(WorkflowEngineError::SessionStore)?;

        let cli_alias = crate::infrastructure::platform::path_aliases::alias_name_for_profile(
            crate::infrastructure::platform::path_aliases::BuildProfile::current(),
        );
        let prompt = match (violation, schema_violations) {
            (SubmissionViolation::InvalidSubmitOutput, Some(violations)) => {
                workflow_contract::build_schema_violation_repair_prompt(
                    cli_alias,
                    execution_id,
                    node_name,
                    contract,
                    violations,
                )
            }
            _ => workflow_contract::build_missing_artifact_repair_prompt(
                cli_alias,
                execution_id,
                node_name,
                contract,
            ),
        };
        let permission_mode = PermissionMode::parse(&session.permission_mode)
            .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
        let _runtime_guard = agent_runtime.acquire_session_lock(session_id).await;
        let start_result = agent_runtime
            .start_turn_locked(
                session_id,
                permission_mode,
                prompt.clone(),
                None,
                Vec::new(),
            )
            .await;
        if let Err(err) = start_result {
            let error = WorkflowEngineError::with_agent_runtime_context(
                "contract output repair turn failed to start",
                err,
            );
            return self
                .fail_missing_required_output_with_metadata(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    execution_id,
                    node_name,
                    contract,
                    &error.to_string(),
                    error.workflow_failure_kind(),
                    error.retry_count(),
                    node_execution_id,
                )
                .await;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn fail_missing_required_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        reason: &str,
        node_execution_id: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        self.fail_missing_required_output_with_metadata(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            execution_id,
            node_name,
            contract,
            reason,
            NodeExecutionFailureKind::StructuredOutputMismatch,
            None,
            node_execution_id,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn fail_missing_required_output_with_metadata<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        execution_id: &str,
        node_name: &str,
        contract: &str,
        reason: &str,
        failure_kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
        node_execution_id: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        if let Some(node_execution_id) = node_execution_id {
            return self
                .fail_fanout_child_missing_required_output(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    execution_id,
                    node_execution_id,
                    node_name,
                    contract,
                    reason,
                    failure_kind,
                    retry_count,
                )
                .await;
        }
        let (snapshot, snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            if exec.is_terminal() {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let mut entry = exec.make_node_history_entry(
                Some("contract_missing_output".to_string()),
                None,
                Some(contract.to_string()),
            );
            entry.state = NODE_STATUS_FAILED.to_string();
            exec.node_history.push(entry);
            exec.state = RuntimeExecutionState::Failed {
                reason: format!(
                    "Required structured output for node '{node_name}' was not submitted: {reason}"
                ),
                kind: failure_kind,
                retry_count,
            };
            exec.updated_at = current_timestamp();
            (exec.to_workflow_state(), snapshot_before)
        };
        self.execute_outcome(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            StepOutcome::Persist(snapshot),
            snapshot_before,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn fail_fanout_child_missing_required_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        execution_id: &str,
        node_execution_id: &str,
        node_name: &str,
        contract: &str,
        reason: &str,
        failure_kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    ) -> Result<(), WorkflowEngineError> {
        let timestamp = current_timestamp();
        let failure_reason = format!(
            "Required structured output for node '{node_name}' was not submitted: {reason}"
        );
        let (
            outcome,
            snapshot_before,
            progress_events,
            interrupted_session_ids,
            interrupted_command_ids,
        ) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
            if exec.is_terminal() {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let Some(parallel_run) = exec.parallel_run.as_mut() else {
                return Err(WorkflowEngineError::InvalidState(format!(
                    "fanout child missing-output failure requires an active fanout runtime for '{node_execution_id}'"
                )));
            };
            let parent_node_execution_id = parallel_run.parent_node_execution_id.clone();
            let Some(child_index) = parallel_run
                .children
                .iter()
                .position(|child| child.node_execution_id == node_execution_id)
            else {
                return Err(WorkflowEngineError::InvalidState(format!(
                    "fanout child missing-output failure references unknown child '{node_execution_id}'"
                )));
            };
            let child_name = parallel_run.children[child_index].node_name.clone();
            let child_attempt = parallel_run.children[child_index].attempt;
            let mut interrupted_session_ids = Vec::new();
            let mut interrupted_command_ids = Vec::new();
            let mut interrupted_execution_ids = Vec::new();
            for (index, child) in parallel_run.children.iter_mut().enumerate() {
                if index == child_index {
                    child.state = FanoutChildRuntimeState::Failed;
                    child.result = Some("contract_missing_output".to_string());
                    child.artifact = None;
                    child.failure_kind = Some(failure_kind);
                    child.failure_disposition = None;
                    child.completed_at = Some(timestamp);
                    continue;
                }
                if child.state != FanoutChildRuntimeState::Running {
                    continue;
                }
                child.state = FanoutChildRuntimeState::Interrupted;
                child.completed_at = Some(timestamp);
                interrupted_execution_ids.push(child.node_execution_id.clone());
                if child.session_id.is_empty() {
                    interrupted_command_ids.push(child.node_execution_id.clone());
                } else {
                    interrupted_session_ids.push(child.session_id.clone());
                }
            }
            exec.fail_node_execution(
                node_execution_id,
                failure_reason.clone(),
                failure_kind,
                timestamp,
            );
            for execution_id in interrupted_execution_ids {
                if let Some(node_execution) = exec
                    .node_executions
                    .iter_mut()
                    .find(|execution| execution.id == execution_id)
                {
                    node_execution.status = NodeExecutionStatus::Aborted;
                    node_execution.completed_at = Some(timestamp);
                }
            }
            exec.fail_node_execution(
                &parent_node_execution_id,
                failure_reason.clone(),
                failure_kind,
                timestamp,
            );
            exec.current_stall_observations.clear();
            exec.state = RuntimeExecutionState::Failed {
                reason: failure_reason.clone(),
                kind: failure_kind,
                retry_count,
            };
            let mut entry = exec.make_node_history_entry(
                Some("contract_missing_output".to_string()),
                None,
                Some(contract.to_string()),
            );
            entry.state = NODE_STATUS_FAILED.to_string();
            exec.node_history.push(entry);
            exec.parallel_run = None;
            exec.updated_at = timestamp;
            let progress_events = vec![WorkflowEvent::NodeFailed {
                execution_id: execution_id.to_string(),
                node_execution_id: node_execution_id.to_string(),
                node_name: child_name,
                attempt: child_attempt,
                reason: failure_reason,
                failure_kind,
                retry_count,
                timestamp,
            }];
            (
                StepOutcome::Persist(exec.to_workflow_state()),
                snapshot_before,
                progress_events,
                interrupted_session_ids,
                interrupted_command_ids,
            )
        };

        self.commit_required_parallel_progress_events_and_execute_outcome(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            outcome,
            snapshot_before,
            progress_events,
            &[],
            Some(FailureClassification::with_disposition(
                failure_kind,
                FailureDisposition::Terminal,
            )),
        )
        .await?;
        for session_id in interrupted_session_ids {
            workflow_runtime_session::interrupt_agent(agent_runtime, &session_id).await;
        }
        if !interrupted_command_ids.is_empty() {
            let handles = self.active_commands.lock().await;
            for execution_id in interrupted_command_ids {
                if let Some(handle) = handles.get(&execution_id) {
                    handle.request_shutdown();
                }
            }
        }
        Ok(())
    }

    /// session_idがワークフロー実行中かどうか。
    pub async fn is_running(&self, session_id: &str) -> bool {
        let Some(worktree_path) = self.resolve_worktree_path(session_id).await else {
            return false;
        };
        let execs = self.executions.lock().await;
        find_by_worktree(&execs, &worktree_path).is_some_and(|(_, e)| e.is_active())
    }

    /// `execution_id` から approval 用 chat session（current step session）と worktree_path を解決する。
    /// Spec issues-1011 line 121: 起動以外の workflow 操作 API は execution_id を主語に取り、
    /// 内部の chat_session_id / worktree_path は engine が解決する。
    ///
    /// Spec issues-1011 finding 3: 任意 step session への注入経路を塞ぐため、resolve 時点で
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
    ) -> Result<(String, String), WorkflowEngineError> {
        let execs = self.executions.lock().await;
        let exec = execs
            .get(execution_id)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
        let session_id = workflow_approval_runtime::resolve_chat_session_for_approval(exec)?;
        Ok((session_id, exec.worktree_path.clone()))
    }

    pub async fn validate_approval_chat_instruction(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowEngineError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        // parent ChatSession 機構撤去後は step session のみが session_workflow_refs に登録される。

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
    ) -> Result<(), WorkflowEngineError> {
        let execs = self.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path)
            .ok_or_else(|| WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string()))?;
        workflow_approval_runtime::validate_approval_target_snapshot(
            exec,
            expected_execution_id,
            expected_node_name,
        )
    }

    /// セッションIDからworktree_pathを解決する。
    /// session_workflow_refsに登録されていない場合はNoneを返す。
    /// SessionWorkflowRef は execution_id を保持するため、executions から exec.worktree_path を
    /// 取得して返す（Spec issues-1011: engine 内部キーも execution_id）。
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

    // set_execution_state の lookup 戦略指定。WorkflowExecutionId バリアントは worktree_path を補助情報
    // として保持する（broadcast / cleanup の対象として）。
    // Note: enum 定義は impl の外側にあり、ここでは参照のみ可能（Rust 制約）。
    // 実体は WorkflowRuntimeService impl の下に置く。

    /// 実行状態を更新し、永続化・ブロードキャストする。
    /// 内部実装は `set_execution_state_inner` に集約され、worktree_path 主語の場合は
    /// `find_by_worktree_mut`、execution_id 主語の場合は `executions.get_mut(execution_id)` で
    /// lookup する。
    async fn set_execution_state<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        new_state: RuntimeExecutionState,
    ) -> Result<(), WorkflowEngineError> {
        self.set_execution_state_inner(
            app,
            session_store,
            agent_runtime,
            ExecutionStateTarget::Worktree(worktree_path.to_string()),
            new_state,
        )
        .await
    }

    /// 実行状態更新の内部実装。lookup 戦略を `target` で切り替える。
    /// Spec issues-1011 finding 10: Execution Store sync 失敗時は engine state も巻き戻し、
    /// engine terminal / Execution Store active のスキューを残さない。
    async fn set_execution_state_inner<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        target: ExecutionStateTarget,
        new_state: RuntimeExecutionState,
    ) -> Result<(), WorkflowEngineError> {
        let (snapshot, execution_id, worktree_path, snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = match &target {
                ExecutionStateTarget::Worktree(wt) => find_by_worktree_mut(&mut execs, wt)
                    .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(wt.clone()))?,
            };
            // 終了状態（Completed/Failed/Aborted）からの上書きを防止
            if exec.is_terminal() {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let timestamp = current_timestamp();
            if let RuntimeExecutionState::Failed { reason, kind, .. } = &new_state {
                exec.fail_current_node_execution_and_abort_active_children(
                    reason, *kind, timestamp,
                );
            }
            exec.state = new_state;
            exec.updated_at = timestamp;
            (
                exec.to_workflow_state(),
                exec.id.clone(),
                exec.worktree_path.clone(),
                snapshot_before,
            )
        };

        let is_terminal = matches!(
            snapshot.state,
            RuntimeExecutionState::Completed
                | RuntimeExecutionState::Failed { .. }
                | RuntimeExecutionState::Aborted
                | RuntimeExecutionState::Interrupted
        );

        // [05] terminal 経路は commit_required_events 基盤の共通 commit 境界に統合する。
        // terminal events (NodeCompleted（Completed のみ）+ ExecutionCompleted / NodeFailed+ExecutionFailed)
        // を required event 列として集約し、ExecutionStore sync → ChatSession persist → event log
        // append の順序で commit する。いずれかが失敗した場合は engine state と Execution Store snapshot
        // を snapshot_before で一括復元する（spec [05] atomic mutation 境界 / best-effort warn 廃止）。
        // Aborted は AbortExecution command handler 側で別途 commit されるため本経路では event 集合に含めない。
        if is_terminal
            && !matches!(
                snapshot.state,
                RuntimeExecutionState::Aborted | RuntimeExecutionState::Interrupted
            )
        {
            let required_events =
                match workflow_runtime_events::terminal_required_events_for_snapshot(&snapshot) {
                    Ok(events) => events,
                    Err(e) => {
                        let mut execs = self.executions.lock().await;
                        if let Some(exec) = execs.get_mut(&execution_id) {
                            *exec = snapshot_before;
                        }
                        return Err(e);
                    }
                };
            let execution_store_snapshot_before = self
                .execution_store
                .active_execution_snapshot(&execution_id)
                .await;
            self.commit_required_events(
                app,
                session_store,
                RequiredEventCommit {
                    execution_id: &execution_id,
                    snapshot_for_commit: &snapshot,
                    snapshot_before,
                    execution_store_snapshot_before,
                    required_events,
                    append_error_context: "set_execution_state terminal event append failed",
                },
            )
            .await?;

            // terminal 副作用: step session release + refs cleanup + broadcast。
            let terminal_session_ids =
                workflow_runtime_commit::terminal_step_session_ids(&snapshot);
            workflow_runtime_session::release_completed_step_sessions(
                app,
                session_store,
                agent_runtime,
                &terminal_session_ids,
            )
            .await;
            self.cleanup_session_workflow_refs_by_execution_id(&execution_id)
                .await;
            workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;
            self.release_terminal_execution(&execution_id).await;
            return Ok(());
        }

        // 非 terminal / Aborted 経路: required event が無いため従来の sync→persist 順で commit する。
        // Aborted は AbortExecution command handler 側で event を別途 append 済み。
        let rollback_engine_state =
            |execution_id_for_rollback: String, previous_snapshot: WorkflowExecution| async move {
                let mut execs = self.executions.lock().await;
                if let Some(exec) = execs.get_mut(&execution_id_for_rollback) {
                    *exec = previous_snapshot;
                }
            };

        if let Err(e) = workflow_runtime_commit::sync_execution_store_from_snapshot(
            &self.execution_store,
            &execution_id,
            &snapshot,
        )
        .await
        {
            rollback_engine_state(execution_id.clone(), snapshot_before).await;
            return Err(e);
        }

        if is_terminal {
            let terminal_session_ids =
                workflow_runtime_commit::terminal_step_session_ids(&snapshot);
            workflow_runtime_session::release_completed_step_sessions(
                app,
                session_store,
                agent_runtime,
                &terminal_session_ids,
            )
            .await;
            self.cleanup_session_workflow_refs_by_execution_id(&execution_id)
                .await;
        }
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;
        if is_terminal {
            self.release_terminal_execution(&execution_id).await;
        }
        Ok(())
    }

    async fn rollback_command_mutation<R: tauri::Runtime>(
        &self,
        _app: &tauri::AppHandle<R>,
        _session_store: &Arc<SessionStore>,
        rollback: CommandMutationRollback<'_>,
    ) -> Result<(), WorkflowEngineError> {
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

    /// autoモードのタグ検出結果を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行する。
    /// contractが設定されたステップではcontract検証を実行し、
    /// 違反時はリトライプロンプトを送信する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_auto_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        _final_parts: &[crate::usecase::agent_session::session::MessagePart],
        node_name: &str,
    ) -> Result<(), WorkflowEngineError> {
        // [08] prose 抽出経路廃止: agent step の structured output は CLI / Tauri 経由の
        // `SubmitOutput` でしか確定しない。contract がある step は、提出済み
        // output が見つからない限り完了扱いにせず、同じ session に修正ターンを投げる。
        let (execution_id, workflow_name, contract, attempt, current_session_id, submitted_output) = {
            let execs = self.executions.lock().await;
            let (execution_id, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let node = &exec.workflow.nodes[exec.current_node_index];
            let contract = node.artifact.clone();
            let attempt = exec
                .node_execution_counts
                .get(&node.name)
                .copied()
                .unwrap_or(1);
            let submitted_output = contract.as_deref().and_then(|contract| {
                workflow_output_submission::submitted_step_output_for(
                    &exec.artifacts,
                    &node.name,
                    attempt,
                    contract,
                )
            });
            (
                execution_id.clone(),
                exec.workflow.name.clone(),
                contract,
                attempt,
                exec.current_session_id.clone(),
                submitted_output,
            )
        };
        let (artifact, contract_result) = if let Some(ref contract) = contract {
            if let Some(output) = submitted_output {
                (output.artifact.clone(), output.result.clone())
            } else {
                self.handle_missing_required_output(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    &execution_id,
                    &workflow_name,
                    node_name,
                    contract,
                    attempt,
                    current_session_id.as_deref(),
                    None,
                    None,
                    SubmissionViolation::MissingSubmitOutput,
                    None,
                )
                .await?;
                return Ok(());
            }
        } else {
            (None, None)
        };
        let _ = attempt;

        let effective_result = contract_result;

        // 判定 + 状態変更 + 履歴記録を原子的に実行
        let (outcome, snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = find_by_worktree_mut(&mut execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let snapshot_before = exec.clone();

            let entry = exec.make_node_history_entry(effective_result, artifact, contract);
            exec.node_history.push(entry);
            let outcome = exec.apply_advance();
            (outcome, snapshot_before)
        };

        self.execute_outcome(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            outcome,
            snapshot_before,
        )
        .await
    }

    async fn start_current_node_runtime<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let kind = {
            let execs = self.executions.lock().await;
            let (_, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            exec.workflow.nodes[exec.current_node_index].kind_name()
        };
        match kind {
            NodeKindName::Command => {
                self.run_current_command_node(app, session_store, agent_runtime, worktree_path)
                    .await
            }
            NodeKindName::Session => {
                self.start_step_session(app, agent_runtime, session_store, worktree_path)
                    .await
            }
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
    ) -> Result<(), WorkflowEngineError> {
        let input = self.command_execution_input(worktree_path).await?;
        self.spawn_command_execution(app, session_store, agent_runtime, input)
            .await
    }

    async fn spawn_command_execution<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        input: CommandExecutionInput,
    ) -> Result<(), WorkflowEngineError> {
        let running = match workflow_command_runner::spawn_shell_command(
            &input.worktree_path,
            &input.command,
            command_env(&input),
        ) {
            Ok(running) => running,
            Err(CommandRunnerError::Spawn(error)) => {
                let reason = format!("failed to spawn command: {error}");
                self.fail_command_execution(app, session_store, agent_runtime, &input, &reason)
                    .await?;
                return Err(WorkflowEngineError::SessionStore(reason));
            }
            Err(error) => {
                let reason = format!("failed to prepare command: {error}");
                self.fail_command_execution(app, session_store, agent_runtime, &input, &reason)
                    .await?;
                return Err(WorkflowEngineError::SessionStore(reason));
            }
        };

        self.active_commands
            .lock()
            .await
            .insert(input.node_execution_id.clone(), running.handle());
        self.active_command_executions
            .lock()
            .await
            .insert(input.node_execution_id.clone(), input.execution_id.clone());
        let engine = self.clone();
        let observer_app = app.clone();
        let observer_session_store = session_store.clone();
        let observer_agent_runtime = agent_runtime.clone();
        let node_execution_id = input.node_execution_id.clone();
        let still_current = self.command_execution_still_current(&input).await;
        let observer_node_execution_id = node_execution_id.clone();
        let runtime_handle = tokio::runtime::Handle::current();
        let observer = tokio::task::spawn_blocking(move || {
            runtime_handle.block_on(async move {
                engine
                    .observe_command_completion(
                        &observer_app,
                        &observer_session_store,
                        &observer_agent_runtime,
                        input,
                        running,
                    )
                    .await;
                engine
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
                    let _ = self
                        .fail_command_execution(
                            app,
                            session_store,
                            agent_runtime,
                            &failure_input,
                            &reason,
                        )
                        .await;
                }
            }
            Err(CommandRunnerError::Cancelled) => {
                let reason = self
                    .command_shutdown_reasons
                    .lock()
                    .await
                    .remove(&input.node_execution_id);
                if matches!(reason, Some(ActiveCommandShutdownReason::AppExit)) {
                    if let Err(error) = self
                        .interrupt_current_command_node(
                            app,
                            session_store,
                            agent_runtime,
                            &input,
                            "app_exit",
                        )
                        .await
                    {
                        log::warn!(
                            "workflow {}: command interruption commit failed: {error}",
                            input.execution_id
                        );
                    }
                }
            }
            Err(error) => {
                let reason = format!("command runtime failed: {error}");
                let _ = self
                    .fail_command_execution(app, session_store, agent_runtime, &input, &reason)
                    .await;
            }
        }
    }

    async fn command_execution_input(
        &self,
        worktree_path: &str,
    ) -> Result<CommandExecutionInput, WorkflowEngineError> {
        let execs = self.executions.lock().await;
        let (execution_id, exec) = find_by_worktree(&execs, worktree_path)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
        let node = &exec.workflow.nodes[exec.current_node_index];
        let Some(command) = node.command() else {
            return Err(WorkflowEngineError::InvalidState(format!(
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
                WorkflowEngineError::InvalidState(format!(
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
            command: rendered_command,
            contract: node.artifact.clone(),
            schemas: workflow_schemas_to_domain(&exec.workflow.schemas),
            fanout_parent: None,
        })
    }

    async fn command_execution_still_current(&self, input: &CommandExecutionInput) -> bool {
        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(&input.execution_id) else {
            return false;
        };
        let execution_is_active = exec.node_executions.iter().any(|node_execution| {
            node_execution.id == input.node_execution_id && node_execution.status.is_active()
        });
        if input.fanout_parent.is_some() {
            execution_is_active
                && exec.parallel_run.as_ref().is_some_and(|run| {
                    run.children.iter().any(|child| {
                        child.node_execution_id == input.node_execution_id
                            && child.state == FanoutChildRuntimeState::Running
                    })
                })
        } else {
            is_still_current_execution(exec, &input.node_name, input.attempt) && execution_is_active
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_command_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        input: CommandExecutionInput,
        output: CommandRunOutput,
    ) -> Result<(), WorkflowEngineError> {
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
            let entry = exec.make_node_history_entry(result_summary, None, None);
            let completed_at = entry.completed_at;
            let attempt = entry.attempt;
            exec.node_history.push(entry);
            exec.artifacts.insert(
                input.node_name.clone(),
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
            );
            if let Some(node_execution) = exec
                .node_executions
                .iter_mut()
                .find(|node_execution| node_execution.id == input.node_execution_id)
            {
                node_execution.status = NodeExecutionStatus::Succeeded;
                node_execution.artifact = Some(artifact_value.clone());
                node_execution.completed_at = Some(timestamp);
            }
            let outcome = exec.apply_advance();
            (
                outcome,
                snapshot_before,
                exec.to_workflow_state(),
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
                execution_id: &input.execution_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                execution_store_snapshot_before,
                required_events,
                append_error_context: "command completion event append failed",
            },
        )
        .await?;
        self.finalize_after_commit(app, &snapshot_for_commit, &worktree_path, false)
            .await;
        Box::pin(self.dispatch_step_outcome_side_effects(
            app,
            session_store,
            agent_runtime,
            &worktree_path,
            outcome,
            OutcomeCommitMode::ProgressEventsAlreadyCommitted,
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
    ) -> Result<(), WorkflowEngineError> {
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
                .parallel_run
                .as_mut()
                .and_then(|run| {
                    run.children
                        .iter_mut()
                        .find(|child| child.node_execution_id == input.node_execution_id)
                })
                .ok_or_else(|| {
                    WorkflowEngineError::InvalidState(format!(
                        "active fanout command child '{}' was not found",
                        input.node_execution_id
                    ))
                })?;
            child.state = FanoutChildRuntimeState::Completed;
            child.result = Some(result_summary.clone());
            child.artifact = Some(artifact_value.clone());
            child.failure_kind = None;
            child.failure_disposition = None;
            child.completed_at = Some(completed_at);
            if let Some(node_execution) = execution
                .node_executions
                .iter_mut()
                .find(|node_execution| node_execution.id == input.node_execution_id)
            {
                node_execution.status = NodeExecutionStatus::Succeeded;
                node_execution.artifact = Some(artifact_value.clone());
                node_execution.failure = None;
                node_execution.completed_at = Some(completed_at);
            }
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
            self.commit_required_parallel_progress_events_and_execute_outcome(
                app,
                session_store,
                agent_runtime,
                &input.worktree_path,
                outcome,
                completion.snapshot_before,
                completion.progress_events,
                &[],
                completion.failure_telemetry,
            )
            .await?;
        }
        Ok(())
    }

    fn command_input_is_active_fanout_child(
        &self,
        execution: &WorkflowExecution,
        input: &CommandExecutionInput,
    ) -> bool {
        execution.is_active()
            && execution.node_executions.iter().any(|node_execution| {
                node_execution.id == input.node_execution_id
                    && node_execution.status == NodeExecutionStatus::Running
            })
            && execution.parallel_run.as_ref().is_some_and(|run| {
                input.fanout_parent.as_deref() == Some(run.parent_node_name.as_str())
                    && run.children.iter().any(|child| {
                        child.node_execution_id == input.node_execution_id
                            && child.state == FanoutChildRuntimeState::Running
                    })
            })
    }

    async fn fail_command_execution<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        input: &CommandExecutionInput,
        reason: &str,
    ) -> Result<(), WorkflowEngineError> {
        if input.fanout_parent.is_none() {
            return self
                .fail_current_command_node(
                    app,
                    session_store,
                    agent_runtime,
                    &input.execution_id,
                    reason,
                )
                .await;
        }

        let failure_kind = NodeExecutionFailureKind::InfrastructureCrash;
        let timestamp = current_timestamp();
        let (completion, interrupted_session_ids, interrupted_command_ids) = {
            let mut executions = self.executions.lock().await;
            let Some(execution) = executions.get_mut(&input.execution_id) else {
                return Ok(());
            };
            if !self.command_input_is_active_fanout_child(execution, input) {
                return Ok(());
            }
            let snapshot_before = execution.clone();
            let terminal_reason = format!("fanout child '{}' failed: {reason}", input.node_name);
            let failure_commit = finalize_fanout_child_failure_state(
                execution,
                snapshot_before,
                FanoutChildFailureInput {
                    child_node_execution_id: input.node_execution_id.clone(),
                    child_failure_reason: reason.to_string(),
                    terminal_reason,
                    failure_kind,
                    retry_count: None,
                    timestamp,
                    record_child_token_usage: false,
                },
            )?;
            (
                failure_commit.completion,
                failure_commit.interrupted_session_ids,
                failure_commit.interrupted_command_ids,
            )
        };

        if let Some(outcome) = completion.outcome {
            self.commit_required_parallel_progress_events_and_execute_outcome(
                app,
                session_store,
                agent_runtime,
                &input.worktree_path,
                outcome,
                completion.snapshot_before,
                completion.progress_events,
                &[],
                completion.failure_telemetry,
            )
            .await?;
        }
        for session_id in interrupted_session_ids {
            workflow_runtime_session::interrupt_agent(agent_runtime, &session_id).await;
        }
        if !interrupted_command_ids.is_empty() {
            let handles = self.active_commands.lock().await;
            for execution_id in interrupted_command_ids {
                if let Some(handle) = handles.get(&execution_id) {
                    handle.request_shutdown();
                }
            }
        }
        Ok(())
    }

    async fn fail_current_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        execution_id: &str,
        reason: &str,
    ) -> Result<(), WorkflowEngineError> {
        let worktree_path = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(execution_id) else {
                return Ok(());
            };
            exec.worktree_path.clone()
        };
        self.set_execution_state(
            app,
            session_store,
            agent_runtime,
            &worktree_path,
            RuntimeExecutionState::Failed {
                reason: reason.to_string(),
                kind: NodeExecutionFailureKind::InfrastructureCrash,
                retry_count: None,
            },
        )
        .await
    }

    async fn interrupt_current_command_node<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        _agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        input: &CommandExecutionInput,
        reason: &str,
    ) -> Result<(), WorkflowEngineError> {
        let (snapshot, snapshot_before, worktree_path) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(&input.execution_id) else {
                return Ok(());
            };
            let command_is_active = exec.node_executions.iter().any(|node_execution| {
                node_execution.id == input.node_execution_id
                    && node_execution.node_name == input.node_name
                    && node_execution.attempt == input.attempt
                    && node_execution.status.is_active()
            });
            if exec.is_terminal() || !command_is_active {
                return Ok(());
            }

            let snapshot_before = exec.clone();
            let timestamp = current_timestamp();
            for node_execution in exec
                .node_executions
                .iter_mut()
                .filter(|node_execution| node_execution.status.is_active())
            {
                node_execution.status = NodeExecutionStatus::Aborted;
                node_execution.completed_at = Some(timestamp);
            }
            if let Some(fanout) = exec.parallel_run.as_mut() {
                for child in fanout
                    .children
                    .iter_mut()
                    .filter(|child| child.state == FanoutChildRuntimeState::Running)
                {
                    child.state = FanoutChildRuntimeState::Interrupted;
                    child.completed_at = Some(timestamp);
                }
            }
            let mut entry = exec.make_node_history_entry(None, None, None);
            entry.state = NODE_STATUS_INTERRUPTED.to_string();
            exec.node_history.push(entry);
            exec.parallel_run = None;
            exec.state = RuntimeExecutionState::Interrupted;
            exec.error_reason = Some(reason.to_string());
            exec.updated_at = timestamp;
            (
                exec.to_workflow_state(),
                snapshot_before,
                exec.worktree_path.clone(),
            )
        };

        let execution_store_snapshot_before = self
            .execution_store
            .active_execution_snapshot(&input.execution_id)
            .await;
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                execution_id: &input.execution_id,
                snapshot_for_commit: &snapshot,
                snapshot_before,
                execution_store_snapshot_before,
                required_events: vec![WorkflowEvent::ExecutionInterrupted {
                    execution_id: input.execution_id.clone(),
                    reason: reason.to_string(),
                    timestamp: snapshot.updated_at,
                }],
                append_error_context: "ExecutionInterrupted log failed",
            },
        )
        .await?;
        self.finalize_after_commit(app, &snapshot, &worktree_path, false)
            .await;
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
        self.command_shutdown_reasons
            .lock()
            .await
            .remove(node_execution_id);
        self.active_command_executions
            .lock()
            .await
            .remove(node_execution_id);
    }

    async fn shutdown_active_commands_for_execution(&self, execution_id: &str) {
        let node_execution_ids = self
            .active_command_executions
            .lock()
            .await
            .iter()
            .filter_map(|(node_execution_id, owner_execution_id)| {
                (owner_execution_id == execution_id).then_some(node_execution_id.clone())
            })
            .collect::<Vec<_>>();
        for node_execution_id in node_execution_ids {
            self.shutdown_active_command_execution(&node_execution_id)
                .await;
        }
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
            let mut reasons = self.command_shutdown_reasons.lock().await;
            for (node_execution_id, _) in &commands {
                reasons.insert(
                    node_execution_id.clone(),
                    ActiveCommandShutdownReason::AppExit,
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
            let mut reasons = self.command_shutdown_reasons.lock().await;
            for node_execution_id in &node_execution_ids {
                reasons.remove(node_execution_id);
            }
        }
        let mut executions = self.active_command_executions.lock().await;
        for node_execution_id in &node_execution_ids {
            executions.remove(node_execution_id);
        }
    }

    /// 現在のステップ用に新しいChatSessionを生成し、AgentSessionを開始してプロンプトを送信する。
    /// ファセット方式と旧prompt方式を自動判別する。
    ///
    /// production 経路。副作用境界を `RealStepSessionDeps` にラップし、コアロジック
    /// `start_step_session_with_deps` に委譲する。
    async fn start_step_session<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        session_store: &Arc<SessionStore>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let deps = RealStepSessionDeps {
            app,
            branch_diff_context: self.branch_diff_context.clone(),
            agent_runtime,
            session_store,
            open_tabs: &self.open_tabs,
        };
        self.start_step_session_with_deps(&deps, worktree_path)
            .await
    }

    /// `start_step_session` のコアロジック。副作用境界は `StepSessionDeps` 経由で注入する。
    ///
    /// 呼び出し順序の不変条件:
    /// 1. `build_step_prompt`（純粋関数）でプロンプト合成
    /// 2. `deps.create_step_session`（`exec.workflow_defaults` を継承元に注入）
    /// 3. `session_workflow_refs` への登録
    /// 4. `deps.dispatch_session_start`（AgentSession 開始）
    /// 5. `executions.current_session_id` 更新
    /// 6. `NodeSessionStarted` append とブロードキャスト
    /// 7. `deps.start_agent_turn`（ターン起動）
    ///
    /// 1 で失敗した場合、2 以降は一切実行されない（合成失敗時に
    /// ChatSession 生成や `session_workflow_refs` への孤立 entry が残らない）。
    /// テストではこの順序保証を `StepSessionDeps` のテストダブル経由で検証する。
    async fn start_step_session_with_deps<D: StepSessionDeps + ?Sized>(
        &self,
        deps: &D,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let (
            execution_id_for_ref,
            step_clone,
            artifacts_clone,
            task_clone,
            workflow_defaults_clone,
            workflow_node_context,
            workflow_clone,
        ) = {
            let execs = self.executions.lock().await;
            let (execution_id, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.nodes[exec.current_node_index];
            let step_attempt = exec
                .node_execution_counts
                .get(&step.name)
                .copied()
                .unwrap_or(1);
            let node_execution_id = exec
                .node_executions
                .iter()
                .rev()
                .find(|node_execution| {
                    node_execution.node_name == step.name
                        && node_execution.attempt == step_attempt
                        && node_execution.fanout_parent.is_none()
                        && node_execution.status.is_active()
                })
                .map(|node_execution| node_execution.id.clone())
                .ok_or_else(|| {
                    WorkflowEngineError::InvalidState(format!(
                        "active NodeExecution for '{}' attempt {} is unavailable",
                        step.name, step_attempt
                    ))
                })?;
            (
                execution_id.clone(),
                step.clone(),
                exec.artifacts.clone(),
                exec.request.clone(),
                exec.workflow_defaults.clone(),
                WorkflowNodeContext {
                    execution_id: execution_id.clone(),
                    node_execution_id,
                    workflow_name: exec.workflow.name.clone(),
                    node_name: step.name.clone(),
                    attempt: step_attempt,
                    parent_node_name: None,
                    parent_attempt: None,
                    order: exec.node_history.len() as u32,
                    startup_timeout_secs: None,
                    startup_max_retries: None,
                    stale_timeout_secs: None,
                },
                exec.workflow.clone(),
            )
        };
        let facet_contents = self
            .facet_contents_for_execution(&execution_id_for_ref, &workflow_clone)
            .await?;
        let step_facet_contents = facet_contents.for_node(&step_clone.name);

        // プロンプト合成（純粋関数）を最初に行う。
        // ここで失敗（参照先ファセットが存在しない等）した場合、後続の
        // ChatSession 生成・`session_workflow_refs` 登録・AgentSession 開始は一切
        // 行われない。これにより、`start_step_session` がエラー経路で孤立した
        // ChatSession や参照マップ entry を残さないことを構造的に保証する。
        let (system_prompt, prompt) = workflow_prompt::build_step_prompt(
            &step_clone,
            step_facet_contents,
            &execution_id_for_ref,
            task_clone.as_deref(),
            &artifacts_clone,
        )?;
        let workflow_instruction = workflow_prompt::render_step_workflow_instruction(
            &step_clone,
            step_facet_contents,
            task_clone.as_deref(),
            &artifacts_clone,
        );
        // ステップ設定の解決 → セッション生成（workflow_defaults を継承元に注入）
        let node_execution_id = workflow_node_context.node_execution_id.clone();
        let step_session = deps
            .create_step_session(
                worktree_path,
                step_clone
                    .session()
                    .and_then(|session| session.model.clone()),
                step_clone
                    .session()
                    .and_then(|session| session.permission.clone()),
                workflow_defaults_clone,
                workflow_node_context,
                workflow_runtime_session::StepRuntimeKindContext::new(
                    node_kind_to_domain(&step_clone.kind).name(),
                    step_clone.is_approval_session(),
                ),
            )
            .await?;
        let permission_mode = step_session.permission_mode.clone();
        let step_session_id = step_session.id.clone();

        // ステップセッションID → SessionWorkflowRefのマッピングを登録
        {
            let mut map = self.session_workflow_refs.lock().await;
            map.insert(
                step_session_id.clone(),
                SessionWorkflowRef {
                    execution_id: execution_id_for_ref.clone(),
                },
            );
        }

        // 合成済み system_prompt を AgentSession 起動経路へ受け渡す。
        deps.dispatch_session_start(
            &step_session_id,
            worktree_path,
            None,
            system_prompt.clone(),
            workflow_instruction.clone(),
        )
        .await?;
        deps.mark_step_tab_open(&step_session_id).await;

        // ステップセッションIDをワークフロー実行に紐付け
        let snapshot = {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(&execution_id_for_ref) {
                exec.current_session_id = Some(step_session_id.clone());
                if let Some(node_execution) = exec
                    .node_executions
                    .iter_mut()
                    .find(|node_execution| node_execution.id == node_execution_id)
                {
                    node_execution.session_id = Some(step_session_id.clone());
                }
                Some(exec.to_workflow_state())
            } else {
                None
            }
        };

        if let Some(snapshot) = snapshot {
            deps.append_node_session_started(&snapshot).await?;
            deps.broadcast_state(worktree_path, snapshot).await;
        }

        // プロンプト送信（ステップ用セッションIDを使用）
        deps.start_agent_turn_locked(
            &step_session_id,
            worktree_path,
            &permission_mode,
            &prompt,
            system_prompt,
            workflow_instruction,
        )
        .await
    }

    /// `build_step_prompt` で合成した `system_prompt` を `dispatch_session_start` 経由で
    /// gate に渡し、`prompt`（user_message 由来）を返すテスト用ヘルパー。
    ///
    /// production では `start_step_session` 内で `build_step_prompt` →
    /// `create_step_session_with_settings` → `dispatch_session_start` を順に呼ぶ
    /// 構造にしている（プロンプト合成失敗時に ChatSession・参照マップ登録が起きない
    /// 順序保証のため）。テストでは記録用 gate を注入することで、合成された
    /// `system_prompt` が None や空文字に置換されずバックエンドへ受け渡される
    /// 経路を直接検証する。
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    async fn build_and_dispatch_step_session<G: SessionStartGate + ?Sized>(
        gate: &G,
        step: &NodeDefinition,
        facet_contents: Option<&crate::adaptor::gateway::workflow::facet::FacetContents>,
        execution_id: &str,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        request: Option<&str>,
        artifacts: &HashMap<String, RuntimeArtifact>,
    ) -> Result<String, WorkflowEngineError> {
        let (system_prompt, prompt) = workflow_prompt::build_step_prompt(
            step,
            facet_contents,
            execution_id,
            request,
            artifacts,
        )?;
        dispatch_session_start(
            gate,
            step_session_id,
            worktree_path,
            permission_mode,
            system_prompt,
            None,
        )
        .await?;
        Ok(prompt)
    }

    /// [08] prose 抽出経路は engine から完全除去された（spec [08] Rule 4 構造化出力の
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
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
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
        session_store: &Arc<SessionStore>,
        commit: RequiredEventCommit<'_>,
    ) -> Result<(), WorkflowEngineError> {
        let RequiredEventCommit {
            execution_id,
            snapshot_for_commit,
            snapshot_before,
            execution_store_snapshot_before,
            required_events,
            append_error_context,
        } = commit;

        if let Err(e) = self.write_log_required_batch(app, &required_events) {
            let _ = self
                .rollback_command_mutation(
                    app,
                    session_store,
                    CommandMutationRollback {
                        execution_id,
                        snapshot_before,
                        execution_store_snapshot_before,
                        context: append_error_context,
                    },
                )
                .await;
            return Err(WorkflowEngineError::SessionStore(format!(
                "{append_error_context}: {e}"
            )));
        }

        let projection_error_context = "required event projection failed";
        if let Err(e) = self
            .sync_state_after_required_event_commit(snapshot_for_commit)
            .await
        {
            return Err(WorkflowEngineError::SessionStore(format!(
                "{projection_error_context}: {e}"
            )));
        }

        record_failed_snapshot_telemetry(snapshot_for_commit);
        Ok(())
    }

    /// [04] pre-commit phase: sync_execution_store + release_completed_step_sessions を実行する。
    /// 本 helper は本 issue scope 外の non-command 経路（NodeCompleted/NodeFailed 系の
    /// `persist_release_and_broadcast` 呼び出し）専用に温存する。
    /// 本 issue scope の command 受理 handler は required event append 前の rollback 可能な
    /// projection と post-commit `release_completed_step_sessions` の組み合わせを使う。
    async fn sync_persist_release<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        snapshot: &WorkflowState,
        completed_step_session_ids: &[String],
    ) -> Result<(), WorkflowEngineError> {
        let execution_id = snapshot.execution_id.clone();
        if let Err(e) = workflow_runtime_commit::sync_execution_store_from_snapshot(
            &self.execution_store,
            &execution_id,
            snapshot,
        )
        .await
        {
            workflow_runtime_commit::rollback_execution_projection_after_execution_store_sync_failure(
                &self.executions,
                &self.execution_store,
                &execution_id,
                snapshot,
            )
            .await;
            return Err(e);
        }
        workflow_runtime_session::release_completed_step_sessions(
            app,
            session_store,
            agent_runtime,
            completed_step_session_ids,
        )
        .await;
        Ok(())
    }

    /// [04] post-commit phase: terminal log + cleanup_refs + broadcast。required append
    /// 完了後の副作用に限定し、失敗は warn として観測する（command 結果には伝播しない）。
    async fn finalize_after_commit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &WorkflowState,
        worktree_path: &str,
        write_terminal_events: bool,
    ) {
        let execution_id = snapshot.execution_id.clone();
        let is_terminal = matches!(
            snapshot.state,
            RuntimeExecutionState::Completed
                | RuntimeExecutionState::Failed { .. }
                | RuntimeExecutionState::Aborted
                | RuntimeExecutionState::Interrupted
        );
        if is_terminal {
            if write_terminal_events {
                if matches!(snapshot.state, RuntimeExecutionState::Completed) {
                    if let Err(e) = self.write_last_step_completed_log(app, snapshot) {
                        log::warn!("Failed to append NodeCompleted workflow event: {e}");
                    }
                }
                if let Err(e) = self.write_terminal_log(app, snapshot) {
                    log::warn!("Failed to append terminal workflow events: {e}");
                }
            }
            self.cleanup_session_workflow_refs_by_execution_id(&execution_id)
                .await;
        }
        workflow_runtime_session::broadcast_state(app, worktree_path, snapshot.clone()).await;
        if is_terminal {
            self.release_terminal_execution(&execution_id).await;
        }
    }

    /// 既存呼び出し元（on_turn_complete 等）から使う一括 helper。pre-commit と post-commit
    /// を順に呼ぶだけで、外部 contract は変えない。
    #[allow(clippy::too_many_arguments)]
    async fn persist_release_and_broadcast<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        snapshot: WorkflowState,
        completed_step_session_ids: &[String],
    ) -> Result<WorkflowState, WorkflowEngineError> {
        self.sync_persist_release(
            app,
            session_store,
            agent_runtime,
            &snapshot,
            completed_step_session_ids,
        )
        .await?;
        self.finalize_after_commit(app, &snapshot, worktree_path, true)
            .await;
        Ok(snapshot)
    }

    /// ロック外でStepOutcomeに応じた副作用（永続化・ブロードキャスト・AgentSession起動）を実行する。
    ///
    /// 本 helper は non-command 経路（NodeCompleted / NodeFailed 等）から呼ばれる。
    ///
    /// [05] commit 境界: spec [04] commit_required_events を基盤に、StepOutcome から
    /// `NodeCompleted` / `NodeFailed` / `ExecutionCompleted` / `ExecutionFailed` の必須 event を
    /// 組み立て、ExecutionStore sync → ChatSession persist → event log append の順で commit
    /// する。いずれかの phase で失敗した場合は engine state と Execution Store snapshot を
    /// `snapshot_before` で一括復元することで、event log と engine state / ExecutionStore /
    /// ChatSession の分離を防ぐ（spec [05]: state mutation と event log の分離を防ぐ
    /// rollback 境界 / atomic mutation 境界）。
    ///
    /// 必須 event が空の場合は従来通り `sync_persist_release` のみを実行する。
    async fn execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        outcome: StepOutcome,
        snapshot_before: WorkflowExecution,
    ) -> Result<(), WorkflowEngineError> {
        let completed_step_session_ids = outcome.completed_step_session_ids();
        let snapshot_for_commit = outcome.snapshot().clone();
        let execution_id = snapshot_for_commit.execution_id.clone();

        // [05] pre-commit phase: 必須 event の生成。`dispatch_internal_node_command` の
        // ValidationError は engine state を snapshot_before で復元して伝播する
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
            // 失敗時は engine state と Execution Store snapshot を一括復元する。
            let execution_store_snapshot_before = self
                .execution_store
                .active_execution_snapshot(&execution_id)
                .await;
            self.commit_required_events(
                app,
                session_store,
                RequiredEventCommit {
                    execution_id: &execution_id,
                    snapshot_for_commit: &snapshot_for_commit,
                    snapshot_before,
                    execution_store_snapshot_before,
                    required_events: pre_commit_events,
                    append_error_context: "execute_outcome required event append failed",
                },
            )
            .await?;
            workflow_runtime_session::release_completed_step_sessions(
                app,
                session_store,
                agent_runtime,
                &completed_step_session_ids,
            )
            .await;
        } else {
            // 必須 event 無し: 従来通り sync_persist_release のみ。
            self.sync_persist_release(
                app,
                session_store,
                agent_runtime,
                &snapshot_for_commit,
                &completed_step_session_ids,
            )
            .await?;
        }

        // terminal / NodeCompleted は append 済みのため finalize_after_commit には
        // write_terminal_events=false を渡し二重 append を避ける（commit 境界の単一性）。
        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path, false)
            .await;
        self.dispatch_step_outcome_side_effects(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            outcome,
            OutcomeCommitMode::ProgressEventsAlreadyCommitted,
        )
        .await
    }

    /// [04] post-commit variant work（共通 side-effect helper）。
    ///
    /// snapshot は既に persist 済みである前提で、outcome variant に応じた残りの副作用
    /// （NodeStarted 書き込み・start_step_session・reduce + 派生 mutation の再帰・
    /// start_fanout_children・auto-approve approval primitive）のみを担当する。`execute_outcome`
    /// （non-command 経路）と `handle_approval` などの 4 command handler の双方から
    /// 呼ばれ、副作用ロジックの単一 source of truth として機能する。失敗は warn 化して
    /// command 結果に伝播させない設計に揃える（spec [04] post-commit 境界）。
    async fn dispatch_step_outcome_side_effects<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        outcome: StepOutcome,
        commit_mode: OutcomeCommitMode,
    ) -> Result<(), WorkflowEngineError> {
        match outcome {
            StepOutcome::Persist(snapshot) => {
                if let Some((execution_id, node_name)) =
                    workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(
                        &snapshot,
                        workflow_approval_runtime::workflow_approval_auto_approve_enabled(app),
                    )
                {
                    return Box::pin(self.resolve_workflow_approval(
                        app,
                        session_store,
                        agent_runtime,
                        &execution_id,
                        None,
                        &node_name,
                        None,
                    ))
                    .await;
                }
                Ok(())
            }
            StepOutcome::RetryCurrentStep { snapshot, .. } => {
                if commit_mode.should_emit_progress_events() {
                    self.write_log(
                        app,
                        workflow_runtime_events::node_started_event_for_snapshot(&snapshot)?,
                    );
                }
                if let Err(e) = Box::pin(self.start_current_node_runtime(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                ))
                .await
                {
                    let failed_state =
                        workflow_runtime_session::record_post_commit_runtime_start_failure(
                            &self.executions,
                            worktree_path,
                            RuntimeStartFailureKind::StepSession,
                            &e,
                        )
                        .await;
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            agent_runtime,
                            worktree_path,
                            failed_state,
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
            StepOutcome::TransitionAndStart(snapshot) => {
                self.emit_post_commit_progress_events(
                    app,
                    commit_mode,
                    workflow_runtime_events::PostCommitProgressEventPlan::TransitionAndStart,
                    &snapshot,
                )?;
                if let Err(e) = Box::pin(self.start_current_node_runtime(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                ))
                .await
                {
                    let failed_state =
                        workflow_runtime_session::record_post_commit_runtime_start_failure(
                            &self.executions,
                            worktree_path,
                            RuntimeStartFailureKind::StepSession,
                            &e,
                        )
                        .await;
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            agent_runtime,
                            worktree_path,
                            failed_state,
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
            StepOutcome::StartParallel(snapshot) => {
                self.emit_post_commit_progress_events(
                    app,
                    commit_mode,
                    workflow_runtime_events::PostCommitProgressEventPlan::StartParallel,
                    &snapshot,
                )?;
                if let Err(e) = Box::pin(self.start_fanout_children(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                ))
                .await
                {
                    let failed_state =
                        workflow_runtime_session::record_post_commit_runtime_start_failure(
                            &self.executions,
                            worktree_path,
                            RuntimeStartFailureKind::ParallelChildren,
                            &e,
                        )
                        .await;
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            agent_runtime,
                            worktree_path,
                            failed_state,
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
        }
    }

    fn emit_post_commit_progress_events<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        commit_mode: OutcomeCommitMode,
        plan: workflow_runtime_events::PostCommitProgressEventPlan,
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        if !commit_mode.should_emit_progress_events() {
            return Ok(());
        }
        if let Err(e) = self.write_last_step_completed_log(app, snapshot) {
            return Err(plan.node_completed_append_error(e));
        }
        if let Some(event) = plan.followup_event(snapshot)? {
            self.write_log(app, event);
        }
        Ok(())
    }

    /// fanout の子 node execution を展開して起動する。
    #[allow(clippy::too_many_arguments)]
    async fn start_fanout_children<R: tauri::Runtime + 'static>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let workflow_runtime_session::FanoutStartRuntimeInputs {
            fanout_start,
            prompt_inputs,
        } = workflow_runtime_session::load_fanout_start_runtime_inputs(
            &self.executions,
            worktree_path,
        )
        .await?;
        let workflow_for_facets = {
            let execs = self.executions.lock().await;
            let (_execution_id, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            exec.workflow.clone()
        };
        let facet_contents = self
            .facet_contents_for_execution(&fanout_start.execution_id, &workflow_for_facets)
            .await?;
        let command_artifacts = workflow_prompt::artifact_values(
            &prompt_inputs.artifacts,
            fanout_start.request.as_deref(),
        );
        let command_schemas = workflow_schemas_to_domain(&workflow_for_facets.schemas);
        let command_inputs = fanout_start
            .children
            .iter()
            .filter_map(|child| {
                let command = child.node.command()?;
                Some(CommandExecutionInput {
                    execution_id: fanout_start.execution_id.clone(),
                    node_execution_id: child.node_execution_id.clone(),
                    node_name: child.node.name.clone(),
                    attempt: child.attempt,
                    worktree_path: worktree_path.to_string(),
                    command: workflow_prompt::render_artifact_references(
                        command,
                        &command_artifacts,
                        child.item.as_ref(),
                    ),
                    contract: child.node.artifact.clone(),
                    schemas: command_schemas.clone(),
                    fanout_parent: Some(fanout_start.parent_node_name.clone()),
                })
            })
            .collect::<Vec<_>>();

        // Phase 1: セッション生成 + ref登録 + プロンプト構築（AgentSessionはまだ起動しない）
        let child_setups = workflow_runtime_session::prepare_fanout_child_session_setups(
            app,
            agent_runtime.backend_registry(),
            session_store,
            &self.session_workflow_refs,
            worktree_path,
            &fanout_start,
            &prompt_inputs,
            &facet_contents,
        )
        .await?;

        let timestamp = current_timestamp();
        let (snapshot_before, snapshot) = {
            let mut executions = self.executions.lock().await;
            let execution = find_by_worktree_mut(&mut executions, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let snapshot_before = execution.clone();
            let snapshot = workflow_parallel_runtime::apply_fanout_runtime_state(
                execution,
                &fanout_start,
                &child_setups,
                timestamp,
            )?;
            (snapshot_before, snapshot)
        };

        // Child IDs are allocated during expansion, after the parent transition commit. Commit
        // their NodeStarted facts atomically with the expanded live state before activating any
        // runtime; otherwise replay could miss children that are already running in memory.
        if !fanout_start.children.is_empty() {
            let started_events = fanout_start
                .children
                .iter()
                .map(|child| WorkflowEvent::NodeStarted {
                    execution_id: fanout_start.execution_id.clone(),
                    node_execution_id: child.node_execution_id.clone(),
                    node_name: child.node.name.clone(),
                    kind: child.node.kind_name(),
                    attempt: child.attempt,
                    fanout_parent: Some(
                        crate::adaptor::gateway::workflow::event::FanoutParentRef {
                            parent_node: fanout_start.parent_node_name.clone(),
                            parent_attempt: fanout_start.parent_attempt,
                            item_index: child.item_index,
                            child_index: child.child_index,
                        },
                    ),
                    timestamp,
                })
                .collect::<Vec<_>>();
            let execution_store_snapshot_before = self
                .execution_store
                .active_execution_snapshot(&fanout_start.execution_id)
                .await;
            if let Err(error) = self
                .commit_required_events(
                    app,
                    session_store,
                    RequiredEventCommit {
                        execution_id: &fanout_start.execution_id,
                        snapshot_for_commit: &snapshot,
                        snapshot_before: snapshot_before.clone(),
                        execution_store_snapshot_before,
                        required_events: started_events,
                        append_error_context: "fanout child NodeStarted append failed",
                    },
                )
                .await
            {
                return Err(
                    workflow_runtime_session::rollback_prepared_fanout_child_sessions(
                        app,
                        session_store,
                        &self.session_workflow_refs,
                        &child_setups,
                        error,
                    )
                    .await,
                );
            }
        }

        if fanout_start.children.is_empty() {
            let completion = {
                let mut executions = self.executions.lock().await;
                let execution =
                    executions
                        .get_mut(&fanout_start.execution_id)
                        .ok_or_else(|| {
                            WorkflowEngineError::ExecutionNotFound(
                                fanout_start.execution_id.clone(),
                            )
                        })?;
                complete_fanout_parent_after_all_children(
                    execution,
                    snapshot_before,
                    Vec::new(),
                    true,
                    None,
                )?
            };
            if let Some(outcome) = completion.outcome {
                self.commit_required_parallel_progress_events_and_execute_outcome(
                    app,
                    session_store,
                    agent_runtime,
                    worktree_path,
                    outcome,
                    completion.snapshot_before,
                    completion.progress_events,
                    &[],
                    completion.failure_telemetry,
                )
                .await?;
            }
            return Ok(());
        }

        let observer = FanoutChildStartedLogObserver {
            engine: self,
            app,
            execution_id: &fanout_start.execution_id,
        };
        workflow_runtime_session::activate_fanout_child_sessions(
            app,
            self.branch_diff_context.clone(),
            session_store,
            agent_runtime,
            &self.open_tabs,
            worktree_path,
            &child_setups,
            snapshot,
            &observer,
        )
        .await?;

        for input in command_inputs {
            self.spawn_command_execution(app, session_store, agent_runtime, input)
                .await?;
        }

        Ok(())
    }

    /// 終了状態（Completed/Failed）のログを書き込む required append helper。
    /// StepCompletedログは呼び出し元で書き込み済みのため、ここでは書かない。
    ///
    /// `Aborted` 状態の `ExecutionAborted` event は本 issue [04] の典型 typed command
    /// `AbortExecution` に対応する事実列であり、command handler 側で `write_log_required`
    /// を経由して必須 append + snapshot 一括復元の atomic 境界に乗せる。本ヘルパーは
    /// `AbortExecution` の rollback 経路を担保できないため Aborted はここで書かない（重複
    /// append 防止）。
    ///
    /// [05] event 発行点の集約: terminal events（NodeFailed / ExecutionCompleted / ExecutionFailed）は
    /// `dispatch_internal_node_command` 経由で生成し、`write_log_required_batch` で必須
    /// append 経路に乗せる。append 失敗時は `Err` を返し、呼出側で state mutation
    /// rollback / persist スキップに乗せる（spec [05]: best-effort warn を廃止し
    /// commit 境界に揃える）。
    fn write_terminal_log<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &WorkflowState,
    ) -> Result<(), String> {
        let events = workflow_runtime_events::terminal_events_for_append(snapshot)?;
        if events.is_empty() {
            return Ok(());
        }
        self.write_log_required_batch(app, &events)
    }

    /// 最後のステップの NodeCompleted ログを書き込む required append helper。
    /// [05] event 発行点の集約: `dispatch_internal_node_command` 経由で生成した
    /// `NodeCompleted` を `write_log_required` で必須 append 経路に乗せる。
    /// append 失敗時は `Err` を返し、呼出側で commit 境界に乗せる（spec [05]:
    /// best-effort warn を廃止）。
    fn write_last_step_completed_log<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &WorkflowState,
    ) -> Result<(), String> {
        match workflow_runtime_events::last_step_completed_event_for_append(snapshot)? {
            Some(event) => self.write_log_required(app, event),
            None => Ok(()),
        }
    }

    /// NDJSONログにイベントを書き込む。失敗してもワークフロー実行には影響しない。
    fn write_log<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, event: WorkflowEvent) {
        if let Err(e) = self.write_log_required(app, event) {
            log::warn!("Failed to write workflow log: {e}");
        }
    }

    /// NDJSONログにイベントを書き込む。履歴復元に必須のログでのみ失敗を伝播する。
    fn write_log_required<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        event: WorkflowEvent,
    ) -> Result<(), String> {
        // [08] テスト fixture (`fail_next_required_event_append_for_test`) を
        // 単発の write_log_required 経路でも観測できるよう、内部で batch helper に
        // 委譲する。production の振る舞いは変わらず、SubmitOutput 等の rollback
        // テストが append 失敗を再現できる。
        self.write_log_required_batch(app, std::slice::from_ref(&event))
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
        #[cfg(test)]
        if self
            .fail_next_required_event_append
            .swap(false, Ordering::AcqRel)
        {
            return Err("injected required event append failure".to_string());
        }
        workflow_event_log_writer::append_required_events_for_app(app, events)
    }

    #[cfg(test)]
    async fn handle_approval_with_output_for_test(
        &self,
        worktree_path: &str,
        expected_execution_id: Option<&str>,
        expected_node_name: Option<&str>,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let execution_id = {
            let execs = self.executions.lock().await;
            let (execution_id, _) = find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string())
            })?;
            execution_id.clone()
        };
        self.handle_approval_with_output_for_run_for_test(
            &execution_id,
            expected_execution_id,
            expected_node_name,
        )
        .await
    }

    /// [05] Test-only: 既に `Failed` state に遷移した snapshot に対して
    /// `execute_outcome(StepOutcome::Persist(snapshot))` を実行する production 経路の
    /// ショートカット。pre-commit append 失敗時に ExecutionStore / state が persist されない
    /// ことを検証するために用いる（spec [05] commit 境界の継承）。
    #[cfg(test)]
    async fn execute_outcome_persist_failed_for_test<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        worktree_path: &str,
        snapshot: WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        // テスト helper の snapshot_before は engine.executions の現在状態を採用する。
        // production 経路では call site が mutation 前に capture するが、本 helper は
        // 既に mutated snapshot を直接渡すための短絡として、現在状態を rollback target
        // 扱いにする（pre-commit 失敗時の挙動を観測する用途のため）。
        let snapshot_before = {
            let execs = self.executions.lock().await;
            execs.get(&snapshot.execution_id).cloned().ok_or_else(|| {
                WorkflowEngineError::ExecutionNotFound(snapshot.execution_id.clone())
            })?
        };
        self.execute_outcome(
            app,
            session_store,
            agent_runtime,
            worktree_path,
            StepOutcome::Persist(snapshot),
            snapshot_before,
        )
        .await
    }

    #[cfg(test)]
    async fn handle_approval_with_output_for_run_for_test(
        &self,
        execution_id: &str,
        expected_execution_id: Option<&str>,
        expected_node_name: Option<&str>,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(execution_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
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
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
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
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(execution_id.to_string()))?;
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
        snapshot: &WorkflowState,
    ) -> Result<Option<StepOutcome>, WorkflowEngineError> {
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
    ) -> WorkflowState {
        let workflow = Workflow {
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
        let exec = WorkflowExecution {
            id: "exec-approval-chat".to_string(),
            workflow,
            state,
            current_node_index: 0,
            node_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            node_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(current_session_id.to_string()),
            current_step_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: vec![NodeExecution {
                id: "node-exec-approval".to_string(),
                execution_id: "exec-approval-chat".to_string(),
                node_name: "implementation_fix_policy".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: node_status,
                session_id: Some(current_session_id.to_string()),
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 1000.0,
                completed_at: None,
            }],
            request: None,
            parallel_run: None,
            current_stall_observations: Vec::new(),
            worktree_path: worktree_path.to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            error_reason: None,
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let snapshot = exec.to_workflow_state();
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
