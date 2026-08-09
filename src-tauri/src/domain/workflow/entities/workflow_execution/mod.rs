//! Workflow execution lifecycle aggregate.
//!
//! This aggregate is the single authority for execution lifecycle admission and
//! transitions. Runtime orchestration and event replay both call these methods;
//! neither is allowed to assign lifecycle state directly.

use std::collections::HashMap;

use crate::domain::workflow::services::routing::LoopGuardResetBaselines;
use crate::domain::workflow::services::{
    history as workflow_history, routing as workflow_routing, submission as workflow_submission,
    transition as workflow_transition,
};
use crate::domain::workflow::value_objects::{
    ExecutionInterruptionReason, ExecutionOrigin, FanoutParentRef, NodeExecutionFailureKind,
    NodeHistoryEntry, NodeKindName, RuntimeArtifact, RuntimeExecutionState, TokenUsage,
    WorkflowDefinition,
};
use crate::domain::workflow::FailureDisposition;

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutRuntimeState {
    pub parent_node_name: String,
    pub parent_node_execution_id: String,
    pub children: Vec<FanoutChildRuntime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildRuntime {
    pub node_execution_id: String,
    pub node_name: String,
    pub session_id: String,
    pub state: FanoutChildRuntimeState,
    pub result: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub contract: Option<String>,
    pub failure_kind: Option<NodeExecutionFailureKind>,
    pub failure_disposition: Option<FailureDisposition>,
    pub token_usage: TokenUsage,
    pub attempt: u32,
    pub completed_at: Option<f64>,
}

impl FanoutChildRuntime {
    pub fn complete(
        &mut self,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
        token_usage: TokenUsage,
        completed_at: f64,
    ) -> TransitionOutcome {
        if self.state == FanoutChildRuntimeState::Completed {
            return TransitionOutcome::AlreadyApplied;
        }
        if self.state != FanoutChildRuntimeState::Running {
            return TransitionOutcome::NotApplicable;
        }
        self.state = FanoutChildRuntimeState::Completed;
        self.result = result;
        self.artifact = artifact;
        self.contract = contract;
        self.failure_kind = None;
        self.failure_disposition = None;
        self.token_usage = token_usage;
        self.completed_at = Some(completed_at);
        TransitionOutcome::Applied
    }

    pub fn fail(
        &mut self,
        kind: NodeExecutionFailureKind,
        disposition: FailureDisposition,
        completed_at: f64,
    ) -> TransitionOutcome {
        if self.state == FanoutChildRuntimeState::Failed
            && self.failure_kind == Some(kind)
            && self.failure_disposition == Some(disposition)
        {
            return TransitionOutcome::AlreadyApplied;
        }
        if self.state != FanoutChildRuntimeState::Running {
            return TransitionOutcome::NotApplicable;
        }
        self.state = FanoutChildRuntimeState::Failed;
        self.failure_kind = Some(kind);
        self.failure_disposition = Some(disposition);
        self.completed_at = Some(completed_at);
        TransitionOutcome::Applied
    }

    pub fn interrupt(&mut self, completed_at: f64) -> TransitionOutcome {
        if self.state == FanoutChildRuntimeState::Interrupted {
            return TransitionOutcome::AlreadyApplied;
        }
        if self.state != FanoutChildRuntimeState::Running {
            return TransitionOutcome::NotApplicable;
        }
        self.state = FanoutChildRuntimeState::Interrupted;
        self.completed_at = Some(completed_at);
        TransitionOutcome::Applied
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanoutChildRuntimeState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl FanoutChildRuntimeState {
    pub fn is_completed(self) -> bool {
        self == Self::Completed
    }
}

/// Workflow-level defaults captured when an execution starts.
///
/// These values affect future node activation and therefore belong to the
/// execution aggregate rather than to an agent-session gateway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowDefaults {
    pub backend_id: Option<String>,
    pub permission_mode: String,
}

/// A non-terminal stall observation retained by the execution aggregate.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeStallObservation {
    pub session_id: String,
    pub node_name: String,
    pub attempt: u32,
    pub turn_phase: String,
    pub idle_secs: u64,
    pub signal_count: u32,
    pub cap_reached: bool,
    pub observed_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeNodeExecutionStatus {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

impl RuntimeNodeExecutionStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeNodeExecutionFailure {
    pub reason: String,
    pub kind: NodeExecutionFailureKind,
}

/// One node attempt held inside the execution aggregate.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeNodeExecution {
    pub id: String,
    pub execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    pub attempt: u32,
    pub status: RuntimeNodeExecutionStatus,
    pub session_id: Option<String>,
    pub display_command: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<RuntimeNodeExecutionFailure>,
    pub fanout_parent: Option<FanoutParentRef>,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

impl RuntimeNodeExecution {
    #[cfg(test)]
    pub fn replay_started(&mut self) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Running {
            return TransitionOutcome::AlreadyApplied;
        }
        self.status = RuntimeNodeExecutionStatus::Running;
        self.completed_at = None;
        TransitionOutcome::Applied
    }

    pub fn attach_session(&mut self, session_id: String) -> TransitionOutcome {
        if self.session_id.as_deref() == Some(session_id.as_str()) {
            return TransitionOutcome::AlreadyApplied;
        }
        if !self.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        self.session_id = Some(session_id);
        TransitionOutcome::Applied
    }

    pub fn prepare_command(&mut self, display_command: String) -> TransitionOutcome {
        if self.display_command.as_deref() == Some(display_command.as_str()) {
            return TransitionOutcome::AlreadyApplied;
        }
        if !self.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        self.display_command = Some(display_command);
        TransitionOutcome::Applied
    }

    pub fn wait_for_approval(&mut self) -> TransitionOutcome {
        self.transition_status(RuntimeNodeExecutionStatus::WaitingApproval, None)
    }

    pub fn resume_after_approval(&mut self) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Running {
            return TransitionOutcome::AlreadyApplied;
        }
        if self.status != RuntimeNodeExecutionStatus::WaitingApproval {
            return TransitionOutcome::NotApplicable;
        }
        self.status = RuntimeNodeExecutionStatus::Running;
        TransitionOutcome::Applied
    }

    pub fn complete(
        &mut self,
        artifact: Option<serde_json::Value>,
        token_usage: Option<TokenUsage>,
        completed_at: f64,
    ) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Succeeded {
            return TransitionOutcome::AlreadyApplied;
        }
        if !self.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        self.status = RuntimeNodeExecutionStatus::Succeeded;
        self.artifact = artifact;
        self.token_usage = token_usage;
        self.failure = None;
        self.completed_at = Some(completed_at);
        TransitionOutcome::Applied
    }

    pub fn record_completed(
        &mut self,
        artifact: Option<serde_json::Value>,
        token_usage: Option<TokenUsage>,
        completed_at: f64,
    ) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Succeeded {
            return TransitionOutcome::AlreadyApplied;
        }
        self.status = RuntimeNodeExecutionStatus::Succeeded;
        self.artifact = artifact;
        self.token_usage = token_usage;
        self.failure = None;
        self.completed_at = Some(completed_at);
        TransitionOutcome::Applied
    }

    pub fn fail(
        &mut self,
        reason: String,
        kind: NodeExecutionFailureKind,
        completed_at: f64,
    ) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Failed
            && self
                .failure
                .as_ref()
                .is_some_and(|failure| failure.reason == reason && failure.kind == kind)
        {
            return TransitionOutcome::AlreadyApplied;
        }
        if !self.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        self.record_failed(reason, kind, completed_at)
    }

    pub fn record_failed(
        &mut self,
        reason: String,
        kind: NodeExecutionFailureKind,
        completed_at: f64,
    ) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Failed
            && self
                .failure
                .as_ref()
                .is_some_and(|failure| failure.reason == reason && failure.kind == kind)
        {
            return TransitionOutcome::AlreadyApplied;
        }
        self.status = RuntimeNodeExecutionStatus::Failed;
        self.failure = Some(RuntimeNodeExecutionFailure { reason, kind });
        self.completed_at = Some(completed_at);
        TransitionOutcome::Applied
    }

    pub fn abort(&mut self, completed_at: f64) -> TransitionOutcome {
        self.transition_status(RuntimeNodeExecutionStatus::Aborted, Some(completed_at))
    }

    fn transition_status(
        &mut self,
        status: RuntimeNodeExecutionStatus,
        completed_at: Option<f64>,
    ) -> TransitionOutcome {
        if self.status == status {
            return TransitionOutcome::AlreadyApplied;
        }
        if !self.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        self.status = status;
        self.completed_at = completed_at;
        TransitionOutcome::Applied
    }
}

/// Function-scoped restore input for the aggregate.
///
/// Gateways may construct this DTO from durable events or snapshots, but must
/// not retain it as mutable execution state.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionRestore {
    pub id: String,
    pub workflow: WorkflowDefinition,
    pub lifecycle: WorkflowExecutionLifecycleRestore,
    pub current_node_index: usize,
    pub node_execution_counts: HashMap<String, u32>,
    pub loop_guard_reset_baselines: LoopGuardResetBaselines,
    pub node_history: Vec<NodeHistoryEntry>,
    pub workflow_defaults: WorkflowDefaults,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub error_reason: Option<String>,
    pub started_at: f64,
    pub updated_at: f64,
    pub current_session_id: Option<String>,
    pub current_node_token_usage: TokenUsage,
    pub artifacts: HashMap<String, RuntimeArtifact>,
    pub node_executions: Vec<RuntimeNodeExecution>,
    pub request: Option<String>,
    pub fanout_runtime: Option<FanoutRuntimeState>,
    pub current_stall_observations: Vec<NodeStallObservation>,
}

#[derive(Clone, Debug)]
pub struct OutputSubmissionRollback {
    node_name: String,
    node_execution_id: String,
    prior_artifact_entry: Option<RuntimeArtifact>,
    prior_node_execution_artifact: Option<serde_json::Value>,
    prior_fanout_child_output: Option<(Option<String>, Option<serde_json::Value>)>,
    fanout_child: bool,
    prior_updated_at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionLifecycleRestore {
    state: RuntimeExecutionState,
    interruption_reason: Option<ExecutionInterruptionReason>,
}

impl Default for WorkflowExecutionRestore {
    fn default() -> Self {
        Self {
            id: String::new(),
            workflow: WorkflowDefinition::default(),
            lifecycle: WorkflowExecutionLifecycleRestore {
                state: RuntimeExecutionState::Running,
                interruption_reason: None,
            },
            current_node_index: 0,
            node_execution_counts: HashMap::new(),
            loop_guard_reset_baselines: LoopGuardResetBaselines::default(),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults::default(),
            worktree_path: String::new(),
            created_from: ExecutionOrigin::DesktopUi,
            error_reason: None,
            started_at: 0.0,
            updated_at: 0.0,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        }
    }
}

/// The three lifecycle state sets used by the workflow lifecycle specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStateSet {
    Active,
    Resumable,
    Finished,
}

/// A defined outcome for every operation/state matrix cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionOutcome {
    Applied,
    AlreadyApplied,
    NotApplicable,
    Rejected(TransitionRejection),
}

/// Stable rejection reasons returned by aggregate admission methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionRejection {
    AlreadyStopped,
    NotActive,
    NotResumable,
    NotWaitingApproval,
    ArtifactNotAccepted,
    WorkflowTurnNotAuthorized,
}

/// Canonical fact derived from an observed turn result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalNodeFact {
    Completed,
    Failed {
        reason: String,
        kind: NodeExecutionFailureKind,
    },
}

/// How a canonical turn-completion fact is applied for the current state set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCompletionApplication {
    Live,
    RecordOnly,
    Superseded,
}

/// One closed decision: a turn observation always carries its canonical fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCompletionDecision {
    pub application: TurnCompletionApplication,
    pub fact: CanonicalNodeFact,
}

/// Replay outcomes distinguish valid application, idempotency, and contradiction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayOutcome {
    Applied,
    AlreadyApplied,
    NotApplicable,
    Rejected(TransitionRejection),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NextNodeDecision {
    Completed,
    TransitionTo(String),
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionAdvanceDecision {
    Persist,
    StartFanout,
    TransitionAndStart,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub enum LoopGuardResult {
    Allowed,
    Exceeded {
        max_iterations: u32,
        count: u32,
        on_exhausted: Option<String>,
    },
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub enum TurnCompleteAction {
    SessionError {
        node_name: String,
        exit_code: i64,
        kind: NodeExecutionFailureKind,
    },
    AutoEvaluate {
        node_name: String,
    },
    WaitApproval,
    UnexpectedNodeKind {
        node_name: String,
        kind: NodeKindName,
    },
    NotRunning,
}

/// Aggregate that owns the workflow execution lifecycle state.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecution {
    state: RuntimeExecutionState,
    interruption_reason: Option<ExecutionInterruptionReason>,
    runtime: WorkflowExecutionView,
}

/// Read-only runtime view exposed by the aggregate.
///
/// `WorkflowExecution` implements `Deref` but deliberately not `DerefMut`.
/// Adapters can inspect this projection while every mutation remains an
/// aggregate method.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecutionView {
    pub id: String,
    pub workflow: WorkflowDefinition,
    pub current_node_index: usize,
    pub node_execution_counts: HashMap<String, u32>,
    pub loop_guard_reset_baselines: LoopGuardResetBaselines,
    pub node_history: Vec<NodeHistoryEntry>,
    pub workflow_defaults: WorkflowDefaults,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub error_reason: Option<String>,
    pub started_at: f64,
    pub updated_at: f64,
    pub current_session_id: Option<String>,
    pub current_node_token_usage: TokenUsage,
    pub artifacts: HashMap<String, RuntimeArtifact>,
    pub node_executions: Vec<RuntimeNodeExecution>,
    pub request: Option<String>,
    pub fanout_runtime: Option<FanoutRuntimeState>,
    pub current_stall_observations: Vec<NodeStallObservation>,
}

impl std::ops::Deref for WorkflowExecution {
    type Target = WorkflowExecutionView;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

#[cfg(test)]
impl std::ops::DerefMut for WorkflowExecution {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
}

impl WorkflowExecution {
    /// Restores a lifecycle snapshot before replaying subsequent durable facts.
    pub fn restore(
        state: RuntimeExecutionState,
        interruption_reason: Option<ExecutionInterruptionReason>,
    ) -> Self {
        Self::restore_runtime(WorkflowExecutionRestore {
            lifecycle: WorkflowExecutionLifecycleRestore {
                state,
                interruption_reason,
            },
            ..WorkflowExecutionRestore::default()
        })
    }

    pub fn lifecycle_from_state(state: RuntimeExecutionState) -> WorkflowExecutionLifecycleRestore {
        let interruption_reason = matches!(state, RuntimeExecutionState::Interrupted)
            .then_some(ExecutionInterruptionReason::Crash);
        WorkflowExecutionLifecycleRestore {
            state,
            interruption_reason,
        }
    }

    /// Restores the entire execution aggregate from a durable projection.
    pub fn restore_runtime(restore: WorkflowExecutionRestore) -> Self {
        Self {
            state: restore.lifecycle.state,
            interruption_reason: restore.lifecycle.interruption_reason,
            runtime: WorkflowExecutionView {
                id: restore.id,
                workflow: restore.workflow,
                current_node_index: restore.current_node_index,
                node_execution_counts: restore.node_execution_counts,
                loop_guard_reset_baselines: restore.loop_guard_reset_baselines,
                node_history: restore.node_history,
                workflow_defaults: restore.workflow_defaults,
                worktree_path: restore.worktree_path,
                created_from: restore.created_from,
                error_reason: restore.error_reason,
                started_at: restore.started_at,
                updated_at: restore.updated_at,
                current_session_id: restore.current_session_id,
                current_node_token_usage: restore.current_node_token_usage,
                artifacts: restore.artifacts,
                node_executions: restore.node_executions,
                request: restore.request,
                fanout_runtime: restore.fanout_runtime,
                current_stall_observations: restore.current_stall_observations,
            },
        }
    }

    pub fn state(&self) -> &RuntimeExecutionState {
        &self.state
    }

    #[cfg(test)]
    pub fn node_executions(&self) -> &[RuntimeNodeExecution] {
        &self.runtime.node_executions
    }

    #[cfg(test)]
    pub fn fanout_runtime(&self) -> Option<&FanoutRuntimeState> {
        self.runtime.fanout_runtime.as_ref()
    }

    pub fn transition_completed(&mut self) -> TransitionOutcome {
        self.complete()
    }

    pub fn transition_failed(
        &mut self,
        reason: String,
        kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    ) -> TransitionOutcome {
        self.fail(reason, kind, retry_count)
    }

    pub fn transition_running(&mut self) -> TransitionOutcome {
        match self.state {
            RuntimeExecutionState::Interrupted => self.resume(),
            RuntimeExecutionState::WaitingApproval => self.resolve_approval(),
            RuntimeExecutionState::Running => TransitionOutcome::AlreadyApplied,
            RuntimeExecutionState::Completed
            | RuntimeExecutionState::Failed { .. }
            | RuntimeExecutionState::Aborted => TransitionOutcome::NotApplicable,
        }
    }

    pub fn transition_waiting_approval(&mut self) -> TransitionOutcome {
        self.request_approval()
    }

    pub fn transition_interrupted(
        &mut self,
        reason: ExecutionInterruptionReason,
    ) -> TransitionOutcome {
        if reason == ExecutionInterruptionReason::Stop {
            self.stop()
        } else {
            self.interrupt(reason)
        }
    }

    pub fn transition_aborted(&mut self) -> TransitionOutcome {
        self.abort()
    }

    pub fn start_node_execution(
        &mut self,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        node_execution_id: Option<String>,
        timestamp: f64,
    ) -> String {
        self.begin_node_attempt(
            node_name,
            kind,
            attempt,
            fanout_parent,
            node_execution_id,
            timestamp,
        )
        .expect("node execution start must be admitted before applying its decision")
    }

    pub fn start_current_node_execution(&mut self, timestamp: f64) -> String {
        let node_name = self.runtime.workflow.nodes[self.runtime.current_node_index]
            .name
            .clone();
        let kind = self.runtime.workflow.nodes[self.runtime.current_node_index].kind_name();
        let attempt = self
            .runtime
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        self.start_node_execution(node_name, kind, attempt, None, None, timestamp)
    }

    pub fn active_current_node_execution_id(&self) -> Option<&str> {
        let node = self
            .runtime
            .workflow
            .nodes
            .get(self.runtime.current_node_index)?;
        let attempt = self
            .runtime
            .node_execution_counts
            .get(&node.name)
            .copied()
            .unwrap_or(1);
        self.runtime
            .node_executions
            .iter()
            .rev()
            .find(|execution| {
                execution.node_name == node.name
                    && execution.attempt == attempt
                    && execution.fanout_parent.is_none()
                    && execution.status.is_active()
            })
            .map(|execution| execution.id.as_str())
    }

    pub fn make_aborted_history_entry(&mut self, timestamp: f64) -> NodeHistoryEntry {
        let node_name = self.runtime.workflow.nodes[self.runtime.current_node_index]
            .name
            .clone();
        let attempt = self
            .runtime
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        let token_usage = std::mem::take(&mut self.runtime.current_node_token_usage);
        workflow_history::aborted_node_history_entry(
            node_name,
            attempt,
            self.runtime.current_session_id.clone(),
            token_usage,
            timestamp,
        )
    }

    pub fn make_aborted_fanout_history_entry(&self, timestamp: f64) -> Option<NodeHistoryEntry> {
        let fanout = self.runtime.fanout_runtime.as_ref()?;
        let parent_attempt = self
            .runtime
            .node_execution_counts
            .get(&fanout.parent_node_name)
            .copied()
            .unwrap_or(1);
        Some(workflow_history::aborted_fanout_history_entry(
            fanout,
            &self.runtime.artifacts,
            parent_attempt,
            timestamp,
        ))
    }

    pub fn make_node_history_entry_at(
        &mut self,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
        completed_at: f64,
    ) -> NodeHistoryEntry {
        let node_name = self.runtime.workflow.nodes[self.runtime.current_node_index]
            .name
            .clone();
        let attempt = self
            .runtime
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        let token_usage = std::mem::take(&mut self.runtime.current_node_token_usage);
        let entry = workflow_history::completed_node_history_entry(
            workflow_history::CompletedNodeHistoryInput {
                node_name: node_name.clone(),
                completed_at,
                result,
                session_id: self.runtime.current_session_id.clone(),
                token_usage: Some(token_usage),
                artifact,
                attempt,
            },
        );
        if let Some(output) =
            workflow_history::artifact_from_completed_history_entry(&entry, contract)
        {
            self.runtime.artifacts.insert(node_name, output);
        }
        self.runtime.current_session_id = None;
        self.runtime.current_stall_observations.clear();
        entry
    }

    pub fn make_failed_node_history_entry_at(
        &mut self,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
        timestamp: f64,
    ) -> NodeHistoryEntry {
        let mut entry = self.make_node_history_entry_at(result, artifact, contract, timestamp);
        entry.state = crate::domain::workflow::NODE_STATUS_FAILED.to_string();
        entry
    }

    pub fn clear_artifacts_for_new_execution(&mut self, node_index: usize) {
        for key in workflow_submission::artifact_keys_to_clear_for_new_node_execution(
            &self.runtime.workflow,
            node_index,
        ) {
            self.runtime.artifacts.remove(&key);
        }
    }

    pub fn decide_next_node(&self) -> NextNodeDecision {
        match workflow_routing::route_with_reset_baselines(
            &self.runtime.workflow,
            self.runtime.current_node_index,
            self.current_node_artifact(),
            &self.runtime.node_execution_counts,
            &self.runtime.loop_guard_reset_baselines,
        ) {
            Ok(workflow_routing::RouteDecision::Completed) => NextNodeDecision::Completed,
            Ok(workflow_routing::RouteDecision::TransitionTo(name)) => {
                NextNodeDecision::TransitionTo(name)
            }
            Err(err) => NextNodeDecision::Failed {
                reason: err.to_string(),
            },
        }
    }

    pub fn apply_advance_at(&mut self, timestamp: f64) -> ExecutionAdvanceDecision {
        let completed_node_name = self.runtime.workflow.nodes[self.runtime.current_node_index]
            .name
            .clone();
        self.runtime
            .loop_guard_reset_baselines
            .record_successful_completion(
                &self.runtime.workflow,
                &completed_node_name,
                &self.runtime.node_execution_counts,
            );
        match self.decide_next_node() {
            NextNodeDecision::Completed => {
                let _ = self.transition_completed();
                self.runtime.updated_at = timestamp;
                ExecutionAdvanceDecision::Persist
            }
            NextNodeDecision::Failed { reason } => {
                let _ = self.transition_failed(
                    reason,
                    NodeExecutionFailureKind::ValidationFailure,
                    None,
                );
                self.runtime.updated_at = timestamp;
                ExecutionAdvanceDecision::Persist
            }
            NextNodeDecision::TransitionTo(name) => {
                let index = self
                    .runtime
                    .workflow
                    .nodes
                    .iter()
                    .position(|node| node.name == name)
                    .expect("routing decision must reference a known node");
                self.apply_transition_index(index, &name, timestamp);
                if self.runtime.workflow.nodes[index].is_fanout() {
                    ExecutionAdvanceDecision::StartFanout
                } else {
                    ExecutionAdvanceDecision::TransitionAndStart
                }
            }
        }
    }

    pub fn retry_current_node_at(&mut self, timestamp: f64) {
        let node_index = self.runtime.current_node_index;
        let node_name = self.runtime.workflow.nodes[node_index].name.clone();
        let _ = self.transition_running();
        *self
            .runtime
            .node_execution_counts
            .entry(node_name)
            .or_insert(0) += 1;
        self.runtime.current_session_id = None;
        self.runtime.current_node_token_usage = TokenUsage::default();
        self.runtime.current_stall_observations.clear();
        self.clear_artifacts_for_new_execution(node_index);
        self.runtime.updated_at = timestamp;
        self.start_current_node_execution(timestamp);
    }

    #[cfg(test)]
    pub fn check_loop_guard(
        &self,
        target_node_name: &str,
    ) -> Result<LoopGuardResult, crate::domain::workflow::WorkflowError> {
        let decision = workflow_routing::guarded_target_with_reset_baselines(
            &self.runtime.workflow,
            target_node_name.to_string(),
            &self.runtime.node_execution_counts,
            &self.runtime.loop_guard_reset_baselines,
        )?;
        if matches!(
            decision,
            workflow_routing::RouteDecision::TransitionTo(ref name) if name == target_node_name
        ) {
            return Ok(LoopGuardResult::Allowed);
        }
        let node = self
            .runtime
            .workflow
            .nodes
            .iter()
            .find(|node| node.name == target_node_name)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::validation(format!(
                    "Node '{target_node_name}' not found in workflow"
                ))
            })?;
        let Some((max_iterations, on_exhausted, reset_on)) = workflow_routing::loop_guard(node)
        else {
            return Ok(LoopGuardResult::Allowed);
        };
        let cumulative_count = self
            .runtime
            .node_execution_counts
            .get(target_node_name)
            .copied()
            .unwrap_or(0);
        let count = self.runtime.loop_guard_reset_baselines.execution_count(
            target_node_name,
            cumulative_count,
            reset_on,
        );
        Ok(LoopGuardResult::Exceeded {
            max_iterations,
            count,
            on_exhausted: Some(on_exhausted.to_string()),
        })
    }

    #[cfg(test)]
    pub fn decide_turn_complete_action(&self, exit_code: i64) -> TurnCompleteAction {
        let action = workflow_transition::decide_turn_complete_action(
            &self.runtime.workflow,
            self.runtime.current_node_index,
            &self.state,
            exit_code,
        )
        .expect("current node index must reference workflow node");
        match action {
            workflow_transition::TurnCompleteDecision::NotRunning => TurnCompleteAction::NotRunning,
            workflow_transition::TurnCompleteDecision::SessionError {
                node_name,
                exit_code,
                kind,
            } => TurnCompleteAction::SessionError {
                node_name,
                exit_code,
                kind,
            },
            workflow_transition::TurnCompleteDecision::AutoEvaluate { node_name } => {
                TurnCompleteAction::AutoEvaluate { node_name }
            }
            workflow_transition::TurnCompleteDecision::WaitApproval => {
                TurnCompleteAction::WaitApproval
            }
            workflow_transition::TurnCompleteDecision::UnexpectedNodeKind { node_name, kind } => {
                TurnCompleteAction::UnexpectedNodeKind { node_name, kind }
            }
        }
    }

    #[cfg(test)]
    pub fn decide_approve_action(&self) -> Result<(), crate::domain::workflow::WorkflowError> {
        workflow_transition::decide_approve_action(
            &self.runtime.workflow,
            self.runtime.current_node_index,
            &self.state,
        )
    }

    pub fn plan_turn_complete_mutation(
        &self,
        exit_code: i64,
        failure_signal: Option<workflow_transition::SessionFailureSignal>,
    ) -> Result<workflow_transition::TurnCompleteMutationPlan, crate::domain::workflow::WorkflowError>
    {
        workflow_transition::plan_turn_complete_mutation_with_signal(
            &self.runtime.workflow,
            self.runtime.current_node_index,
            &self.state,
            exit_code,
            failure_signal,
        )
    }

    pub fn plan_approval_application(
        &self,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<workflow_transition::ApprovalApplicationPlan, crate::domain::workflow::WorkflowError>
    {
        workflow_transition::plan_approval_application(
            &self.runtime.workflow,
            self.runtime.current_node_index,
            &self.state,
            application,
        )
    }

    fn current_node_artifact(&self) -> Option<&serde_json::Value> {
        let node_name = &self
            .runtime
            .workflow
            .nodes
            .get(self.runtime.current_node_index)?
            .name;
        self.runtime
            .artifacts
            .get(node_name)
            .and_then(|output| output.artifact.as_ref())
    }

    fn apply_transition_index(&mut self, node_index: usize, node_name: &str, timestamp: f64) {
        self.runtime.current_node_index = node_index;
        let _ = self.transition_running();
        *self
            .runtime
            .node_execution_counts
            .entry(node_name.to_string())
            .or_insert(0) += 1;
        self.runtime.current_session_id = None;
        self.runtime.current_stall_observations.clear();
        self.clear_artifacts_for_new_execution(node_index);
        self.runtime.updated_at = timestamp;
        self.start_current_node_execution(timestamp);
    }

    pub fn set_current_node(&mut self, index: usize, timestamp: f64) -> TransitionOutcome {
        if !self.is_active() || index >= self.runtime.workflow.nodes.len() {
            return TransitionOutcome::Rejected(TransitionRejection::NotActive);
        }
        if self.runtime.current_node_index == index {
            return TransitionOutcome::AlreadyApplied;
        }
        self.runtime.current_node_index = index;
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    pub fn begin_node_attempt(
        &mut self,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        node_execution_id: Option<String>,
        timestamp: f64,
    ) -> Result<String, TransitionRejection> {
        if !self.is_active() {
            return Err(TransitionRejection::NotActive);
        }
        let id = node_execution_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if self
            .node_executions
            .iter()
            .any(|execution| execution.id == id)
        {
            return Ok(id);
        }
        self.runtime
            .node_execution_counts
            .entry(node_name.clone())
            .and_modify(|current| *current = (*current).max(attempt))
            .or_insert(attempt);
        self.runtime.node_executions.push(RuntimeNodeExecution {
            id: id.clone(),
            execution_id: self.runtime.id.clone(),
            node_name,
            kind,
            attempt,
            status: RuntimeNodeExecutionStatus::Running,
            session_id: None,
            display_command: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent,
            started_at: timestamp,
            completed_at: None,
        });
        self.runtime.updated_at = timestamp;
        Ok(id)
    }

    pub fn attach_node_session(
        &mut self,
        node_execution_id: &str,
        session_id: String,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        if execution.session_id.as_deref() == Some(session_id.as_str())
            && self.runtime.current_session_id.as_deref() == Some(session_id.as_str())
        {
            return TransitionOutcome::AlreadyApplied;
        }
        execution.session_id = Some(session_id.clone());
        self.runtime.current_session_id = Some(session_id);
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    pub fn record_node_display_command(
        &mut self,
        node_execution_id: &str,
        display_command: String,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let outcome = execution.prepare_command(display_command);
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn attach_child_node_session(
        &mut self,
        node_execution_id: &str,
        session_id: String,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let outcome = execution.attach_session(session_id);
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn record_node_token_usage(
        &mut self,
        node_execution_id: &str,
        token_usage: TokenUsage,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        execution.token_usage = Some(token_usage);
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    pub fn increase_node_attempt_count_to(
        &mut self,
        node_name: String,
        attempt: u32,
        timestamp: f64,
    ) {
        self.runtime
            .node_execution_counts
            .entry(node_name)
            .and_modify(|current| *current = (*current).max(attempt))
            .or_insert(attempt);
        self.runtime.updated_at = timestamp;
    }

    pub fn replay_artifact_produced(
        &mut self,
        node_execution_id: &str,
        node_name: &str,
        contract: Option<String>,
        value: serde_json::Value,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        if execution.node_name != node_name {
            return TransitionOutcome::Rejected(TransitionRejection::ArtifactNotAccepted);
        }
        if execution.artifact.as_ref() == Some(&value) {
            return TransitionOutcome::AlreadyApplied;
        }
        execution.artifact = Some(value.clone());
        if execution.fanout_parent.is_some() {
            let Some(child) = self.runtime.fanout_runtime.as_mut().and_then(|fanout| {
                fanout
                    .children
                    .iter_mut()
                    .find(|child| child.node_execution_id == node_execution_id)
            }) else {
                return TransitionOutcome::NotApplicable;
            };
            child.artifact = Some(value);
            child.contract = contract;
        } else {
            self.runtime.artifacts.insert(
                node_name.to_string(),
                RuntimeArtifact {
                    node_name: node_name.to_string(),
                    attempt: execution.attempt,
                    session_id: execution.session_id.clone(),
                    result: None,
                    artifact: Some(value),
                    contract,
                    token_usage: None,
                    completed_at: timestamp,
                },
            );
        }
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_submitted_output(
        &mut self,
        node_name: String,
        node_execution_id: &str,
        attempt: u32,
        session_id: Option<String>,
        contract: String,
        output: serde_json::Value,
        result: Option<String>,
        timestamp: f64,
    ) -> Option<OutputSubmissionRollback> {
        let execution_index = self
            .runtime
            .node_executions
            .iter()
            .position(|execution| execution.id == node_execution_id)?;
        let fanout_child = self.runtime.node_executions[execution_index]
            .fanout_parent
            .is_some();
        let fanout_child_index = if fanout_child {
            Some(
                self.runtime
                    .fanout_runtime
                    .as_ref()?
                    .children
                    .iter()
                    .position(|child| child.node_execution_id == node_execution_id)?,
            )
        } else {
            None
        };
        let prior_updated_at = self.runtime.updated_at;
        let prior_node_execution_artifact = self.runtime.node_executions[execution_index]
            .artifact
            .replace(output.clone());
        let prior_artifact_entry = (!fanout_child)
            .then(|| self.runtime.artifacts.get(&node_name).cloned())
            .flatten();
        let prior_fanout_child_output = if fanout_child {
            let child = &mut self.runtime.fanout_runtime.as_mut()?.children[fanout_child_index?];
            let prior = (child.result.clone(), child.artifact.clone());
            child.result = result;
            child.artifact = Some(output.clone());
            Some(prior)
        } else {
            self.runtime.artifacts.insert(
                node_name.clone(),
                RuntimeArtifact {
                    node_name: node_name.clone(),
                    attempt,
                    session_id,
                    result,
                    artifact: Some(output),
                    contract: Some(contract),
                    token_usage: None,
                    completed_at: timestamp,
                },
            );
            None
        };
        self.runtime.updated_at = timestamp;
        Some(OutputSubmissionRollback {
            node_name,
            node_execution_id: node_execution_id.to_string(),
            prior_artifact_entry,
            prior_node_execution_artifact,
            prior_fanout_child_output,
            fanout_child,
            prior_updated_at,
        })
    }

    pub fn rollback_submitted_output(
        &mut self,
        rollback: OutputSubmissionRollback,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == rollback.node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        execution.artifact = rollback.prior_node_execution_artifact;
        if rollback.fanout_child {
            let Some((result, artifact)) = rollback.prior_fanout_child_output else {
                return TransitionOutcome::NotApplicable;
            };
            let Some(child) = self.runtime.fanout_runtime.as_mut().and_then(|fanout| {
                fanout
                    .children
                    .iter_mut()
                    .find(|child| child.node_execution_id == rollback.node_execution_id)
            }) else {
                return TransitionOutcome::NotApplicable;
            };
            child.result = result;
            child.artifact = artifact;
        } else {
            match rollback.prior_artifact_entry {
                Some(prior) => {
                    self.runtime.artifacts.insert(rollback.node_name, prior);
                }
                None => {
                    self.runtime.artifacts.remove(&rollback.node_name);
                }
            }
        }
        self.runtime.updated_at = rollback.prior_updated_at;
        TransitionOutcome::Applied
    }

    pub fn record_successful_node_completion(&mut self, child_node_name: &str, timestamp: f64) {
        self.runtime
            .loop_guard_reset_baselines
            .record_successful_completion(
                &self.runtime.workflow,
                child_node_name,
                &self.runtime.node_execution_counts,
            );
        self.runtime.updated_at = timestamp;
    }

    pub fn mark_node_waiting_approval(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let outcome = execution.wait_for_approval();
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn mark_node_running(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let outcome = execution.resume_after_approval();
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn abort_node_execution(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> TransitionOutcome {
        self.set_node_status(
            node_execution_id,
            RuntimeNodeExecutionStatus::Aborted,
            timestamp,
        )
    }

    pub fn abort_active_node_executions(&mut self, timestamp: f64) -> Vec<String> {
        let mut aborted = Vec::new();
        for execution in &mut self.runtime.node_executions {
            if execution.status.is_active() {
                let _ = execution.abort(timestamp);
                aborted.push(execution.id.clone());
            }
        }
        if !aborted.is_empty() {
            self.runtime.updated_at = timestamp;
        }
        aborted
    }

    pub fn complete_node_execution(
        &mut self,
        node_execution_id: &str,
        artifact: Option<serde_json::Value>,
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let outcome = execution.complete(artifact, token_usage, timestamp);
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn fail_node_execution(
        &mut self,
        node_execution_id: &str,
        reason: String,
        kind: NodeExecutionFailureKind,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let outcome = execution.fail(reason, kind, timestamp);
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    /// Applies an observed turn and its canonical node fact atomically.
    ///
    /// Interrupted executions accept the fact record-only; finished executions
    /// return Superseded without mutating their already-decided projection.
    pub fn apply_observed_turn(
        &mut self,
        node_execution_id: &str,
        fact: CanonicalNodeFact,
        artifact: Option<serde_json::Value>,
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    ) -> TurnCompletionDecision {
        let decision = self.apply_turn_completion(fact.clone());
        let fanout_fact = fact.clone();
        let fanout_artifact = artifact.clone();
        let fanout_token_usage = token_usage.clone();
        match decision.application {
            TurnCompletionApplication::Live => match fact {
                CanonicalNodeFact::Completed => {
                    self.complete_node_execution(
                        node_execution_id,
                        artifact,
                        token_usage,
                        timestamp,
                    );
                }
                CanonicalNodeFact::Failed { reason, kind } => {
                    self.fail_node_execution(node_execution_id, reason, kind, timestamp);
                }
            },
            TurnCompletionApplication::RecordOnly => {
                if let Some(execution) = self
                    .runtime
                    .node_executions
                    .iter_mut()
                    .find(|execution| execution.id == node_execution_id)
                {
                    match fact {
                        CanonicalNodeFact::Completed => {
                            let _ = execution.record_completed(artifact, token_usage, timestamp);
                        }
                        CanonicalNodeFact::Failed { reason, kind } => {
                            let _ = execution.record_failed(reason, kind, timestamp);
                        }
                    }
                    self.runtime.updated_at = timestamp;
                }
            }
            TurnCompletionApplication::Superseded => {}
        }
        if decision.application != TurnCompletionApplication::Superseded {
            if let Some(child) = self.runtime.fanout_runtime.as_mut().and_then(|fanout| {
                fanout
                    .children
                    .iter_mut()
                    .find(|child| child.node_execution_id == node_execution_id)
            }) {
                match fanout_fact {
                    CanonicalNodeFact::Completed => {
                        child.state = FanoutChildRuntimeState::Completed;
                        child.artifact = fanout_artifact;
                        if let Some(token_usage) = fanout_token_usage {
                            child.token_usage = token_usage;
                        }
                        child.failure_kind = None;
                        child.failure_disposition = None;
                    }
                    CanonicalNodeFact::Failed { kind, .. } => {
                        child.state = FanoutChildRuntimeState::Failed;
                        child.result = Some(kind.as_str().to_string());
                        child.artifact = None;
                        child.failure_kind = Some(kind);
                        child.failure_disposition = Some(FailureDisposition::Terminal);
                    }
                }
                child.completed_at = Some(timestamp);
            }
        }
        decision
    }

    pub fn install_fanout(
        &mut self,
        fanout: FanoutRuntimeState,
        timestamp: f64,
    ) -> TransitionOutcome {
        if !self.is_active() {
            return TransitionOutcome::Rejected(TransitionRejection::NotActive);
        }
        if self.runtime.fanout_runtime.as_ref() == Some(&fanout) {
            return TransitionOutcome::AlreadyApplied;
        }
        self.runtime.fanout_runtime = Some(fanout);
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_fanout_child_execution(
        &mut self,
        parent_node_name: String,
        parent_node_execution_id: String,
        node_execution_id: String,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: FanoutParentRef,
        timestamp: f64,
    ) -> Result<TransitionOutcome, TransitionRejection> {
        let node_outcome = if self
            .runtime
            .node_executions
            .iter()
            .any(|execution| execution.id == node_execution_id)
        {
            TransitionOutcome::AlreadyApplied
        } else {
            self.begin_node_attempt(
                node_name.clone(),
                kind,
                attempt,
                Some(fanout_parent),
                Some(node_execution_id.clone()),
                timestamp,
            )?;
            TransitionOutcome::Applied
        };
        let fanout = self
            .runtime
            .fanout_runtime
            .get_or_insert_with(|| FanoutRuntimeState {
                parent_node_name,
                parent_node_execution_id,
                children: Vec::new(),
            });
        if fanout
            .children
            .iter()
            .any(|child| child.node_execution_id == node_execution_id)
        {
            return Ok(TransitionOutcome::AlreadyApplied);
        }
        fanout.children.push(FanoutChildRuntime {
            node_execution_id,
            node_name,
            session_id: String::new(),
            state: FanoutChildRuntimeState::Running,
            result: None,
            artifact: None,
            contract: None,
            failure_kind: None,
            failure_disposition: None,
            token_usage: TokenUsage::default(),
            attempt,
            completed_at: None,
        });
        self.runtime.updated_at = timestamp;
        Ok(node_outcome)
    }

    pub fn clear_fanout(&mut self, timestamp: f64) -> TransitionOutcome {
        if self.runtime.fanout_runtime.take().is_none() {
            return TransitionOutcome::AlreadyApplied;
        }
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    pub fn finalize_fanout_parent(
        &mut self,
        parent_node_execution_id: &str,
        parent_artifact: RuntimeArtifact,
        history_entry: NodeHistoryEntry,
        timestamp: f64,
    ) -> TransitionOutcome {
        let node_artifact = parent_artifact.artifact.clone();
        let token_usage = parent_artifact.token_usage.clone();
        let outcome = self.complete_node_execution(
            parent_node_execution_id,
            node_artifact,
            token_usage,
            timestamp,
        );
        if matches!(outcome, TransitionOutcome::NotApplicable) {
            return outcome;
        }
        self.runtime
            .artifacts
            .insert(parent_artifact.node_name.clone(), parent_artifact);
        self.runtime.current_node_token_usage = TokenUsage::default();
        self.runtime.current_session_id = None;
        self.runtime.node_history.push(history_entry);
        self.runtime.fanout_runtime = None;
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    pub fn complete_fanout_child_execution(
        &mut self,
        node_execution_id: &str,
        result: Option<String>,
        artifact_value: Option<serde_json::Value>,
        contract: Option<String>,
        token_usage: TokenUsage,
        completed_at: f64,
    ) -> TransitionOutcome {
        if !self.is_active() {
            return TransitionOutcome::Rejected(TransitionRejection::NotActive);
        }
        let Some(fanout) = self.runtime.fanout_runtime.as_mut() else {
            return TransitionOutcome::NotApplicable;
        };
        let Some(child) = fanout
            .children
            .iter_mut()
            .find(|child| child.node_execution_id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let child_outcome = child.complete(
            result,
            artifact_value.clone(),
            contract,
            token_usage.clone(),
            completed_at,
        );
        if child_outcome != TransitionOutcome::Applied {
            return child_outcome;
        }
        let outcome = self.complete_node_execution(
            node_execution_id,
            artifact_value,
            Some(token_usage),
            completed_at,
        );
        if matches!(outcome, TransitionOutcome::Applied) {
            TransitionOutcome::Applied
        } else {
            outcome
        }
    }

    pub fn record_fanout_child_turn_usage(
        &mut self,
        parent_node_name: &str,
        session_id: &str,
        token_usage: Option<TokenUsage>,
    ) -> Option<FanoutChildRuntime> {
        let fanout = self.runtime.fanout_runtime.as_mut()?;
        if fanout.parent_node_name != parent_node_name {
            return None;
        }
        let child = fanout
            .children
            .iter_mut()
            .find(|child| child.session_id == session_id)?;
        if let Some(token_usage) = token_usage {
            child.token_usage.add(&token_usage);
        }
        Some(child.clone())
    }

    pub fn fail_fanout_child_execution(
        &mut self,
        node_execution_id: &str,
        reason: String,
        kind: NodeExecutionFailureKind,
        disposition: FailureDisposition,
        timestamp: f64,
    ) -> TransitionOutcome {
        if !self.is_active() {
            return TransitionOutcome::Rejected(TransitionRejection::NotActive);
        }
        let Some(fanout) = self.runtime.fanout_runtime.as_mut() else {
            return TransitionOutcome::NotApplicable;
        };
        let Some(child) = fanout
            .children
            .iter_mut()
            .find(|child| child.node_execution_id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        let child_outcome = child.fail(kind, disposition, timestamp);
        if child_outcome != TransitionOutcome::Applied {
            return child_outcome;
        }
        child.result = Some(kind.as_str().to_string());
        child.artifact = None;
        self.fail_node_execution(node_execution_id, reason, kind, timestamp)
    }

    pub fn interrupt_running_fanout_children(
        &mut self,
        except_node_execution_id: Option<&str>,
        timestamp: f64,
    ) -> Vec<String> {
        let Some(fanout) = self.runtime.fanout_runtime.as_mut() else {
            return Vec::new();
        };
        let mut interrupted = Vec::new();
        for child in &mut fanout.children {
            if child.state != FanoutChildRuntimeState::Running
                || except_node_execution_id == Some(child.node_execution_id.as_str())
            {
                continue;
            }
            child.state = FanoutChildRuntimeState::Interrupted;
            child.completed_at = Some(timestamp);
            interrupted.push(child.node_execution_id.clone());
        }
        for node_execution_id in &interrupted {
            self.abort_node_execution(node_execution_id, timestamp);
        }
        interrupted
    }

    pub fn record_history_entry(&mut self, entry: NodeHistoryEntry, timestamp: f64) {
        self.runtime.node_history.push(entry);
        self.runtime.updated_at = timestamp;
    }

    pub fn touch(&mut self, timestamp: f64) {
        self.runtime.updated_at = timestamp;
    }

    pub fn record_interruption_metadata(
        &mut self,
        reason: ExecutionInterruptionReason,
        timestamp: f64,
    ) {
        self.runtime.error_reason = Some(reason.as_str().to_string());
        self.runtime.current_session_id = None;
        self.runtime.current_stall_observations.clear();
        self.runtime.updated_at = timestamp;
    }

    pub fn interrupt_runtime(
        &mut self,
        reason: ExecutionInterruptionReason,
        timestamp: f64,
    ) -> TransitionOutcome {
        for execution in self
            .runtime
            .node_executions
            .iter_mut()
            .filter(|execution| execution.status.is_active())
        {
            let _ = execution.abort(timestamp);
        }
        if let Some(fanout) = self.runtime.fanout_runtime.as_mut() {
            for child in fanout
                .children
                .iter_mut()
                .filter(|child| child.state == FanoutChildRuntimeState::Running)
            {
                let _ = child.interrupt(timestamp);
            }
        }
        let outcome = self.transition_interrupted(reason);
        self.record_interruption_metadata(reason, timestamp);
        outcome
    }

    pub fn clear_stalls_for_session(&mut self, session_id: &str, timestamp: f64) -> bool {
        let before = self.runtime.current_stall_observations.len();
        self.runtime
            .current_stall_observations
            .retain(|observation| observation.session_id != session_id);
        let changed = self.runtime.current_stall_observations.len() != before;
        if changed {
            self.runtime.updated_at = timestamp;
        }
        changed
    }

    pub fn upsert_artifact(&mut self, artifact: RuntimeArtifact, timestamp: f64) {
        self.runtime
            .artifacts
            .insert(artifact.node_name.clone(), artifact);
        self.runtime.updated_at = timestamp;
    }

    pub fn add_current_token_usage(&mut self, usage: &TokenUsage) {
        self.runtime.current_node_token_usage.add(usage);
    }

    pub fn observe_node_stall(&mut self, observation: NodeStallObservation) -> TransitionOutcome {
        if !self.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        let observed_at = observation.observed_at;
        if let Some(existing) = self
            .runtime
            .current_stall_observations
            .iter_mut()
            .find(|existing| existing.session_id == observation.session_id)
        {
            *existing = observation;
        } else {
            self.runtime.current_stall_observations.push(observation);
        }
        self.runtime.updated_at = observed_at;
        TransitionOutcome::Applied
    }

    pub fn clear_node_stalls(&mut self, timestamp: f64) {
        self.runtime.current_stall_observations.clear();
        self.runtime.updated_at = timestamp;
    }

    fn set_node_status(
        &mut self,
        node_execution_id: &str,
        status: RuntimeNodeExecutionStatus,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        if execution.status == status {
            return TransitionOutcome::AlreadyApplied;
        }
        if !execution.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        execution.status = status;
        if !status.is_active() {
            execution.completed_at = Some(timestamp);
        }
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
    }

    pub fn state_set(&self) -> ExecutionStateSet {
        match self.state {
            RuntimeExecutionState::Running | RuntimeExecutionState::WaitingApproval => {
                ExecutionStateSet::Active
            }
            RuntimeExecutionState::Interrupted => ExecutionStateSet::Resumable,
            RuntimeExecutionState::Completed
            | RuntimeExecutionState::Failed { .. }
            | RuntimeExecutionState::Aborted => ExecutionStateSet::Finished,
        }
    }

    pub fn is_active(&self) -> bool {
        self.state_set() == ExecutionStateSet::Active
    }

    pub fn is_finished(&self) -> bool {
        self.state_set() == ExecutionStateSet::Finished
    }

    pub fn stop(&mut self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => {
                self.set_interrupted(ExecutionInterruptionReason::Stop);
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Resumable => {
                TransitionOutcome::Rejected(TransitionRejection::AlreadyStopped)
            }
            ExecutionStateSet::Finished => {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            }
        }
    }

    pub fn interrupt(&mut self, reason: ExecutionInterruptionReason) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => {
                self.set_interrupted(reason);
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Resumable if self.interruption_reason == Some(reason) => {
                TransitionOutcome::AlreadyApplied
            }
            ExecutionStateSet::Resumable => {
                self.interruption_reason = Some(reason);
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Finished => TransitionOutcome::NotApplicable,
        }
    }

    pub fn resume(&mut self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Resumable => {
                self.state = RuntimeExecutionState::Running;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Active => {
                TransitionOutcome::Rejected(TransitionRejection::NotResumable)
            }
            ExecutionStateSet::Finished => {
                TransitionOutcome::Rejected(TransitionRejection::NotResumable)
            }
        }
    }

    pub fn abort(&mut self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active | ExecutionStateSet::Resumable => {
                self.state = RuntimeExecutionState::Aborted;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Finished => TransitionOutcome::NotApplicable,
        }
    }

    pub fn request_approval(&mut self) -> TransitionOutcome {
        match &self.state {
            RuntimeExecutionState::Running => {
                self.state = RuntimeExecutionState::WaitingApproval;
                TransitionOutcome::Applied
            }
            RuntimeExecutionState::WaitingApproval => TransitionOutcome::AlreadyApplied,
            _ => TransitionOutcome::Rejected(TransitionRejection::NotActive),
        }
    }

    pub fn approve(&mut self) -> TransitionOutcome {
        match &self.state {
            RuntimeExecutionState::WaitingApproval => {
                self.state = RuntimeExecutionState::Running;
                TransitionOutcome::Applied
            }
            _ => TransitionOutcome::Rejected(TransitionRejection::NotWaitingApproval),
        }
    }

    pub fn reject(&mut self) -> TransitionOutcome {
        self.approve()
    }

    pub fn resolve_approval(&mut self) -> TransitionOutcome {
        self.reject()
    }

    /// Admits approval of a fanout child without changing the execution-level
    /// lifecycle. Fanout child approval is represented by NodeExecution state.
    pub fn admit_fanout_approval(&self) -> TransitionOutcome {
        self.active_only()
    }

    pub fn admit_artifact_submission(&self, target_accepts: bool) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active if target_accepts => TransitionOutcome::Applied,
            ExecutionStateSet::Active => {
                TransitionOutcome::Rejected(TransitionRejection::ArtifactNotAccepted)
            }
            ExecutionStateSet::Resumable | ExecutionStateSet::Finished => {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            }
        }
    }

    pub fn apply_turn_completion(&self, fact: CanonicalNodeFact) -> TurnCompletionDecision {
        let application = match self.state_set() {
            ExecutionStateSet::Active => TurnCompletionApplication::Live,
            ExecutionStateSet::Resumable => TurnCompletionApplication::RecordOnly,
            ExecutionStateSet::Finished => TurnCompletionApplication::Superseded,
        };
        TurnCompletionDecision { application, fact }
    }

    pub fn orphan_interrupt(&mut self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => {
                self.set_interrupted(ExecutionInterruptionReason::Orphan);
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Resumable | ExecutionStateSet::Finished => {
                TransitionOutcome::NotApplicable
            }
        }
    }

    pub fn admit_workflow_turn(&self, context_authorized: bool) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active if context_authorized => TransitionOutcome::Applied,
            ExecutionStateSet::Active => {
                TransitionOutcome::Rejected(TransitionRejection::WorkflowTurnNotAuthorized)
            }
            ExecutionStateSet::Resumable | ExecutionStateSet::Finished => {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            }
        }
    }

    pub fn observe_stall(&self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => TransitionOutcome::Applied,
            ExecutionStateSet::Resumable | ExecutionStateSet::Finished => {
                TransitionOutcome::NotApplicable
            }
        }
    }

    pub fn start_fanout_child(&self) -> TransitionOutcome {
        self.active_only()
    }

    pub fn complete_fanout_child(&self) -> TransitionOutcome {
        self.active_only()
    }

    pub fn complete(&mut self) -> TransitionOutcome {
        match &self.state {
            RuntimeExecutionState::Running | RuntimeExecutionState::WaitingApproval => {
                self.state = RuntimeExecutionState::Completed;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            RuntimeExecutionState::Completed => TransitionOutcome::AlreadyApplied,
            _ => TransitionOutcome::NotApplicable,
        }
    }

    pub fn fail(
        &mut self,
        reason: String,
        kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    ) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => {
                self.state = RuntimeExecutionState::Failed {
                    reason,
                    kind,
                    retry_count,
                };
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Resumable | ExecutionStateSet::Finished => {
                TransitionOutcome::NotApplicable
            }
        }
    }

    /// Applies one durable lifecycle event through the same transition methods
    /// used by live execution.
    pub fn replay_started(&mut self) -> ReplayOutcome {
        match &self.state {
            RuntimeExecutionState::Running => ReplayOutcome::AlreadyApplied,
            _ => ReplayOutcome::Rejected(TransitionRejection::NotActive),
        }
    }

    pub fn replay_approval_requested(&mut self) -> ReplayOutcome {
        transition_to_replay(self.request_approval())
    }

    pub fn replay_approval_resolved(&mut self) -> ReplayOutcome {
        transition_to_replay(self.approve())
    }

    pub fn replay_fanout_approval(&self) -> ReplayOutcome {
        transition_to_replay(self.admit_fanout_approval())
    }

    pub fn replay_completed(&mut self) -> ReplayOutcome {
        transition_to_replay(self.complete())
    }

    pub fn replay_failed(
        &mut self,
        reason: String,
        kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    ) -> ReplayOutcome {
        transition_to_replay(self.fail(reason, kind, retry_count))
    }

    pub fn replay_aborted(&mut self) -> ReplayOutcome {
        transition_to_replay(self.abort())
    }

    pub fn replay_resumed(&mut self) -> ReplayOutcome {
        transition_to_replay(self.resume())
    }

    pub fn replay_completed_at(&mut self, timestamp: f64) -> ReplayOutcome {
        let outcome = self.replay_completed();
        if matches!(
            outcome,
            ReplayOutcome::Applied | ReplayOutcome::AlreadyApplied
        ) {
            let active = self
                .runtime
                .node_executions
                .iter()
                .filter(|execution| execution.status.is_active())
                .map(|execution| {
                    (
                        execution.id.clone(),
                        execution.artifact.clone(),
                        execution.token_usage.clone(),
                    )
                })
                .collect::<Vec<_>>();
            for (node_execution_id, artifact, token_usage) in active {
                let _ = self.complete_node_execution(
                    &node_execution_id,
                    artifact,
                    token_usage,
                    timestamp,
                );
            }
            self.runtime.error_reason = None;
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn replay_failed_at(
        &mut self,
        reason: String,
        kind: NodeExecutionFailureKind,
        timestamp: f64,
    ) -> ReplayOutcome {
        let outcome = self.replay_failed(reason.clone(), kind, None);
        if matches!(
            outcome,
            ReplayOutcome::Applied | ReplayOutcome::AlreadyApplied
        ) {
            let active = self
                .runtime
                .node_executions
                .iter()
                .filter(|execution| execution.status.is_active())
                .map(|execution| execution.id.clone())
                .collect::<Vec<_>>();
            for node_execution_id in active {
                let _ =
                    self.fail_node_execution(&node_execution_id, reason.clone(), kind, timestamp);
            }
            self.runtime.error_reason = Some(reason);
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn replay_aborted_at(&mut self, timestamp: f64) -> ReplayOutcome {
        let outcome = self.replay_aborted();
        if matches!(
            outcome,
            ReplayOutcome::Applied | ReplayOutcome::AlreadyApplied
        ) {
            self.abort_active_node_executions(timestamp);
            self.runtime.error_reason = None;
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn replay_interrupted_at(
        &mut self,
        reason: ExecutionInterruptionReason,
        timestamp: f64,
    ) -> ReplayOutcome {
        let outcome = transition_to_replay(self.interrupt_runtime(reason, timestamp));
        if matches!(
            outcome,
            ReplayOutcome::Applied | ReplayOutcome::AlreadyApplied
        ) {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn replay_resumed_at(&mut self, timestamp: f64) -> ReplayOutcome {
        let outcome = self.replay_resumed();
        if matches!(
            outcome,
            ReplayOutcome::Applied | ReplayOutcome::AlreadyApplied
        ) {
            self.runtime.error_reason = None;
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    /// Restores a pre-commit lifecycle snapshot when the enclosing usecase
    /// transaction did not durably append its decision.
    #[cfg(test)]
    pub fn restore_after_failed_commit(
        &mut self,
        state: RuntimeExecutionState,
        interruption_reason: Option<ExecutionInterruptionReason>,
    ) {
        self.state = state;
        self.interruption_reason = interruption_reason;
    }

    #[cfg(test)]
    pub(crate) fn force_state_for_test(&mut self, state: RuntimeExecutionState) {
        self.state = state;
        if !matches!(self.state, RuntimeExecutionState::Interrupted) {
            self.interruption_reason = None;
        }
    }

    fn active_only(&self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => TransitionOutcome::Applied,
            ExecutionStateSet::Resumable | ExecutionStateSet::Finished => {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            }
        }
    }

    fn set_interrupted(&mut self, reason: ExecutionInterruptionReason) {
        self.state = RuntimeExecutionState::Interrupted;
        self.interruption_reason = Some(reason);
    }
}

fn transition_to_replay(outcome: TransitionOutcome) -> ReplayOutcome {
    match outcome {
        TransitionOutcome::Applied => ReplayOutcome::Applied,
        TransitionOutcome::AlreadyApplied => ReplayOutcome::AlreadyApplied,
        TransitionOutcome::NotApplicable => ReplayOutcome::NotApplicable,
        TransitionOutcome::Rejected(reason) => ReplayOutcome::Rejected(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aggregate(state: RuntimeExecutionState) -> WorkflowExecution {
        WorkflowExecution::restore(state, None)
    }

    fn states() -> [(ExecutionStateSet, RuntimeExecutionState); 3] {
        [
            (ExecutionStateSet::Active, RuntimeExecutionState::Running),
            (
                ExecutionStateSet::Resumable,
                RuntimeExecutionState::Interrupted,
            ),
            (
                ExecutionStateSet::Finished,
                RuntimeExecutionState::Completed,
            ),
        ]
    }

    fn active_states() -> [RuntimeExecutionState; 2] {
        [
            RuntimeExecutionState::Running,
            RuntimeExecutionState::WaitingApproval,
        ]
    }

    fn finished_states() -> [RuntimeExecutionState; 3] {
        [
            RuntimeExecutionState::Completed,
            RuntimeExecutionState::Failed {
                reason: "failed".to_string(),
                kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: None,
            },
            RuntimeExecutionState::Aborted,
        ]
    }

    #[test]
    fn state_sets_are_exhaustive() {
        for (expected, state) in states() {
            assert_eq!(aggregate(state).state_set(), expected);
        }
        assert_eq!(
            aggregate(RuntimeExecutionState::WaitingApproval).state_set(),
            ExecutionStateSet::Active
        );
        assert_eq!(
            aggregate(RuntimeExecutionState::Failed {
                reason: "failure".to_string(),
                kind: NodeExecutionFailureKind::ValidationFailure,
                retry_count: None,
            })
            .state_set(),
            ExecutionStateSet::Finished
        );
        assert_eq!(
            aggregate(RuntimeExecutionState::Aborted).state_set(),
            ExecutionStateSet::Finished
        );
    }

    #[test]
    fn operation_state_matrix_stop_resume_and_abort() {
        for state in active_states() {
            let mut active = aggregate(state);
            assert_eq!(active.stop(), TransitionOutcome::Applied);
            assert_eq!(
                active.stop(),
                TransitionOutcome::Rejected(TransitionRejection::AlreadyStopped)
            );
        }
        assert_eq!(
            aggregate(RuntimeExecutionState::Interrupted).stop(),
            TransitionOutcome::Rejected(TransitionRejection::AlreadyStopped)
        );
        for state in finished_states() {
            let mut finished = aggregate(state);
            assert_eq!(
                finished.stop(),
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            );
        }

        for state in active_states() {
            let mut active = aggregate(state);
            assert_eq!(
                active.resume(),
                TransitionOutcome::Rejected(TransitionRejection::NotResumable)
            );
        }
        let mut resumable = aggregate(RuntimeExecutionState::Interrupted);
        assert_eq!(resumable.resume(), TransitionOutcome::Applied);
        assert_eq!(
            resumable.resume(),
            TransitionOutcome::Rejected(TransitionRejection::NotResumable)
        );
        for state in finished_states() {
            assert_eq!(
                aggregate(state).resume(),
                TransitionOutcome::Rejected(TransitionRejection::NotResumable)
            );
        }

        for state in active_states()
            .into_iter()
            .chain([RuntimeExecutionState::Interrupted])
        {
            let mut execution = aggregate(state);
            assert_eq!(execution.abort(), TransitionOutcome::Applied);
            assert_eq!(execution.abort(), TransitionOutcome::NotApplicable);
        }
        for state in finished_states() {
            assert_eq!(aggregate(state).abort(), TransitionOutcome::NotApplicable);
        }
    }

    #[test]
    fn operation_state_matrix_approval_and_artifact() {
        for operation in [
            WorkflowExecution::approve as fn(&mut WorkflowExecution) -> TransitionOutcome,
            WorkflowExecution::reject,
        ] {
            let mut waiting = aggregate(RuntimeExecutionState::WaitingApproval);
            assert_eq!(operation(&mut waiting), TransitionOutcome::Applied);
            for state in [
                RuntimeExecutionState::Running,
                RuntimeExecutionState::Interrupted,
            ]
            .into_iter()
            .chain(finished_states())
            {
                assert_eq!(
                    operation(&mut aggregate(state)),
                    TransitionOutcome::Rejected(TransitionRejection::NotWaitingApproval)
                );
            }
        }

        for state in active_states() {
            assert_eq!(
                aggregate(state.clone()).admit_artifact_submission(true),
                TransitionOutcome::Applied
            );
            assert_eq!(
                aggregate(state).admit_artifact_submission(false),
                TransitionOutcome::Rejected(TransitionRejection::ArtifactNotAccepted)
            );
        }
        for state in [RuntimeExecutionState::Interrupted]
            .into_iter()
            .chain(finished_states())
        {
            assert_eq!(
                aggregate(state).admit_artifact_submission(true),
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            );
        }

        let mut approval = aggregate(RuntimeExecutionState::Running);
        assert_eq!(approval.request_approval(), TransitionOutcome::Applied);
        assert_eq!(
            approval.request_approval(),
            TransitionOutcome::AlreadyApplied
        );
    }

    #[test]
    fn operation_state_matrix_turn_completion() {
        for (state, application) in active_states()
            .into_iter()
            .map(|state| (state, TurnCompletionApplication::Live))
            .chain([(
                RuntimeExecutionState::Interrupted,
                TurnCompletionApplication::RecordOnly,
            )])
            .chain(
                finished_states()
                    .into_iter()
                    .map(|state| (state, TurnCompletionApplication::Superseded)),
            )
        {
            let fact = CanonicalNodeFact::Failed {
                reason: "exit 1".to_string(),
                kind: NodeExecutionFailureKind::ValidationFailure,
            };
            assert_eq!(
                aggregate(state).apply_turn_completion(fact.clone()),
                TurnCompletionDecision { application, fact }
            );
        }
    }

    #[test]
    fn operation_state_matrix_orphan_turn_and_stall() {
        for state in active_states() {
            let mut active = aggregate(state);
            assert_eq!(active.orphan_interrupt(), TransitionOutcome::Applied);
            assert_eq!(active.orphan_interrupt(), TransitionOutcome::NotApplicable);
        }
        for state in [RuntimeExecutionState::Interrupted]
            .into_iter()
            .chain(finished_states())
        {
            assert_eq!(
                aggregate(state.clone()).orphan_interrupt(),
                TransitionOutcome::NotApplicable
            );
            assert_eq!(
                aggregate(state.clone()).admit_workflow_turn(true),
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            );
            assert_eq!(
                aggregate(state).observe_stall(),
                TransitionOutcome::NotApplicable
            );
        }
        for state in active_states() {
            assert_eq!(
                aggregate(state.clone()).admit_workflow_turn(true),
                TransitionOutcome::Applied
            );
            assert_eq!(
                aggregate(state.clone()).admit_workflow_turn(false),
                TransitionOutcome::Rejected(TransitionRejection::WorkflowTurnNotAuthorized)
            );
            assert_eq!(aggregate(state).observe_stall(), TransitionOutcome::Applied);
        }
    }

    #[test]
    fn fanout_and_interrupt_cells_are_closed() {
        for state in active_states() {
            let expected = TransitionOutcome::Applied;
            assert_eq!(aggregate(state.clone()).start_fanout_child(), expected);
            assert_eq!(
                aggregate(state).complete_fanout_child(),
                TransitionOutcome::Applied
            );
        }
        for state in [RuntimeExecutionState::Interrupted]
            .into_iter()
            .chain(finished_states())
        {
            let expected = TransitionOutcome::Rejected(TransitionRejection::NotActive);
            assert_eq!(
                aggregate(state.clone()).start_fanout_child(),
                expected.clone()
            );
            assert_eq!(aggregate(state).complete_fanout_child(), expected);
        }

        let mut running = aggregate(RuntimeExecutionState::Running);
        assert_eq!(
            running.interrupt(ExecutionInterruptionReason::Crash),
            TransitionOutcome::Applied
        );
        assert_eq!(
            running.interrupt(ExecutionInterruptionReason::Crash),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            aggregate(RuntimeExecutionState::Completed)
                .interrupt(ExecutionInterruptionReason::Crash),
            TransitionOutcome::NotApplicable
        );
    }

    fn restored_execution(state: RuntimeExecutionState) -> WorkflowExecution {
        WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "workflow".to_string(),
                nodes: vec![crate::domain::workflow::NodeDefinition {
                    name: "implement".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            lifecycle: WorkflowExecution::lifecycle_from_state(state),
            ..WorkflowExecutionRestore::default()
        })
    }

    #[test]
    fn aggregate_owns_node_attempt_session_and_terminal_fact() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                Some("node-execution-1".to_string()),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.attach_node_session(&node_execution_id, "session-1".to_string(), 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.complete_node_execution(
                &node_execution_id,
                Some(serde_json::json!({"ok": true})),
                Some(TokenUsage {
                    input_tokens: 2,
                    output_tokens: 3,
                }),
                12.0,
            ),
            TransitionOutcome::Applied
        );

        let node = &execution.node_executions()[0];
        assert_eq!(node.status, RuntimeNodeExecutionStatus::Succeeded);
        assert_eq!(node.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            node.artifact.as_ref(),
            Some(&serde_json::json!({"ok": true}))
        );
        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 13.0),
            TransitionOutcome::AlreadyApplied
        );
    }

    #[test]
    fn node_attempt_failure_abort_and_approval_transitions_are_closed() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let first_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                Some("node-execution-1".to_string()),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.mark_node_waiting_approval(&first_id, 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.mark_node_waiting_approval(&first_id, 11.5),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.mark_node_running(&first_id, 12.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.fail_node_execution(
                &first_id,
                "invalid".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                13.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.fail_node_execution(
                &first_id,
                "invalid".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                14.0,
            ),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.complete_node_execution(&first_id, None, None, 15.0),
            TransitionOutcome::NotApplicable
        );

        let second_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                2,
                None,
                Some("node-execution-2".to_string()),
                16.0,
            )
            .unwrap();
        assert_eq!(
            execution.abort_node_execution(&second_id, 17.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.abort_node_execution(&second_id, 18.0),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(execution.node_execution_counts["implement"], 2);
        assert_eq!(execution.node_executions.len(), 2);
    }

    #[test]
    fn interrupted_turn_completion_records_canonical_fact_without_resuming() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                Some("node-execution-1".to_string()),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.abort_node_execution(&node_execution_id, 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.interrupt(ExecutionInterruptionReason::Crash),
            TransitionOutcome::Applied
        );

        let fact = CanonicalNodeFact::Failed {
            reason: "exit 1".to_string(),
            kind: NodeExecutionFailureKind::ValidationFailure,
        };
        let decision =
            execution.apply_observed_turn(&node_execution_id, fact.clone(), None, None, 12.0);

        assert_eq!(
            decision,
            TurnCompletionDecision {
                application: TurnCompletionApplication::RecordOnly,
                fact,
            }
        );
        assert_eq!(execution.state(), &RuntimeExecutionState::Interrupted);
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Failed
        );
    }

    #[test]
    fn fanout_child_completion_updates_child_and_node_as_one_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                Some(FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                }),
                Some("child-execution-1".to_string()),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.install_fanout(
                FanoutRuntimeState {
                    parent_node_name: "fanout".to_string(),
                    parent_node_execution_id: "parent-execution-1".to_string(),
                    children: vec![FanoutChildRuntime {
                        node_execution_id: node_execution_id.clone(),
                        node_name: "implement".to_string(),
                        session_id: "session-1".to_string(),
                        state: FanoutChildRuntimeState::Running,
                        result: None,
                        artifact: None,
                        contract: None,
                        failure_kind: None,
                        failure_disposition: None,
                        token_usage: TokenUsage::default(),
                        attempt: 1,
                        completed_at: None,
                    }],
                },
                11.0,
            ),
            TransitionOutcome::Applied
        );

        assert_eq!(
            execution.complete_fanout_child_execution(
                &node_execution_id,
                Some("done".to_string()),
                Some(serde_json::json!({"ok": true})),
                Some("result".to_string()),
                TokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                },
                12.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.fanout_runtime().unwrap().children[0].state,
            FanoutChildRuntimeState::Completed
        );
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Succeeded
        );
    }

    #[test]
    fn fanout_child_start_failure_and_sibling_interrupt_are_aggregate_transitions() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution
            .begin_node_attempt(
                "fanout".to_string(),
                NodeKindName::Fanout,
                1,
                None,
                Some("parent-execution-1".to_string()),
                10.0,
            )
            .unwrap();
        for (index, child_id) in ["child-1", "child-2"].into_iter().enumerate() {
            assert_eq!(
                execution
                    .start_fanout_child_execution(
                        "fanout".to_string(),
                        "parent-execution-1".to_string(),
                        child_id.to_string(),
                        "implement".to_string(),
                        NodeKindName::Session,
                        1,
                        FanoutParentRef {
                            parent_node: "fanout".to_string(),
                            parent_attempt: 1,
                            item_index: Some(index),
                            child_index: 0,
                        },
                        11.0 + index as f64,
                    )
                    .unwrap(),
                TransitionOutcome::Applied
            );
        }
        assert_eq!(
            execution.fail_fanout_child_execution(
                "child-1",
                "child failed".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
                13.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.fail_fanout_child_execution(
                "child-1",
                "child failed".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
                14.0,
            ),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.interrupt_running_fanout_children(Some("child-1"), 15.0),
            vec!["child-2".to_string()]
        );
        let fanout = execution.fanout_runtime().unwrap();
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Failed);
        assert_eq!(
            fanout.children[1].state,
            FanoutChildRuntimeState::Interrupted
        );
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == "child-1")
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::Failed
        );
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == "child-2")
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::Aborted
        );
    }
}
