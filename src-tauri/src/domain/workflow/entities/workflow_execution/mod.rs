//! Workflow execution lifecycle aggregate.
//!
//! This aggregate is the single authority for execution lifecycle admission and
//! transitions. Runtime orchestration and event replay both call these methods;
//! neither is allowed to assign lifecycle state directly.

use std::collections::HashMap;

use crate::domain::workflow::services::routing::LoopGuardResetBaselines;
use crate::domain::workflow::services::{
    fanout as workflow_fanout, history as workflow_history, routing as workflow_routing,
    submission as workflow_submission, transition as workflow_transition,
};
use crate::domain::workflow::value_objects::{
    ExecutionInterruptionReason, ExecutionOrigin, FanoutParentRef, NodeCompletionSignal,
    NodeCompletionSignalState, NodeExecutionFailureKind, NodeHistoryEntry, NodeKindName,
    RuntimeArtifact, RuntimeExecutionState, TokenUsage, WorkflowDefinition,
};
use crate::domain::workflow::FailureDisposition;
use crate::domain::workflow::WorkflowEvent;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowDefaults;

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
    Paused,
    WaitingApproval,
    Succeeded,
    Failed,
    Aborted,
}

impl RuntimeNodeExecutionStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::WaitingApproval)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeNodeExecutionFailure {
    pub reason: String,
    pub kind: NodeExecutionFailureKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalAttemptTarget {
    pub node_execution_id: String,
    pub node_name: String,
    pub session_id: Option<String>,
    pub attempt: u32,
    pub fanout_parent: Option<FanoutParentRef>,
    pub artifact: Option<serde_json::Value>,
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
    pub completion_signals: NodeCompletionSignalState,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

impl RuntimeNodeExecution {
    pub fn can_retry(&self) -> bool {
        self.status == RuntimeNodeExecutionStatus::Failed
            || (matches!(
                self.status,
                RuntimeNodeExecutionStatus::Running | RuntimeNodeExecutionStatus::Paused
            ) && self.completion_signals.is_partial())
    }

    pub fn can_restart_paused_command(&self) -> bool {
        self.kind == NodeKindName::Command && self.status == RuntimeNodeExecutionStatus::Paused
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

    pub fn pause(&mut self) -> TransitionOutcome {
        match self.status {
            RuntimeNodeExecutionStatus::Paused => TransitionOutcome::AlreadyApplied,
            RuntimeNodeExecutionStatus::Running
                if self.completion_signals != NodeCompletionSignalState::StopReceived =>
            {
                self.transition_status(RuntimeNodeExecutionStatus::Paused, None)
            }
            _ => TransitionOutcome::NotApplicable,
        }
    }

    pub fn resume(&mut self) -> TransitionOutcome {
        if self.status == RuntimeNodeExecutionStatus::Running {
            return TransitionOutcome::AlreadyApplied;
        }
        if self.status != RuntimeNodeExecutionStatus::Paused {
            return TransitionOutcome::NotApplicable;
        }
        self.transition_status(RuntimeNodeExecutionStatus::Running, None)
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

    pub fn record_completion_signal(&mut self, signal: NodeCompletionSignal) -> TransitionOutcome {
        if self.kind != NodeKindName::Session || !self.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        let next = match (self.completion_signals, signal) {
            (NodeCompletionSignalState::Pending, NodeCompletionSignal::Submit) => {
                NodeCompletionSignalState::SubmitReceived
            }
            (NodeCompletionSignalState::Pending, NodeCompletionSignal::Stop) => {
                NodeCompletionSignalState::StopReceived
            }
            (NodeCompletionSignalState::SubmitReceived, NodeCompletionSignal::Stop)
            | (NodeCompletionSignalState::StopReceived, NodeCompletionSignal::Submit) => {
                NodeCompletionSignalState::Ready
            }
            (NodeCompletionSignalState::SubmitReceived, NodeCompletionSignal::Submit)
            | (NodeCompletionSignalState::StopReceived, NodeCompletionSignal::Stop)
            | (NodeCompletionSignalState::Ready, _) => {
                return TransitionOutcome::AlreadyApplied;
            }
        };
        self.completion_signals = next;
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
        if self.kind == NodeKindName::Session && !self.completion_signals.is_ready() {
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
            workflow_defaults: WorkflowDefaults,
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
    #[cfg(test)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeCompletionHandshakeDecision {
    AwaitingSignal,
    CompleteAuto,
    RequestApproval,
    AlreadySettled,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppliedNodeCompletionHandshake {
    pub advance: Option<ExecutionAdvanceDecision>,
    pub events: Vec<WorkflowEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSubmitTarget {
    pub node_name: String,
    pub session_id: Option<String>,
    pub attempt: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRestartMode {
    ExplicitRetry,
    CommandResume,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestartedNodeAttempt {
    pub attempt: RuntimeNodeExecution,
    pub fanout_child: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSubmitRejection {
    ExecutionNotActive,
    NodeExecutionNotFound,
    AttemptNotCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStopRejection {
    NodeExecutionNotFound,
    SessionDoesNotOwnAttempt,
}

/// Stable rejection reasons returned by aggregate admission methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionRejection {
    #[cfg(test)]
    AlreadyStopped,
    NotActive,
    #[cfg(test)]
    NotResumable,
    #[cfg(test)]
    NotWaitingApproval,
    ArtifactNotAccepted,
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
    #[cfg(test)]
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
    #[cfg(test)]
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
        #[cfg(test)]
        let interruption_reason = matches!(state, RuntimeExecutionState::Interrupted)
            .then_some(ExecutionInterruptionReason::Crash);
        #[cfg(not(test))]
        let interruption_reason = None;
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

    pub fn transition_running(&mut self) -> TransitionOutcome {
        match self.state {
            #[cfg(test)]
            RuntimeExecutionState::Interrupted => self.resume(),
            #[cfg(test)]
            RuntimeExecutionState::WaitingApproval => self.resolve_approval(),
            RuntimeExecutionState::Running => TransitionOutcome::AlreadyApplied,
            RuntimeExecutionState::Completed | RuntimeExecutionState::Aborted => {
                TransitionOutcome::NotApplicable
            }
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
        node_execution_id: String,
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

    pub fn start_current_node_execution(
        &mut self,
        node_execution_id: String,
        timestamp: f64,
    ) -> String {
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
        self.start_node_execution(node_name, kind, attempt, None, node_execution_id, timestamp)
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

    pub fn apply_advance_at(
        &mut self,
        next_node_execution_id: String,
        timestamp: f64,
    ) -> Result<ExecutionAdvanceDecision, crate::domain::workflow::WorkflowError> {
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
        let decision = match self.decide_next_node() {
            NextNodeDecision::Failed { reason } => {
                return Err(crate::domain::workflow::WorkflowError::invalid_state(
                    reason,
                ));
            }
            decision => decision,
        };
        Ok(match decision {
            NextNodeDecision::Completed => {
                let _ = self.transition_completed();
                self.runtime.updated_at = timestamp;
                ExecutionAdvanceDecision::Persist
            }
            NextNodeDecision::Failed { .. } => unreachable!("routing failure returned above"),
            NextNodeDecision::TransitionTo(name) => {
                let index = self
                    .runtime
                    .workflow
                    .nodes
                    .iter()
                    .position(|node| node.name == name)
                    .expect("routing decision must reference a known node");
                self.apply_transition_index(index, &name, next_node_execution_id, timestamp);
                if self.runtime.workflow.nodes[index].is_fanout() {
                    ExecutionAdvanceDecision::StartFanout
                } else {
                    ExecutionAdvanceDecision::TransitionAndStart
                }
            }
        })
    }

    pub fn retry_current_node_at(
        &mut self,
        new_node_execution_id: String,
        timestamp: f64,
    ) -> TransitionOutcome {
        self.retry_current_node_with(
            new_node_execution_id,
            timestamp,
            RuntimeNodeExecution::can_retry,
        )
    }

    pub fn restart_node_attempt_at(
        &mut self,
        node_execution_id: &str,
        new_node_execution_id: String,
        timestamp: f64,
        mode: NodeRestartMode,
    ) -> Option<RestartedNodeAttempt> {
        let fanout_child = self
            .runtime
            .node_executions
            .iter()
            .find(|attempt| attempt.id == node_execution_id)
            .is_some_and(|attempt| attempt.fanout_parent.is_some());
        let attempt = if fanout_child {
            match mode {
                NodeRestartMode::ExplicitRetry => {
                    self.retry_fanout_child_at(node_execution_id, new_node_execution_id, timestamp)
                }
                NodeRestartMode::CommandResume => self.restart_paused_fanout_command_at(
                    node_execution_id,
                    new_node_execution_id,
                    timestamp,
                ),
            }?
        } else {
            let outcome = match mode {
                NodeRestartMode::ExplicitRetry => {
                    self.retry_current_node_at(new_node_execution_id.clone(), timestamp)
                }
                NodeRestartMode::CommandResume => {
                    self.restart_paused_current_command_at(new_node_execution_id.clone(), timestamp)
                }
            };
            if outcome != TransitionOutcome::Applied {
                return None;
            }
            self.runtime
                .node_executions
                .iter()
                .find(|attempt| attempt.id == new_node_execution_id)
                .cloned()?
        };
        Some(RestartedNodeAttempt {
            attempt,
            fanout_child,
        })
    }

    pub fn restart_paused_current_command_at(
        &mut self,
        new_node_execution_id: String,
        timestamp: f64,
    ) -> TransitionOutcome {
        self.retry_current_node_with(
            new_node_execution_id,
            timestamp,
            RuntimeNodeExecution::can_restart_paused_command,
        )
    }

    fn retry_current_node_with(
        &mut self,
        new_node_execution_id: String,
        timestamp: f64,
        admission: fn(&RuntimeNodeExecution) -> bool,
    ) -> TransitionOutcome {
        let node_index = self.runtime.current_node_index;
        let node_name = self.runtime.workflow.nodes[node_index].name.clone();
        let current_attempt = self
            .runtime
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        let Some(node_execution_id) = self
            .runtime
            .node_executions
            .iter()
            .rev()
            .find(|execution| {
                execution.node_name == node_name
                    && execution.attempt == current_attempt
                    && execution.fanout_parent.is_none()
            })
            .filter(|execution| admission(execution))
            .map(|execution| execution.id.clone())
        else {
            return TransitionOutcome::NotApplicable;
        };
        if self.request_node_restart_with(&node_execution_id, timestamp, admission)
            != TransitionOutcome::Applied
        {
            return TransitionOutcome::NotApplicable;
        }
        self.state = RuntimeExecutionState::Running;
        self.interruption_reason = None;
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
        self.start_current_node_execution(new_node_execution_id, timestamp);
        TransitionOutcome::Applied
    }

    pub fn retry_fanout_child_at(
        &mut self,
        node_execution_id: &str,
        new_node_execution_id: String,
        timestamp: f64,
    ) -> Option<RuntimeNodeExecution> {
        self.retry_fanout_child_with(
            node_execution_id,
            new_node_execution_id,
            timestamp,
            RuntimeNodeExecution::can_retry,
        )
    }

    pub fn restart_paused_fanout_command_at(
        &mut self,
        node_execution_id: &str,
        new_node_execution_id: String,
        timestamp: f64,
    ) -> Option<RuntimeNodeExecution> {
        self.retry_fanout_child_with(
            node_execution_id,
            new_node_execution_id,
            timestamp,
            RuntimeNodeExecution::can_restart_paused_command,
        )
    }

    fn retry_fanout_child_with(
        &mut self,
        node_execution_id: &str,
        new_node_execution_id: String,
        timestamp: f64,
        admission: fn(&RuntimeNodeExecution) -> bool,
    ) -> Option<RuntimeNodeExecution> {
        let target = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)?
            .clone();
        let fanout_parent = target.fanout_parent.clone()?;
        if !admission(&target) {
            return None;
        }
        let fanout = self.runtime.fanout_runtime.as_ref()?;
        let current_child = fanout
            .children
            .iter()
            .find(|child| child.node_execution_id == node_execution_id)?;
        if current_child.state == FanoutChildRuntimeState::Completed {
            return None;
        }
        let parent_node_execution_id = fanout.parent_node_execution_id.clone();
        let new_attempt = self
            .runtime
            .node_execution_counts
            .get(&target.node_name)
            .copied()
            .unwrap_or(target.attempt)
            .max(target.attempt)
            .saturating_add(1);
        if self.request_node_restart_with(node_execution_id, timestamp, admission)
            != TransitionOutcome::Applied
        {
            return None;
        }
        self.increase_node_attempt_count_to(target.node_name.clone(), new_attempt, timestamp);
        if self
            .start_fanout_child_execution(
                fanout_parent.parent_node.clone(),
                parent_node_execution_id,
                new_node_execution_id.clone(),
                target.node_name,
                target.kind,
                new_attempt,
                fanout_parent,
                timestamp,
            )
            .ok()?
            != TransitionOutcome::Applied
        {
            return None;
        }
        self.runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == new_node_execution_id)
            .cloned()
    }

    pub fn plan_approval_application(
        &self,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<workflow_transition::ApprovalApplicationPlan, crate::domain::workflow::WorkflowError>
    {
        workflow_transition::plan_approval_application(
            &self.runtime.workflow,
            self.runtime.current_node_index,
            self.runtime.node_executions.iter().rev().any(|execution| {
                execution.fanout_parent.is_none()
                    && execution.node_name
                        == self.runtime.workflow.nodes[self.runtime.current_node_index].name
                    && execution.status == RuntimeNodeExecutionStatus::WaitingApproval
            }),
            application,
        )
    }

    pub fn resolve_approval_attempt_target(
        &self,
        node_name: &str,
        node_execution_id: Option<&str>,
    ) -> Result<ApprovalAttemptTarget, crate::domain::workflow::WorkflowError> {
        if !self.is_active() {
            return Err(crate::domain::workflow::WorkflowError::invalid_state(
                "workflow execution is not active",
            ));
        }
        let candidates = self
            .runtime
            .node_executions
            .iter()
            .filter(|attempt| attempt.node_name == node_name && attempt.status.is_active())
            .collect::<Vec<_>>();
        let target = if let Some(node_execution_id) = node_execution_id {
            candidates
                .into_iter()
                .find(|attempt| attempt.id == node_execution_id)
                .ok_or_else(|| {
                    crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(format!(
							"active NodeExecution '{node_execution_id}' for node '{node_name}' was not found"
						))
                })?
        } else {
            match candidates.as_slice() {
                [target] => *target,
                [] => {
                    return Err(crate::domain::workflow::WorkflowError::invalid_state(
                        format!("node '{node_name}' has no active execution"),
                    ));
                }
                multiple => {
                    let ids = multiple
                        .iter()
                        .map(|attempt| attempt.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(crate::domain::workflow::WorkflowError::invalid_state(
						format!(
							"node '{node_name}' has {} active executions; node_execution_id is required; candidates: [{ids}]",
							multiple.len()
						),
					));
                }
            }
        };
        if target.status != RuntimeNodeExecutionStatus::WaitingApproval {
            return Err(
                crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(format!(
                    "NodeExecution '{}' is not waiting for approval",
                    target.id
                )),
            );
        }
        let node = self
            .runtime
            .workflow
            .nodes
            .iter()
            .find(|node| node.name == node_name)
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::validation(format!(
                    "Node '{node_name}' not found in workflow"
                ))
            })?;
        if !node.requires_approval_completion() {
            return Err(
                crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "node does not declare completion: approval".to_string(),
                ),
            );
        }
        if target.fanout_parent.is_none()
            && self
                .runtime
                .workflow
                .nodes
                .get(self.runtime.current_node_index)
                .is_none_or(|current| current.name != node_name)
        {
            return Err(
                crate::domain::workflow::WorkflowError::UnauthorizedApprovalTarget(
                    "node is not the current workflow node".to_string(),
                ),
            );
        }
        Ok(ApprovalAttemptTarget {
            node_execution_id: target.id.clone(),
            node_name: target.node_name.clone(),
            session_id: target.session_id.clone(),
            attempt: target.attempt,
            fanout_parent: target.fanout_parent.clone(),
            artifact: target.artifact.clone(),
        })
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

    fn apply_transition_index(
        &mut self,
        node_index: usize,
        node_name: &str,
        node_execution_id: String,
        timestamp: f64,
    ) {
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
        self.start_current_node_execution(node_execution_id, timestamp);
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
        node_execution_id: String,
        timestamp: f64,
    ) -> Result<String, TransitionRejection> {
        if !self.is_active() {
            return Err(TransitionRejection::NotActive);
        }
        let id = node_execution_id;
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
            completion_signals: NodeCompletionSignalState::Pending,
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
        let outcome = execution.attach_session(session_id.clone());
        if outcome == TransitionOutcome::Applied {
            if let Some(child) = self.runtime.fanout_runtime.as_mut().and_then(|fanout| {
                fanout
                    .children
                    .iter_mut()
                    .find(|child| child.node_execution_id == node_execution_id)
            }) {
                child.session_id = session_id;
            }
            self.runtime.updated_at = timestamp;
        }
        outcome
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
    ) -> TransitionOutcome {
        let Some(execution_index) = self
            .runtime
            .node_executions
            .iter()
            .position(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        if !self.runtime.node_executions[execution_index]
            .status
            .is_active()
        {
            return TransitionOutcome::NotApplicable;
        }
        let fanout_child = self.runtime.node_executions[execution_index]
            .fanout_parent
            .is_some();
        let fanout_child_index = if fanout_child {
            let Some(index) = self.runtime.fanout_runtime.as_ref().and_then(|fanout| {
                fanout
                    .children
                    .iter()
                    .position(|child| child.node_execution_id == node_execution_id)
            }) else {
                return TransitionOutcome::NotApplicable;
            };
            Some(index)
        } else {
            None
        };
        self.runtime.node_executions[execution_index].artifact = Some(output.clone());
        if fanout_child {
            let child = &mut self
                .runtime
                .fanout_runtime
                .as_mut()
                .expect("fanout child index was validated")
                .children[fanout_child_index.expect("fanout child index was resolved")];
            child.result = result;
            child.artifact = Some(output.clone());
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
        }
        self.runtime.updated_at = timestamp;
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

    pub fn pause_node_execution(
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
        let outcome = execution.pause();
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn resume_node_execution(
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
        let outcome = execution.resume();
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

    pub fn request_node_retry(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> TransitionOutcome {
        self.request_node_restart_with(
            node_execution_id,
            timestamp,
            RuntimeNodeExecution::can_retry,
        )
    }

    fn request_node_restart_with(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
        admission: fn(&RuntimeNodeExecution) -> bool,
    ) -> TransitionOutcome {
        let Some(target) = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        if !admission(target) {
            return TransitionOutcome::NotApplicable;
        }
        let target_is_active = target.status.is_active();
        let target_is_fanout_child = target.fanout_parent.is_some();
        let target_node_name = target.node_name.clone();
        let target_attempt = target.attempt;
        if target_is_active {
            let _ = self.abort_node_execution(node_execution_id, timestamp);
            if target_is_fanout_child {
                if let Some(child) = self.runtime.fanout_runtime.as_mut().and_then(|fanout| {
                    fanout
                        .children
                        .iter_mut()
                        .find(|child| child.node_execution_id == node_execution_id)
                }) {
                    let _ = child.interrupt(timestamp);
                }
            }
        }
        if !target_is_fanout_child {
            if let Some(node_index) = self
                .runtime
                .workflow
                .nodes
                .iter()
                .position(|node| node.name == target_node_name)
            {
                self.clear_artifacts_for_new_execution(node_index);
            }
            self.runtime.current_session_id = None;
            self.runtime.current_node_token_usage = TokenUsage::default();
        }
        self.runtime
            .current_stall_observations
            .retain(|observation| {
                observation.node_name != target_node_name || observation.attempt != target_attempt
            });
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
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

    pub fn record_reused_node_completion(
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
        let outcome = execution.record_completed(artifact, token_usage, timestamp);
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn record_node_completion_signal(
        &mut self,
        node_execution_id: &str,
        signal: NodeCompletionSignal,
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
        let outcome = execution.record_completion_signal(signal);
        if outcome == TransitionOutcome::Applied {
            self.runtime.updated_at = timestamp;
        }
        outcome
    }

    pub fn record_provider_stop(
        &mut self,
        node_execution_id: &str,
        agent_session_id: &str,
        timestamp: f64,
    ) -> Result<TransitionOutcome, ProviderStopRejection> {
        let execution = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
            .ok_or(ProviderStopRejection::NodeExecutionNotFound)?;
        if execution.session_id.as_deref() != Some(agent_session_id) {
            return Err(ProviderStopRejection::SessionDoesNotOwnAttempt);
        }
        if !execution.status.is_active() {
            return Ok(TransitionOutcome::NotApplicable);
        }
        Ok(self.record_node_completion_signal(
            node_execution_id,
            NodeCompletionSignal::Stop,
            timestamp,
        ))
    }

    pub fn admit_node_submit(
        &self,
        node_execution_id: &str,
    ) -> Result<NodeSubmitTarget, NodeSubmitRejection> {
        if !self.is_active() {
            return Err(NodeSubmitRejection::ExecutionNotActive);
        }
        let requested = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
            .ok_or(NodeSubmitRejection::NodeExecutionNotFound)?;
        let node_name = requested.node_name.as_str();
        let current_node = self
            .runtime
            .workflow
            .nodes
            .get(self.runtime.current_node_index)
            .ok_or(NodeSubmitRejection::AttemptNotCurrent)?;
        let parent_attempt = self
            .runtime
            .node_execution_counts
            .get(&current_node.name)
            .copied()
            .unwrap_or(1);
        let execution = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| {
                execution.id == node_execution_id
                    && execution.node_name == node_name
                    && execution.status.is_active()
                    && match execution.fanout_parent.as_ref() {
                        None => {
                            current_node.name == node_name && execution.attempt == parent_attempt
                        }
                        Some(parent) => {
                            current_node.is_fanout()
                                && parent.parent_node == current_node.name
                                && parent.parent_attempt == parent_attempt
                        }
                    }
            })
            .ok_or(NodeSubmitRejection::AttemptNotCurrent)?;
        Ok(NodeSubmitTarget {
            node_name: execution.node_name.clone(),
            session_id: execution.session_id.clone(),
            attempt: execution.attempt,
        })
    }

    pub fn decide_node_completion_handshake(
        &self,
        node_execution_id: &str,
    ) -> NodeCompletionHandshakeDecision {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
        else {
            return NodeCompletionHandshakeDecision::NotApplicable;
        };
        match execution.status {
            RuntimeNodeExecutionStatus::WaitingApproval | RuntimeNodeExecutionStatus::Succeeded => {
                return NodeCompletionHandshakeDecision::AlreadySettled;
            }
            RuntimeNodeExecutionStatus::Failed | RuntimeNodeExecutionStatus::Aborted => {
                return NodeCompletionHandshakeDecision::NotApplicable;
            }
            RuntimeNodeExecutionStatus::Running | RuntimeNodeExecutionStatus::Paused => {}
        }
        if !execution.completion_signals.is_ready() {
            return NodeCompletionHandshakeDecision::AwaitingSignal;
        }
        let Some(node) = self
            .runtime
            .workflow
            .nodes
            .iter()
            .find(|node| node.name == execution.node_name)
            .filter(|node| node.is_session())
        else {
            return NodeCompletionHandshakeDecision::NotApplicable;
        };
        match workflow_transition::decide_completion_disposition(node) {
            workflow_transition::CompletionDisposition::Complete => {
                NodeCompletionHandshakeDecision::CompleteAuto
            }
            workflow_transition::CompletionDisposition::RequestApproval => {
                NodeCompletionHandshakeDecision::RequestApproval
            }
        }
    }

    pub fn apply_node_completion_handshake(
        &mut self,
        node_execution_id: &str,
        next_node_execution_id: String,
        timestamp: f64,
    ) -> Result<AppliedNodeCompletionHandshake, crate::domain::workflow::WorkflowError> {
        match self.decide_node_completion_handshake(node_execution_id) {
            NodeCompletionHandshakeDecision::AwaitingSignal
            | NodeCompletionHandshakeDecision::AlreadySettled => {
                Ok(AppliedNodeCompletionHandshake {
                    advance: None,
                    events: Vec::new(),
                })
            }
            NodeCompletionHandshakeDecision::NotApplicable => Err(
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "node execution '{node_execution_id}' cannot settle its completion handshake"
                )),
            ),
            NodeCompletionHandshakeDecision::RequestApproval => {
                let node = self
                    .runtime
                    .node_executions
                    .iter()
                    .find(|node| node.id == node_execution_id)
                    .cloned()
                    .ok_or_else(|| {
                        crate::domain::workflow::WorkflowError::invalid_state(format!(
                            "node execution '{node_execution_id}' disappeared"
                        ))
                    })?;
                if self.mark_node_waiting_approval(node_execution_id, timestamp)
                    != TransitionOutcome::Applied
                {
                    return Err(crate::domain::workflow::WorkflowError::invalid_state(
                        format!("node execution '{node_execution_id}' cannot wait for Approval"),
                    ));
                }
                Ok(AppliedNodeCompletionHandshake {
                    advance: None,
                    events: vec![WorkflowEvent::ApprovalRequested {
                        execution_id: self.id.clone(),
                        node_execution_id: node.id,
                        node_name: node.node_name,
                        timestamp,
                    }],
                })
            }
            NodeCompletionHandshakeDecision::CompleteAuto => self.apply_auto_node_completion(
                node_execution_id,
                next_node_execution_id,
                timestamp,
            ),
        }
    }

    fn apply_auto_node_completion(
        &mut self,
        node_execution_id: &str,
        next_node_execution_id: String,
        timestamp: f64,
    ) -> Result<AppliedNodeCompletionHandshake, crate::domain::workflow::WorkflowError> {
        let node = self
            .runtime
            .node_executions
            .iter()
            .find(|node| node.id == node_execution_id)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "node execution '{node_execution_id}' disappeared"
                ))
            })?;
        if node.fanout_parent.is_some() {
            return self.apply_auto_fanout_child_completion(
                node,
                next_node_execution_id,
                timestamp,
            );
        }

        let output = self.runtime.artifacts.get(&node.node_name).cloned();
        let artifact = output
            .as_ref()
            .and_then(|output| output.artifact.clone())
            .or_else(|| node.artifact.clone());
        let result = output.as_ref().and_then(|output| output.result.clone());
        let contract = output.as_ref().and_then(|output| output.contract.clone());
        if self.complete_node_execution(node_execution_id, artifact.clone(), None, timestamp)
            != TransitionOutcome::Applied
        {
            return Err(crate::domain::workflow::WorkflowError::invalid_state(
                format!("node execution '{node_execution_id}' cannot complete"),
            ));
        }
        let entry = self.make_node_history_entry_at(result, artifact, contract, timestamp);
        self.record_history_entry(entry, timestamp);
        let advance = self.apply_advance_at(next_node_execution_id, timestamp)?;
        Ok(AppliedNodeCompletionHandshake {
            advance: Some(advance),
            events: Vec::new(),
        })
    }

    fn apply_auto_fanout_child_completion(
        &mut self,
        node: RuntimeNodeExecution,
        next_node_execution_id: String,
        timestamp: f64,
    ) -> Result<AppliedNodeCompletionHandshake, crate::domain::workflow::WorkflowError> {
        let child = self
            .runtime
            .fanout_runtime
            .as_ref()
            .and_then(|fanout| {
                fanout
                    .children
                    .iter()
                    .find(|child| child.node_execution_id == node.id)
            })
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "fanout child '{}' disappeared",
                    node.id
                ))
            })?;
        if self.complete_fanout_child_execution(
            &node.id,
            child.result.clone(),
            child.artifact.clone(),
            child.contract.clone(),
            child.token_usage.clone(),
            timestamp,
        ) != TransitionOutcome::Applied
        {
            return Err(crate::domain::workflow::WorkflowError::invalid_state(
                format!("fanout child '{}' cannot complete", node.id),
            ));
        }
        if !child.session_id.is_empty() {
            self.clear_stalls_for_session(&child.session_id, timestamp);
        }
        self.record_successful_node_completion(&child.node_name, timestamp);
        let mut events = vec![WorkflowEvent::NodeCompleted {
            execution_id: self.id.clone(),
            node_execution_id: node.id,
            node_name: child.node_name,
            attempt: child.attempt,
            result_summary: child.result,
            token_usage: Some(child.token_usage),
            timestamp,
        }];
        let all_done = self.runtime.fanout_runtime.as_ref().is_some_and(|fanout| {
            fanout.children.iter().all(|child| {
                matches!(
                    child.state,
                    FanoutChildRuntimeState::Completed | FanoutChildRuntimeState::Failed
                )
            })
        });
        if !all_done {
            self.touch(timestamp);
            return Ok(AppliedNodeCompletionHandshake {
                advance: None,
                events,
            });
        }

        let fanout = self.runtime.fanout_runtime.as_ref().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(
                "fanout parent completion requires an active fanout runtime",
            )
        })?;
        let parent_node_name = fanout.parent_node_name.clone();
        let parent_node_execution_id = fanout.parent_node_execution_id.clone();
        let parent_requires_approval = self
            .runtime
            .workflow
            .nodes
            .iter()
            .find(|node| node.name == parent_node_name)
            .map(workflow_transition::decide_completion_disposition)
            == Some(workflow_transition::CompletionDisposition::RequestApproval);
        if parent_requires_approval {
            // completion: approval — 全子完了後、human の承認まで parent は完了しない。
            // 親 artifact の集約と ArtifactProduced は承認時（apply_fanout_parent_approval）が担う。
            if self.mark_node_waiting_approval(&parent_node_execution_id, timestamp)
                != TransitionOutcome::Applied
            {
                return Err(crate::domain::workflow::WorkflowError::invalid_state(
                    format!(
                        "fanout parent NodeExecution '{parent_node_execution_id}' cannot wait for approval"
                    ),
                ));
            }
            events.push(WorkflowEvent::ApprovalRequested {
                execution_id: self.id.clone(),
                node_execution_id: parent_node_execution_id,
                node_name: parent_node_name,
                timestamp,
            });
            return Ok(AppliedNodeCompletionHandshake {
                advance: None,
                events,
            });
        }
        let parent_attempt = self
            .runtime
            .node_execution_counts
            .get(&parent_node_name)
            .copied()
            .unwrap_or(1);
        let children = fanout
            .children
            .iter()
            .map(|child| workflow_fanout::FanoutChildCompletionInput {
                node_name: child.node_name.clone(),
                session_id: (!child.session_id.is_empty()).then(|| child.session_id.clone()),
                result: child.result.clone(),
                artifact: child.artifact.clone().unwrap_or(serde_json::Value::Null),
                contract: child.contract.clone(),
                token_usage: child.token_usage.clone(),
                attempt: child.attempt,
                completed_at: child.completed_at.unwrap_or(timestamp),
                state: match child.state {
                    FanoutChildRuntimeState::Running => {
                        crate::domain::workflow::NODE_STATUS_RUNNING
                    }
                    FanoutChildRuntimeState::Completed => {
                        crate::domain::workflow::NODE_STATUS_COMPLETED
                    }
                    FanoutChildRuntimeState::Failed => crate::domain::workflow::NODE_STATUS_FAILED,
                    FanoutChildRuntimeState::Interrupted => {
                        crate::domain::workflow::NODE_STATUS_INTERRUPTED
                    }
                }
                .to_string(),
                failure_kind: child.failure_kind,
                failure_disposition: child.failure_disposition,
            })
            .collect::<Vec<_>>();
        let completion = workflow_fanout::plan_fanout_parent_completion(
            &parent_node_name,
            parent_attempt,
            &children,
            timestamp,
        );
        let parent_artifact = completion
            .parent_artifact
            .artifact
            .clone()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        events.insert(
            0,
            WorkflowEvent::ArtifactProduced {
                execution_id: self.id.clone(),
                node_execution_id: parent_node_execution_id.clone(),
                node_name: parent_node_name,
                contract: None,
                value: parent_artifact,
                request_id: None,
                submitted_at: None,
                timestamp,
            },
        );
        let _ = self.finalize_fanout_parent(
            &parent_node_execution_id,
            completion.parent_artifact,
            completion.history_entry,
            timestamp,
        );
        let advance = self.apply_advance_at(next_node_execution_id, timestamp)?;
        Ok(AppliedNodeCompletionHandshake {
            advance: Some(advance),
            events,
        })
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
            #[cfg(test)]
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
        let retry_child_index = self.runtime.fanout_runtime.as_ref().and_then(|fanout| {
            fanout.children.iter().position(|child| {
                child.state != FanoutChildRuntimeState::Running
                    && self.runtime.node_executions.iter().any(|execution| {
                        execution.id == child.node_execution_id
                            && execution.fanout_parent.as_ref() == Some(&fanout_parent)
                    })
            })
        });
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
                node_execution_id.clone(),
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
        let current_child = FanoutChildRuntime {
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
        };
        if let Some(index) = retry_child_index {
            fanout.children[index] = current_child;
        } else {
            fanout.children.push(current_child);
        }
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

    /// fanout の全 child が terminal（Completed / Failed）に達しているか。
    /// parent の既定完了条件であり、承認要求（divert）と承認適用は同じこの条件を使う。
    pub fn all_fanout_children_terminal(&self) -> bool {
        self.runtime.fanout_runtime.as_ref().is_some_and(|fanout| {
            fanout.children.iter().all(|child| {
                matches!(
                    child.state,
                    FanoutChildRuntimeState::Completed | FanoutChildRuntimeState::Failed
                )
            })
        })
    }

    /// 完了を保留したまま fanout child の成果物を記録する
    /// （`completion: approval` の child が承認待ちに入るときに使う）。
    pub fn record_fanout_child_output(
        &mut self,
        node_execution_id: &str,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(child) = self.runtime.fanout_runtime.as_mut().and_then(|fanout| {
            fanout
                .children
                .iter_mut()
                .find(|child| child.node_execution_id == node_execution_id)
        }) else {
            return TransitionOutcome::NotApplicable;
        };
        child.result = result;
        child.artifact = artifact;
        child.contract = contract;
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

    #[cfg(test)]
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
            RuntimeExecutionState::Running => ExecutionStateSet::Active,
            #[cfg(test)]
            RuntimeExecutionState::WaitingApproval => ExecutionStateSet::Active,
            #[cfg(test)]
            RuntimeExecutionState::Interrupted => ExecutionStateSet::Resumable,
            RuntimeExecutionState::Completed | RuntimeExecutionState::Aborted => {
                ExecutionStateSet::Finished
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.state_set() == ExecutionStateSet::Active
    }

    pub fn is_finished(&self) -> bool {
        self.state_set() == ExecutionStateSet::Finished
    }

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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
            ExecutionStateSet::Active => {
                self.state = RuntimeExecutionState::Aborted;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            #[cfg(test)]
            ExecutionStateSet::Resumable => {
                self.state = RuntimeExecutionState::Aborted;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            ExecutionStateSet::Finished => TransitionOutcome::NotApplicable,
        }
    }

    #[cfg(test)]
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

    #[cfg(test)]
    pub fn approve(&mut self) -> TransitionOutcome {
        match &self.state {
            RuntimeExecutionState::WaitingApproval => {
                self.state = RuntimeExecutionState::Running;
                TransitionOutcome::Applied
            }
            _ => TransitionOutcome::Rejected(TransitionRejection::NotWaitingApproval),
        }
    }

    #[cfg(test)]
    pub fn reject(&mut self) -> TransitionOutcome {
        self.approve()
    }

    #[cfg(test)]
    pub fn resolve_approval(&mut self) -> TransitionOutcome {
        self.reject()
    }

    pub fn admit_fanout_approval(&self) -> TransitionOutcome {
        self.active_only()
    }

    pub fn apply_turn_completion(&self, fact: CanonicalNodeFact) -> TurnCompletionDecision {
        let application = match self.state_set() {
            ExecutionStateSet::Active => TurnCompletionApplication::Live,
            #[cfg(test)]
            ExecutionStateSet::Resumable => TurnCompletionApplication::RecordOnly,
            ExecutionStateSet::Finished => TurnCompletionApplication::Superseded,
        };
        TurnCompletionDecision { application, fact }
    }

    pub fn start_fanout_child(&self) -> TransitionOutcome {
        self.active_only()
    }

    pub fn complete(&mut self) -> TransitionOutcome {
        match &self.state {
            RuntimeExecutionState::Running => {
                self.state = RuntimeExecutionState::Completed;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            #[cfg(test)]
            RuntimeExecutionState::WaitingApproval => {
                self.state = RuntimeExecutionState::Completed;
                self.interruption_reason = None;
                TransitionOutcome::Applied
            }
            RuntimeExecutionState::Completed => TransitionOutcome::AlreadyApplied,
            _ => TransitionOutcome::NotApplicable,
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

    pub fn replay_completed(&mut self) -> ReplayOutcome {
        transition_to_replay(self.complete())
    }

    pub fn replay_aborted(&mut self) -> ReplayOutcome {
        transition_to_replay(self.abort())
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

    fn active_only(&self) -> TransitionOutcome {
        match self.state_set() {
            ExecutionStateSet::Active => TransitionOutcome::Applied,
            #[cfg(test)]
            ExecutionStateSet::Resumable => {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            }
            ExecutionStateSet::Finished => {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            }
        }
    }

    #[cfg(test)]
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

    fn finished_states() -> [RuntimeExecutionState; 2] {
        [
            RuntimeExecutionState::Completed,
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
    fn operation_state_matrix_approval() {
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
    fn fanout_and_interrupt_cells_are_closed() {
        for state in active_states() {
            let expected = TransitionOutcome::Applied;
            assert_eq!(aggregate(state.clone()).start_fanout_child(), expected);
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
                entry: "implement".to_string(),
                ..Default::default()
            },
            lifecycle: WorkflowExecution::lifecycle_from_state(state),
            ..WorkflowExecutionRestore::default()
        })
    }

    #[test]
    fn node_submit_target_is_derived_from_node_execution_identity() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();

        let target = execution.admit_node_submit("node-execution-1").unwrap();
        assert_eq!(target.node_name, "implement");
        assert_eq!(target.attempt, 1);
        assert_eq!(
            execution.admit_node_submit("missing"),
            Err(NodeSubmitRejection::NodeExecutionNotFound)
        );
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
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();

        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::AwaitingSignal
        );
        assert_eq!(
            execution.attach_node_session(&node_execution_id, "session-1".to_string(), 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                11.5,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                11.75,
            ),
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
    fn provider_stop_admission_belongs_to_the_workflow_aggregate() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution.attach_node_session(&node_execution_id, "session-1".to_string(), 11.0);

        assert_eq!(
            execution.record_provider_stop(&node_execution_id, "session-2", 12.0),
            Err(ProviderStopRejection::SessionDoesNotOwnAttempt)
        );
        assert_eq!(
            execution.record_provider_stop(&node_execution_id, "session-1", 13.0),
            Ok(TransitionOutcome::Applied)
        );
        assert_eq!(
            execution.record_provider_stop(&node_execution_id, "session-1", 14.0),
            Ok(TransitionOutcome::AlreadyApplied)
        );
    }

    #[test]
    fn routing_failure_rejects_the_candidate_without_workflow_terminal_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.nodes[0].rules = vec![crate::domain::workflow::Rule::Next(
            "missing-node".to_string(),
        )];
        execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        let before = execution.clone();

        let result = execution.apply_advance_at("next-node".to_string(), 11.0);

        assert!(result.is_err());
        assert_eq!(execution, before);
    }

    #[test]
    fn agent_node_attempt_cannot_complete_before_submit_and_stop() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();

        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 11.0),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Running
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                12.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.node_executions()[0].completion_signals,
            NodeCompletionSignalState::StopReceived
        );
        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::AwaitingSignal
        );
        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 13.0),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                14.0,
            ),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                15.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.node_executions()[0].completion_signals,
            NodeCompletionSignalState::Ready
        );
        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::CompleteAuto
        );
        assert_eq!(
            execution.complete_node_execution(&node_execution_id, None, None, 16.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.decide_node_completion_handshake(&node_execution_id),
            NodeCompletionHandshakeDecision::AlreadySettled
        );
    }

    #[test]
    fn test_fanout親_completion承認はauto子の完了経路でも承認待ちになる() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        child: vec!["worker".to_string()],
                        items: None,
                    },
                ),
                completion: crate::domain::workflow::NodeCompletion::Approval,
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "worker".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.entry = "fanout".to_string();
        let parent_execution_id = execution
            .begin_node_attempt(
                "fanout".to_string(),
                NodeKindName::Fanout,
                1,
                None,
                "parent-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution
            .start_fanout_child_execution(
                "fanout".to_string(),
                parent_execution_id.clone(),
                "child-execution-1".to_string(),
                "worker".to_string(),
                NodeKindName::Session,
                1,
                FanoutParentRef {
                    parent_node: "fanout".to_string(),
                    parent_attempt: 1,
                    item_index: None,
                    child_index: 0,
                },
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "child-execution-1",
            NodeCompletionSignal::Submit,
            11.0,
        );
        execution.record_node_completion_signal(
            "child-execution-1",
            NodeCompletionSignal::Stop,
            12.0,
        );

        let result = execution
            .apply_node_completion_handshake(
                "child-execution-1",
                "node-execution-next".to_string(),
                13.0,
            )
            .unwrap();

        assert_eq!(result.advance, None, "承認まで次 node へ進まない");
        assert!(
            result.events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ApprovalRequested { node_execution_id, node_name, .. }
                    if node_execution_id == &parent_execution_id && node_name == "fanout"
            )),
            "親の ApprovalRequested が発行される: {:?}",
            result.events
        );
        let parent = execution
            .node_executions()
            .iter()
            .find(|node| node.id == parent_execution_id)
            .unwrap();
        assert_eq!(
            parent.status,
            RuntimeNodeExecutionStatus::WaitingApproval,
            "親は承認待ちで完了しない"
        );
        assert!(
            execution.fanout_runtime().is_some(),
            "承認時の artifact 集約のため fanout runtime は保持される"
        );
    }

    #[test]
    fn completion_handshake_applies_the_domain_transition_and_uses_the_supplied_next_id() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution
            .runtime
            .workflow
            .nodes
            .push(crate::domain::workflow::NodeDefinition {
                name: "verify".to_string(),
                kind: crate::domain::workflow::NodeKind::Command(
                    crate::domain::workflow::CommandSpec {
                        command: "true".to_string(),
                    },
                ),
                ..Default::default()
            });
        execution.runtime.workflow.nodes[0].rules =
            vec![crate::domain::workflow::Rule::Next("verify".to_string())];
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            &node_execution_id,
            NodeCompletionSignal::Submit,
            11.0,
        );
        execution.record_node_completion_signal(
            &node_execution_id,
            NodeCompletionSignal::Stop,
            12.0,
        );

        let result = execution
            .apply_node_completion_handshake(
                &node_execution_id,
                "node-execution-2".to_string(),
                13.0,
            )
            .unwrap();

        assert_eq!(
            result.advance,
            Some(ExecutionAdvanceDecision::TransitionAndStart)
        );
        assert_eq!(
            execution.node_executions().last().unwrap().id,
            "node-execution-2"
        );
    }

    #[test]
    fn pause_and_resume_preserve_partial_signal_on_the_same_attempt() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            &node_execution_id,
            NodeCompletionSignal::Submit,
            11.0,
        );

        assert_eq!(
            execution.pause_node_execution(&node_execution_id, 12.0),
            TransitionOutcome::Applied
        );
        let paused = &execution.node_executions()[0];
        assert_eq!(paused.status, RuntimeNodeExecutionStatus::Paused);
        assert_eq!(paused.attempt, 1);
        assert_eq!(
            paused.completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );

        assert_eq!(
            execution.resume_node_execution(&node_execution_id, 13.0),
            TransitionOutcome::Applied
        );
        let resumed = &execution.node_executions()[0];
        assert_eq!(resumed.status, RuntimeNodeExecutionStatus::Running);
        assert_eq!(resumed.attempt, 1);
        assert_eq!(
            resumed.completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
    }

    #[test]
    fn paused_attempt_accepts_artifact_replacement_without_changing_status() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                11.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.pause_node_execution(&node_execution_id, 12.0),
            TransitionOutcome::Applied
        );
        assert!(execution.admit_node_submit(&node_execution_id).is_ok());
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                13.0,
            ),
            TransitionOutcome::AlreadyApplied
        );
        assert_eq!(
            execution.apply_submitted_output(
                "implement".to_string(),
                &node_execution_id,
                1,
                None,
                "result".to_string(),
                serde_json::json!({"result": "replacement"}),
                None,
                14.0,
            ),
            TransitionOutcome::Applied
        );

        let paused = &execution.node_executions()[0];
        assert_eq!(paused.status, RuntimeNodeExecutionStatus::Paused);
        assert_eq!(
            paused.artifact.as_ref(),
            Some(&serde_json::json!({"result": "replacement"}))
        );
    }

    #[test]
    fn pause_does_not_layer_over_stop_received_or_waiting_approval() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let node_execution_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            &node_execution_id,
            NodeCompletionSignal::Stop,
            11.0,
        );
        assert_eq!(
            execution.pause_node_execution(&node_execution_id, 12.0),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Running
        );

        execution.runtime.node_executions[0].status = RuntimeNodeExecutionStatus::WaitingApproval;
        assert_eq!(
            execution.pause_node_execution(&node_execution_id, 13.0),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::WaitingApproval
        );
    }

    #[test]
    fn approval_target_requires_an_exact_attempt_when_fanout_names_are_ambiguous() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.nodes[0].completion =
            crate::domain::workflow::NodeCompletion::Approval;
        for (id, child_index) in [("child-1", 0), ("child-2", 1)] {
            execution
                .begin_node_attempt(
                    "implement".to_string(),
                    NodeKindName::Session,
                    1,
                    Some(FanoutParentRef {
                        parent_node: "fanout".to_string(),
                        parent_attempt: 1,
                        item_index: None,
                        child_index,
                    }),
                    id.to_string(),
                    10.0,
                )
                .unwrap();
            assert_eq!(
                execution.mark_node_waiting_approval(id, 11.0),
                TransitionOutcome::Applied
            );
        }

        assert!(matches!(
            execution.resolve_approval_attempt_target("implement", None),
            Err(crate::domain::workflow::WorkflowError::InvalidState(_))
        ));
        let target = execution
            .resolve_approval_attempt_target("implement", Some("child-2"))
            .unwrap();
        assert_eq!(target.node_execution_id, "child-2");
        assert_eq!(target.fanout_parent.unwrap().child_index, 1);
    }

    #[test]
    fn retry_current_node_isolates_partial_signal_attempt_and_preserves_its_history() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let previous_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        assert_eq!(
            execution.attach_node_session(&previous_id, "session-1".to_string(), 11.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &previous_id,
                NodeCompletionSignal::Submit,
                12.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.apply_submitted_output(
                "implement".to_string(),
                &previous_id,
                1,
                Some("session-1".to_string()),
                "result".to_string(),
                serde_json::json!({"attempt": 1}),
                Some("first".to_string()),
                13.0,
            ),
            TransitionOutcome::Applied
        );

        execution.retry_current_node_at("node-execution-2".to_string(), 20.0);

        assert_eq!(execution.node_executions().len(), 2);
        let previous = &execution.node_executions()[0];
        assert_eq!(previous.id, previous_id);
        assert_eq!(previous.status, RuntimeNodeExecutionStatus::Aborted);
        assert_eq!(
            previous.completion_signals,
            NodeCompletionSignalState::SubmitReceived
        );
        assert_eq!(previous.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            previous.artifact.as_ref(),
            Some(&serde_json::json!({"attempt": 1}))
        );

        let current = &execution.node_executions()[1];
        assert_eq!(current.id, "node-execution-2");
        assert_eq!(current.attempt, 2);
        assert_eq!(current.status, RuntimeNodeExecutionStatus::Running);
        assert_eq!(
            current.completion_signals,
            NodeCompletionSignalState::Pending
        );
        assert!(current.session_id.is_none());
        assert!(current.artifact.is_none());
        assert!(!execution.runtime.artifacts.contains_key("implement"));

        assert_eq!(
            execution
                .record_node_completion_signal(&previous_id, NodeCompletionSignal::Stop, 21.0,),
            TransitionOutcome::NotApplicable
        );
        assert_eq!(
            execution.node_executions()[1].completion_signals,
            NodeCompletionSignalState::Pending
        );
    }

    #[test]
    fn retry_current_node_accepts_only_failed_or_partial_signal_attempts() {
        let mut pending = restored_execution(RuntimeExecutionState::Running);
        pending
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "pending-attempt".to_string(),
                10.0,
            )
            .unwrap();
        pending.retry_current_node_at("pending-retry".to_string(), 11.0);
        assert_eq!(pending.node_executions().len(), 1);

        let mut waiting = restored_execution(RuntimeExecutionState::Running);
        let waiting_id = waiting
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "waiting-attempt".to_string(),
                10.0,
            )
            .unwrap();
        waiting.record_node_completion_signal(&waiting_id, NodeCompletionSignal::Stop, 11.0);
        waiting.retry_current_node_at("waiting-retry".to_string(), 12.0);
        assert_eq!(waiting.node_executions().len(), 2);

        let mut failed = restored_execution(RuntimeExecutionState::Running);
        let failed_id = failed
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                None,
                "failed-attempt".to_string(),
                10.0,
            )
            .unwrap();
        failed.fail_node_execution(
            &failed_id,
            "provider execution failed".to_string(),
            NodeExecutionFailureKind::InfrastructureCrash,
            11.0,
        );
        failed.retry_current_node_at("failed-retry".to_string(), 12.0);
        assert_eq!(failed.node_executions().len(), 2);
        assert_eq!(
            failed.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Failed
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
                "node-execution-1".to_string(),
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
                "node-execution-2".to_string(),
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
                "node-execution-1".to_string(),
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
                "child-execution-1".to_string(),
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
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Submit,
                11.25,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                &node_execution_id,
                NodeCompletionSignal::Stop,
                11.5,
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
    fn fanout_child_retry_replaces_only_the_current_logical_child_attempt() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        let parent_ref = FanoutParentRef {
            parent_node: "fanout".to_string(),
            parent_attempt: 1,
            item_index: Some(0),
            child_index: 0,
        };
        let old_id = execution
            .begin_node_attempt(
                "implement".to_string(),
                NodeKindName::Session,
                1,
                Some(parent_ref.clone()),
                "child-execution-1".to_string(),
                10.0,
            )
            .unwrap();
        execution.install_fanout(
            FanoutRuntimeState {
                parent_node_name: "fanout".to_string(),
                parent_node_execution_id: "parent-execution-1".to_string(),
                children: vec![FanoutChildRuntime {
                    node_execution_id: old_id.clone(),
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
        );
        execution.record_node_completion_signal(&old_id, NodeCompletionSignal::Stop, 12.0);
        assert_eq!(
            execution.request_node_retry(&old_id, 13.0),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.start_fanout_child_execution(
                "fanout".to_string(),
                "parent-execution-1".to_string(),
                "child-execution-2".to_string(),
                "implement".to_string(),
                NodeKindName::Session,
                2,
                parent_ref,
                14.0,
            ),
            Ok(TransitionOutcome::Applied)
        );

        assert_eq!(execution.node_executions().len(), 2);
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Aborted
        );
        assert_eq!(
            execution.node_executions()[0].completion_signals,
            NodeCompletionSignalState::StopReceived
        );
        assert_eq!(
            execution.node_executions()[1].completion_signals,
            NodeCompletionSignalState::Pending
        );
        let fanout = execution.fanout_runtime().unwrap();
        assert_eq!(fanout.children.len(), 1);
        assert_eq!(fanout.children[0].node_execution_id, "child-execution-2");
        assert_eq!(fanout.children[0].attempt, 2);
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Running);
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
                "parent-execution-1".to_string(),
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
