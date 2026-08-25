//! Workflow execution lifecycle aggregate.
//!
//! This aggregate is the single authority for execution lifecycle admission and
//! transitions. Runtime orchestration and event replay both call these methods;
//! neither is allowed to assign lifecycle state directly.
//!
//! 実行状態は実行木で表す: NodeExecution が親参照（`ExecutionParentRef`）で
//! 再帰木を成し、合成子（sequence / fanout）の実行インスタンスごとの進行
//! カーソル・子カウント・子 Artifact は `ScopeRuntime` が所有する。

pub mod scope;

use std::collections::HashMap;

use crate::domain::workflow::services::{
    reference as workflow_reference, routing as workflow_routing, transition as workflow_transition,
};
use crate::domain::workflow::value_objects::{
    ExecutionInterruptionReason, ExecutionOrigin, ExecutionParentRef, NodeCompletionSignal,
    NodeCompletionSignalState, NodeDefinition, NodeExecutionFailureKind, NodeHistoryEntry,
    NodeKindName, OnFailure, RuntimeArtifact, RuntimeExecutionState, TokenUsage,
    WorkflowDefinition,
};
use crate::domain::workflow::FailureDisposition;
use crate::domain::workflow::WorkflowEvent;

pub use scope::{FanoutScopeRuntime, ScopeRuntime, ScopeRuntimeKind, SequenceScopeRuntime};

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
    /// この slot へ供給された items 要素（retry 時の再束縛に使う）。
    pub item: Option<serde_json::Value>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewlyTerminalSession {
    pub node_execution_id: String,
    pub agent_session_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalAttemptTarget {
    pub node_execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    pub session_id: Option<String>,
    pub attempt: u32,
    pub parent: Option<ExecutionParentRef>,
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
    /// 完了前に記録された結果 summary（親スコープを持たない root leaf でも
    /// 保持されるよう node 自身が持つ）。
    pub result_summary: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub failure: Option<RuntimeNodeExecutionFailure>,
    pub parent: Option<ExecutionParentRef>,
    pub completion_signals: NodeCompletionSignalState,
    pub started_at: f64,
    pub completed_at: Option<f64>,
}

impl RuntimeNodeExecution {
    pub fn is_fanout_child(&self) -> bool {
        self.parent
            .as_ref()
            .is_some_and(ExecutionParentRef::is_fanout_child)
    }

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
    pub node_history: Vec<NodeHistoryEntry>,
    pub workflow_defaults: WorkflowDefaults,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub error_reason: Option<String>,
    pub started_at: f64,
    pub updated_at: f64,
    pub current_session_id: Option<String>,
    pub scopes: Vec<ScopeRuntime>,
    pub node_executions: Vec<RuntimeNodeExecution>,
    pub request: Option<String>,
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
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults,
            worktree_path: String::new(),
            created_from: ExecutionOrigin::DesktopUi,
            error_reason: None,
            started_at: 0.0,
            updated_at: 0.0,
            current_session_id: None,
            scopes: Vec::new(),
            node_executions: Vec::new(),
            request: None,
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
    /// 再起動対象の leaf 起動情報（束縛は再解決済み）。
    pub leaf: LeafStart,
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

/// 完了伝播の出力先。
///
/// live 経路は追記イベントと起動 leaf を収集し、fold（事実からの導出）は
/// 状態効果のみを適用する。導出では前進（次 leaf の起動）を行わない —
/// 実際に起きた起動は事実列自身が started として語る。
enum AdvanceEffects<'a> {
    Live {
        new_id: &'a mut dyn FnMut() -> String,
        events: &'a mut Vec<WorkflowEvent>,
        leaves: &'a mut Vec<LeafStart>,
    },
    Derive,
}

impl AdvanceEffects<'_> {
    fn emit(&mut self, event: WorkflowEvent) {
        if let Self::Live { events, .. } = self {
            events.push(event);
        }
    }
}

/// 起動すべき leaf（Session / Command）実行。束縛は起動時に確定した値。
#[derive(Debug, Clone, PartialEq)]
pub struct LeafStart {
    pub node_execution_id: String,
    pub node_name: String,
    pub kind: NodeKindName,
    /// 起動時に解決された入力パラメータ束縛（宣言順）。
    pub bindings: Vec<(String, serde_json::Value)>,
    /// fanout 展開の子なら、その items 要素。
    pub item: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionAdvanceDecision {
    /// 起動すべき runtime は無い（完了・承認待ち・失敗停止・並走子待ち）。
    Persist,
    /// 起動すべき leaf 群（sequence の前進は 1 つ、fanout の展開は複数）。
    StartLeaves(Vec<LeafStart>),
}

/// 前進の適用結果: 起動対象と、追記すべきイベント列（発生順）。
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedAdvance {
    pub decision: ExecutionAdvanceDecision,
    pub events: Vec<WorkflowEvent>,
}

/// 導出状態にあるが、まだ実行されていない前進（reconciliation の検出結果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAdvance {
    /// 完了済みの子からの前進（次の子の started が無い）。
    AfterChild {
        scope_id: String,
        child_name: String,
    },
    /// 合成子は開始済みだが実効 entry の子が未開始。
    StartEntry { scope_id: String },
    /// fanout の展開が途中で途切れている（欠けている展開座標がある）。
    ExpandFanout { scope_id: String },
}

/// children エントリの on_failure 処遇の適用結果: 追記すべきイベント列と
/// 起動すべき leaf（自動 retry の新 attempt / ignore 前進で始まる leaf）。
#[derive(Debug, Clone, PartialEq)]
pub struct FailureTreatmentOutcome {
    pub events: Vec<WorkflowEvent>,
    pub leaves: Vec<LeafStart>,
}

/// replay 中の NodeRetryRequested → 直後の NodeStarted の対応付け。
/// restart の start と fresh visit の start を判別し、visit_bases の更新可否を
/// 決める（live 経路では使わない）。
#[derive(Debug, Clone, PartialEq)]
struct PendingRestart {
    node_execution_id: String,
    node_name: String,
    parent_scope_id: Option<String>,
}

/// Aggregate that owns the workflow execution lifecycle state.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecution {
    state: RuntimeExecutionState,
    interruption_reason: Option<ExecutionInterruptionReason>,
    runtime: WorkflowExecutionView,
    pending_restart: Option<PendingRestart>,
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
    pub node_history: Vec<NodeHistoryEntry>,
    pub workflow_defaults: WorkflowDefaults,
    pub worktree_path: String,
    pub created_from: ExecutionOrigin,
    pub error_reason: Option<String>,
    pub started_at: f64,
    pub updated_at: f64,
    pub current_session_id: Option<String>,
    /// アクティブな合成子実行インスタンスのスコープ（開始順。先頭が root）。
    pub scopes: Vec<ScopeRuntime>,
    pub node_executions: Vec<RuntimeNodeExecution>,
    /// retry で開始された NodeExecution から直前の attempt への関係。
    /// 永続 snapshot ではなく、live transition または事実 replay から導出する。
    pub retry_predecessors: HashMap<String, String>,
    pub request: Option<String>,
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
            pending_restart: None,
            runtime: WorkflowExecutionView {
                id: restore.id,
                workflow: restore.workflow,
                node_history: restore.node_history,
                workflow_defaults: restore.workflow_defaults,
                worktree_path: restore.worktree_path,
                created_from: restore.created_from,
                error_reason: restore.error_reason,
                started_at: restore.started_at,
                updated_at: restore.updated_at,
                current_session_id: restore.current_session_id,
                scopes: restore.scopes,
                node_executions: restore.node_executions,
                retry_predecessors: HashMap::new(),
                request: restore.request,
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

    pub fn scope(&self, scope_id: &str) -> Option<&ScopeRuntime> {
        self.runtime
            .scopes
            .iter()
            .find(|scope| scope.node_execution_id == scope_id)
    }

    fn scope_mut(&mut self, scope_id: &str) -> Option<&mut ScopeRuntime> {
        self.runtime
            .scopes
            .iter_mut()
            .find(|scope| scope.node_execution_id == scope_id)
    }

    pub fn node_execution(&self, node_execution_id: &str) -> Option<&RuntimeNodeExecution> {
        self.runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
    }

    pub fn newly_terminal_sessions_since(
        &self,
        before: &WorkflowExecution,
    ) -> Vec<NewlyTerminalSession> {
        if self.id != before.id {
            return Vec::new();
        }
        before
            .runtime
            .node_executions
            .iter()
            .filter(|node| node.kind == NodeKindName::Session && node.status.is_active())
            .filter_map(|previous| {
                let current = self.node_execution(&previous.id)?;
                if current.status.is_active() {
                    return None;
                }
                Some(NewlyTerminalSession {
                    node_execution_id: current.id.clone(),
                    agent_session_id: current.session_id.clone()?,
                })
            })
            .collect()
    }

    /// node の親スコープ id（root インスタンスなら None）。
    fn parent_scope_id_of(&self, node_execution_id: &str) -> Option<String> {
        self.node_execution(node_execution_id)
            .and_then(|node| node.parent.as_ref())
            .map(|parent| parent.parent_id.clone())
    }

    pub fn transition_aborted(&mut self) -> TransitionOutcome {
        self.abort()
    }

    /// root node の実行を開始する（起動カスケード）。
    ///
    /// root が合成子なら、そのインスタンスと配下の実効 entry の子を leaf に
    /// 到達するまで再帰的に開始する。events には NodeStarted が開始順で並ぶ。
    pub fn start_root(
        &mut self,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> Result<AppliedAdvance, crate::domain::workflow::WorkflowError> {
        let root_name = self.runtime.workflow.entry.clone();
        let mut events = Vec::new();
        let mut leaves = Vec::new();
        self.start_node_instance(
            None,
            &root_name,
            new_id,
            timestamp,
            &mut events,
            &mut leaves,
        )?;
        self.runtime.updated_at = timestamp;
        Ok(AppliedAdvance {
            decision: if leaves.is_empty() {
                ExecutionAdvanceDecision::Persist
            } else {
                ExecutionAdvanceDecision::StartLeaves(leaves)
            },
            events,
        })
    }

    /// スコープ（または root）に node の新しい実行インスタンスを生やす。
    /// 合成子なら配下の開始まで再帰し、leaf なら起動対象として leaves へ積む。
    fn start_node_instance(
        &mut self,
        parent_scope_id: Option<&str>,
        node_name: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
        events: &mut Vec<WorkflowEvent>,
        leaves: &mut Vec<LeafStart>,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        let node = self
            .runtime
            .workflow
            .node_by_name(node_name)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "node '{node_name}' is undefined"
                ))
            })?;
        let attempt = match parent_scope_id {
            Some(scope_id) => {
                // sequence の子起動専用（fanout の子は start_fanout_child_instance）。
                let sequence = self
                    .scope_mut(scope_id)
                    .and_then(ScopeRuntime::sequence_mut)
                    .ok_or_else(|| {
                        crate::domain::workflow::WorkflowError::invalid_state(format!(
                            "sequence scope '{scope_id}' is not active"
                        ))
                    })?;
                sequence.current_child = Some(node_name.to_string());
                sequence.artifacts.remove(node_name);
                let attempt = sequence.record_child_start(node_name);
                // 辺の評価による突入 = 新しい visit。retry の予算基点を張り直す。
                sequence
                    .visit_bases
                    .insert(node_name.to_string(), attempt - 1);
                attempt
            }
            None => 1,
        };
        let id = new_id();
        let parent = parent_scope_id.map(ExecutionParentRef::sequence_child);
        self.push_started_node(
            &node,
            attempt,
            parent.clone(),
            id.clone(),
            timestamp,
            events,
        );
        match node.kind_name() {
            NodeKindName::Command | NodeKindName::Session => {
                let bindings = self.resolve_child_bindings(parent_scope_id, &node, None);
                leaves.push(LeafStart {
                    node_execution_id: id.clone(),
                    node_name: node.name.clone(),
                    kind: node.kind_name(),
                    bindings,
                    item: None,
                });
            }
            NodeKindName::Sequence => {
                let parameters = self.resolve_child_bindings(parent_scope_id, &node, None);
                self.runtime.scopes.push(ScopeRuntime {
                    node_execution_id: id.clone(),
                    node_name: node.name.clone(),
                    parent_scope_id: parent_scope_id.map(str::to_string),
                    parameters,
                    kind: ScopeRuntimeKind::Sequence(SequenceScopeRuntime::default()),
                });
                let entry_child = node
                    .sequence()
                    .and_then(|sequence| sequence.entry_child_name())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        crate::domain::workflow::WorkflowError::invalid_state(format!(
                            "sequence '{}' has no children",
                            node.name
                        ))
                    })?;
                self.start_node_instance(
                    Some(&id),
                    &entry_child,
                    new_id,
                    timestamp,
                    events,
                    leaves,
                )?;
            }
            NodeKindName::Fanout => {
                let parameters = self.resolve_child_bindings(parent_scope_id, &node, None);
                self.runtime.scopes.push(ScopeRuntime {
                    node_execution_id: id.clone(),
                    node_name: node.name.clone(),
                    parent_scope_id: parent_scope_id.map(str::to_string),
                    parameters,
                    kind: ScopeRuntimeKind::Fanout(FanoutScopeRuntime::default()),
                });
                self.expand_fanout_scope(&id, new_id, timestamp, events, leaves)?;
            }
        }
        Ok(id)
    }

    /// fanout スコープを items × children で展開し、各子を開始する。
    /// 子が空（空 items）なら fanout は即座に完了する。
    fn expand_fanout_scope(
        &mut self,
        scope_id: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
        events: &mut Vec<WorkflowEvent>,
        leaves: &mut Vec<LeafStart>,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let scope = self.scope(scope_id).ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(format!(
                "fanout scope '{scope_id}' is not active"
            ))
        })?;
        let node = self
            .runtime
            .workflow
            .node_by_name(&scope.node_name)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "fanout node '{}' is undefined",
                    scope.node_name
                ))
            })?;
        let spec = node.fanout().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(format!(
                "node '{}' is not a fanout",
                node.name
            ))
        })?;
        let items = self.resolve_fanout_items_in_scope(scope, spec)?;
        let spec = spec.clone();
        if let Some(fanout) = self.scope_mut(scope_id).and_then(ScopeRuntime::fanout_mut) {
            fanout.items = items.clone();
        }
        let coordinates: Vec<(String, Option<serde_json::Value>, Option<usize>, usize)> =
            match items {
                Some(items) => items
                    .into_iter()
                    .enumerate()
                    .flat_map(|(item_index, item)| {
                        spec.children
                            .iter()
                            .enumerate()
                            .map(move |(child_index, entry)| {
                                (
                                    entry.name.clone(),
                                    Some(item.clone()),
                                    Some(item_index),
                                    child_index,
                                )
                            })
                    })
                    .collect(),
                None => spec
                    .children
                    .iter()
                    .enumerate()
                    .map(|(child_index, entry)| (entry.name.clone(), None, None, child_index))
                    .collect(),
            };
        if coordinates.is_empty() {
            return self.complete_scope(
                scope_id,
                false,
                &mut AdvanceEffects::Live {
                    new_id,
                    events,
                    leaves,
                },
                timestamp,
            );
        }
        // 展開は冪等: 既に slot が生えている座標は飛ばす（reconciliation が
        // 途中で途切れた展開の続きだけを実行できるように）。
        let occupied: std::collections::HashSet<(Option<usize>, usize)> = self
            .runtime
            .node_executions
            .iter()
            .filter_map(|execution| {
                let parent = execution.parent.as_ref()?;
                if parent.parent_id != scope_id {
                    return None;
                }
                let slot = parent.fanout_slot?;
                Some((slot.item_index, slot.child_index))
            })
            .collect();
        for (child_name, item, item_index, child_index) in coordinates {
            if occupied.contains(&(item_index, child_index)) {
                continue;
            }
            self.start_fanout_child_instance(
                scope_id,
                &child_name,
                item,
                item_index,
                child_index,
                new_id,
                timestamp,
                events,
                leaves,
            )?;
        }
        Ok(())
    }

    /// fanout スコープに 1 slot を生やして子を開始する。
    #[allow(clippy::too_many_arguments)]
    fn start_fanout_child_instance(
        &mut self,
        scope_id: &str,
        child_name: &str,
        item: Option<serde_json::Value>,
        item_index: Option<usize>,
        child_index: usize,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
        events: &mut Vec<WorkflowEvent>,
        leaves: &mut Vec<LeafStart>,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        let node = self
            .runtime
            .workflow
            .node_by_name(child_name)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "fanout child node '{child_name}' is undefined"
                ))
            })?;
        let scope = self.scope_mut(scope_id).ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(format!(
                "fanout scope '{scope_id}' is not active"
            ))
        })?;
        // attempt は slot（lane）ごとに 1 始まり。再試行は slot 差し替え時に採番する。
        let attempt = 1;
        let id = new_id();
        let fanout = scope.fanout_mut().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(format!(
                "scope '{scope_id}' is not a fanout"
            ))
        })?;
        fanout.children.push(FanoutChildRuntime {
            node_execution_id: id.clone(),
            node_name: child_name.to_string(),
            session_id: String::new(),
            state: FanoutChildRuntimeState::Running,
            result: None,
            artifact: None,
            contract: node.artifact.clone(),
            failure_kind: None,
            failure_disposition: None,
            token_usage: TokenUsage::default(),
            attempt,
            completed_at: None,
            item: item.clone(),
        });
        let parent = Some(ExecutionParentRef::fanout_child(
            scope_id,
            item_index,
            child_index,
        ));
        self.push_started_node(&node, attempt, parent, id.clone(), timestamp, events);
        match node.kind_name() {
            NodeKindName::Command | NodeKindName::Session => {
                let bindings = self.resolve_child_bindings(Some(scope_id), &node, item.as_ref());
                leaves.push(LeafStart {
                    node_execution_id: id.clone(),
                    node_name: node.name.clone(),
                    kind: node.kind_name(),
                    bindings,
                    item,
                });
            }
            NodeKindName::Sequence => {
                let parameters = self.resolve_child_bindings(Some(scope_id), &node, item.as_ref());
                self.runtime.scopes.push(ScopeRuntime {
                    node_execution_id: id.clone(),
                    node_name: node.name.clone(),
                    parent_scope_id: Some(scope_id.to_string()),
                    parameters,
                    kind: ScopeRuntimeKind::Sequence(SequenceScopeRuntime::default()),
                });
                let entry_child = node
                    .sequence()
                    .and_then(|sequence| sequence.entry_child_name())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        crate::domain::workflow::WorkflowError::invalid_state(format!(
                            "sequence '{}' has no children",
                            node.name
                        ))
                    })?;
                self.start_node_instance(
                    Some(&id),
                    &entry_child,
                    new_id,
                    timestamp,
                    events,
                    leaves,
                )?;
            }
            NodeKindName::Fanout => {
                let parameters = self.resolve_child_bindings(Some(scope_id), &node, item.as_ref());
                self.runtime.scopes.push(ScopeRuntime {
                    node_execution_id: id.clone(),
                    node_name: node.name.clone(),
                    parent_scope_id: Some(scope_id.to_string()),
                    parameters,
                    kind: ScopeRuntimeKind::Fanout(FanoutScopeRuntime::default()),
                });
                self.expand_fanout_scope(&id, new_id, timestamp, events, leaves)?;
            }
        }
        Ok(id)
    }

    fn push_started_node(
        &mut self,
        node: &NodeDefinition,
        attempt: u32,
        parent: Option<ExecutionParentRef>,
        node_execution_id: String,
        timestamp: f64,
        events: &mut Vec<WorkflowEvent>,
    ) {
        self.runtime.node_executions.push(RuntimeNodeExecution {
            id: node_execution_id.clone(),
            execution_id: self.runtime.id.clone(),
            node_name: node.name.clone(),
            kind: node.kind_name(),
            attempt,
            status: RuntimeNodeExecutionStatus::Running,
            session_id: None,
            display_command: None,
            artifact: None,
            result_summary: None,
            token_usage: None,
            failure: None,
            parent: parent.clone(),
            completion_signals: NodeCompletionSignalState::Pending,
            started_at: timestamp,
            completed_at: None,
        });
        events.push(WorkflowEvent::NodeStarted {
            execution_id: self.runtime.id.clone(),
            node_execution_id,
            node_name: node.name.clone(),
            kind: node.kind_name(),
            attempt,
            parent,
            timestamp,
        });
        self.runtime.updated_at = timestamp;
    }

    /// 親スコープの解決空間から children エントリの inputs を束縛する。
    /// 供給元スコープの規則: sequence の子 = 兄弟 Artifact / 親パラメータ /
    /// `request`、fanout の子 = 親パラメータ / `request` / `items`。
    fn resolve_child_bindings(
        &self,
        parent_scope_id: Option<&str>,
        node: &NodeDefinition,
        item: Option<&serde_json::Value>,
    ) -> Vec<(String, serde_json::Value)> {
        let Some(scope_id) = parent_scope_id else {
            return Vec::new();
        };
        let Some(scope) = self.scope(scope_id) else {
            return Vec::new();
        };
        let Some(parent_node) = self.runtime.workflow.node_by_name(&scope.node_name) else {
            return Vec::new();
        };
        match &scope.kind {
            ScopeRuntimeKind::Sequence(_) => {
                let entry = parent_node
                    .sequence()
                    .and_then(|sequence| sequence.child_entry(&node.name));
                let values = self.scope_resolution_space(scope);
                workflow_reference::resolve_entry_bindings(entry, &values)
            }
            ScopeRuntimeKind::Fanout(_) => {
                let entry = parent_node.fanout().and_then(|fanout| {
                    fanout.children.iter().find(|child| child.name == node.name)
                });
                let parameters: HashMap<String, serde_json::Value> =
                    scope.parameters.iter().cloned().collect();
                let request =
                    serde_json::Value::String(self.runtime.request.clone().unwrap_or_default());
                workflow_reference::resolve_fanout_child_bindings(
                    entry,
                    node,
                    &parameters,
                    Some(&request),
                    item,
                )
            }
        }
    }

    /// スコープの供給元解決空間: `request` + 束縛済みパラメータ + 兄弟 Artifact。
    fn scope_resolution_space(&self, scope: &ScopeRuntime) -> HashMap<String, serde_json::Value> {
        let mut values = HashMap::new();
        values.insert(
            workflow_reference::REQUEST_ARTIFACT.to_string(),
            serde_json::Value::String(self.runtime.request.clone().unwrap_or_default()),
        );
        for (name, value) in &scope.parameters {
            values.insert(name.clone(), value.clone());
        }
        if let Some(sequence) = scope.sequence() {
            for (name, artifact) in &sequence.artifacts {
                if let Some(value) = &artifact.artifact {
                    values.insert(name.clone(), value.clone());
                }
            }
        }
        values
    }

    /// fanout の items をスコープ文脈で解決する。ArtifactField 参照の供給元は
    /// fanout の親 sequence スコープの兄弟 Artifact。
    fn resolve_fanout_items_in_scope(
        &self,
        fanout_scope: &ScopeRuntime,
        spec: &crate::domain::workflow::value_objects::FanoutSpec,
    ) -> Result<Option<Vec<serde_json::Value>>, crate::domain::workflow::WorkflowError> {
        use crate::domain::workflow::value_objects::ItemsSource;
        match &spec.items {
            None => Ok(None),
            Some(ItemsSource::Literal(items)) => Ok(Some(items.clone())),
            Some(ItemsSource::ArtifactField { node, field }) => {
                let value = fanout_scope
                    .parent_scope_id
                    .as_deref()
                    .and_then(|id| self.scope(id))
                    .and_then(ScopeRuntime::sequence)
                    .and_then(|sequence| sequence.artifacts.get(node))
                    .and_then(|artifact| artifact.artifact.as_ref())
                    .and_then(serde_json::Value::as_object)
                    .and_then(|artifact| artifact.get(field))
                    .ok_or_else(|| {
                        crate::domain::workflow::WorkflowError::invalid_state(format!(
                            "fanout items source '{node}.{field}' is unavailable"
                        ))
                    })?;
                let items = value.as_array().ok_or_else(|| {
                    crate::domain::workflow::WorkflowError::invalid_state(format!(
                        "fanout items source '{node}.{field}' is not an array"
                    ))
                })?;
                Ok(Some(items.clone()))
            }
        }
    }

    /// 子の完了を受けてスコープを前進させる（実行木の前進の中核）。
    fn advance_scope_after_child(
        &mut self,
        scope_id: &str,
        completed_child: &str,
        effects: &mut AdvanceEffects<'_>,
        timestamp: f64,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let Some(scope) = self.scope(scope_id) else {
            // スコープが既に確定している（例: 失敗停止後の遅延完了）。前進しない。
            return Ok(());
        };
        match &scope.kind {
            ScopeRuntimeKind::Sequence(sequence) => {
                let workflow = self.runtime.workflow.clone();
                let node = workflow.node_by_name(&scope.node_name).ok_or_else(|| {
                    crate::domain::workflow::WorkflowError::invalid_state(format!(
                        "sequence node '{}' is undefined",
                        scope.node_name
                    ))
                })?;
                let spec = node.sequence().ok_or_else(|| {
                    crate::domain::workflow::WorkflowError::invalid_state(format!(
                        "node '{}' is not a sequence",
                        scope.node_name
                    ))
                })?;
                let artifact = sequence
                    .artifacts
                    .get(completed_child)
                    .and_then(|output| output.artifact.clone());
                let counts = sequence.child_counts.clone();
                match workflow_routing::route_in_scope(
                    &workflow,
                    spec,
                    completed_child,
                    artifact.as_ref(),
                    &counts,
                )? {
                    workflow_routing::RouteDecision::TransitionTo(next) => match effects {
                        AdvanceEffects::Live {
                            new_id,
                            events,
                            leaves,
                        } => {
                            self.start_node_instance(
                                Some(scope_id),
                                &next,
                                &mut **new_id,
                                timestamp,
                                events,
                                leaves,
                            )?;
                        }
                        AdvanceEffects::Derive => {}
                    },
                    workflow_routing::RouteDecision::Completed => {
                        self.complete_scope(scope_id, false, effects, timestamp)?;
                    }
                }
            }
            ScopeRuntimeKind::Fanout(fanout) => {
                let all_terminal = fanout.children.iter().all(|child| {
                    matches!(
                        child.state,
                        FanoutChildRuntimeState::Completed | FanoutChildRuntimeState::Failed
                    )
                });
                if all_terminal {
                    self.complete_scope(scope_id, false, effects, timestamp)?;
                }
            }
        }
        Ok(())
    }

    /// 合成子インスタンスの既定完了条件成立後の処遇。
    ///
    /// `completion: approval` なら承認待ちで停止し（`approved = true` で承認後の
    /// 続きを実行）、それ以外は成果を確定して親スコープへ前進を伝播する。
    fn complete_scope(
        &mut self,
        scope_id: &str,
        approved: bool,
        effects: &mut AdvanceEffects<'_>,
        timestamp: f64,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let scope = self.scope(scope_id).cloned().ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(format!(
                "scope '{scope_id}' is not active"
            ))
        })?;
        let node = self
            .runtime
            .workflow
            .node_by_name(&scope.node_name)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "composite node '{}' is undefined",
                    scope.node_name
                ))
            })?;
        if node.requires_approval_completion() && !approved {
            if self.mark_node_waiting_approval(scope_id, timestamp) == TransitionOutcome::Applied {
                effects.emit(WorkflowEvent::ApprovalRequested {
                    execution_id: self.runtime.id.clone(),
                    node_execution_id: scope_id.to_string(),
                    node_name: scope.node_name.clone(),
                    timestamp,
                });
            }
            return Ok(());
        }

        // 成果の確定。
        let (artifact_value, contract, result, token_usage) = match &scope.kind {
            ScopeRuntimeKind::Sequence(sequence) => {
                let output_child = node.sequence().and_then(|spec| spec.output.as_deref());
                match (&node.artifact, output_child) {
                    (Some(_), Some(output_child)) => match sequence
                        .artifacts
                        .get(output_child)
                        .filter(|output| output.artifact.is_some())
                    {
                        Some(output) => (
                            output.artifact.clone(),
                            output.contract.clone(),
                            output.result.clone(),
                            None,
                        ),
                        None => {
                            // artifact 宣言のある部品 sequence が output 子の
                            // Artifact なしで終端へ到達した: 失敗として停止する。
                            let reason = format!(
                                "sequence '{}' reached its terminal without an artifact from output child '{output_child}'",
                                scope.node_name
                            );
                            let scope_attempt = self
                                .node_execution(scope_id)
                                .map(|execution| execution.attempt)
                                .unwrap_or(1);
                            let _ = self.fail_node_execution(
                                scope_id,
                                reason.clone(),
                                NodeExecutionFailureKind::ValidationFailure,
                                timestamp,
                            );
                            effects.emit(WorkflowEvent::NodeFailed {
                                execution_id: self.runtime.id.clone(),
                                node_execution_id: scope_id.to_string(),
                                node_name: scope.node_name.clone(),
                                attempt: scope_attempt,
                                reason,
                                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                                retry_count: None,
                                timestamp,
                            });
                            self.runtime
                                .scopes
                                .retain(|active| active.node_execution_id != scope_id);
                            self.record_child_failure_in_parent(&scope, timestamp);
                            // `on_failure: ignore` の合成子 child は失敗のまま親を
                            // 前進させる（retry は load 時に拒否済み）。
                            if let Some(parent_scope_id) = scope.parent_scope_id.clone() {
                                let ignored = self
                                    .node_execution(scope_id)
                                    .and_then(|target| self.on_failure_treatment_for(target))
                                    == Some(OnFailure::Ignore);
                                if ignored
                                    && !self
                                        .ignore_advance_blocked_by_halted_sibling(&parent_scope_id)
                                {
                                    self.advance_scope_after_child(
                                        &parent_scope_id,
                                        &scope.node_name,
                                        effects,
                                        timestamp,
                                    )?;
                                }
                            }
                            return Ok(());
                        }
                    },
                    _ => (None, None, None, None),
                }
            }
            ScopeRuntimeKind::Fanout(fanout) => {
                let mut combined = TokenUsage::default();
                for child in &fanout.children {
                    combined.add(&child.token_usage);
                }
                // `on_failure: ignore` 宣言 child の失敗 slot は結果配列から除く。
                // 宣言なしの失敗 slot は現行のまま null で残る。
                let aggregated = serde_json::Value::Array(
                    fanout
                        .children
                        .iter()
                        .filter(|child| !self.is_ignored_failed_fanout_slot(&node, child))
                        .map(|child| child.artifact.clone().unwrap_or(serde_json::Value::Null))
                        .collect(),
                );
                (
                    Some(aggregated),
                    None,
                    Some("complete".to_string()),
                    Some(combined),
                )
            }
        };

        let scope_attempt = self
            .node_execution(scope_id)
            .map(|execution| execution.attempt)
            .unwrap_or(1);
        if let Some(value) = &artifact_value {
            effects.emit(WorkflowEvent::ArtifactProduced {
                execution_id: self.runtime.id.clone(),
                node_execution_id: scope_id.to_string(),
                node_name: scope.node_name.clone(),
                contract: contract.clone(),
                value: value.clone(),
                request_id: None,
                submitted_at: None,
                timestamp,
            });
        }
        if self.node_execution(scope_id).is_some_and(|execution| {
            execution.status == RuntimeNodeExecutionStatus::WaitingApproval
        }) {
            let _ = self.mark_node_running(scope_id, timestamp);
        }
        let _ = self.complete_node_execution(
            scope_id,
            artifact_value.clone(),
            token_usage.clone(),
            timestamp,
        );
        effects.emit(WorkflowEvent::NodeCompleted {
            execution_id: self.runtime.id.clone(),
            node_execution_id: scope_id.to_string(),
            node_name: scope.node_name.clone(),
            attempt: scope_attempt,
            result_summary: result.clone(),
            token_usage: token_usage.clone(),
            timestamp,
        });
        self.runtime
            .scopes
            .retain(|active| active.node_execution_id != scope_id);

        match scope.parent_scope_id.as_deref() {
            None => {
                let _ = self.complete();
                effects.emit(WorkflowEvent::ExecutionCompleted {
                    execution_id: self.runtime.id.clone(),
                    total_token_usage:
                        crate::domain::workflow::services::projection::total_token_usage(
                            &self.runtime.node_history,
                        ),
                    timestamp,
                });
                self.runtime.updated_at = timestamp;
            }
            Some(parent_scope_id) => {
                self.record_child_result_in_scope(
                    parent_scope_id,
                    &scope.node_name,
                    scope_id,
                    RuntimeArtifact {
                        node_name: scope.node_name.clone(),
                        attempt: scope_attempt,
                        session_id: None,
                        result,
                        artifact: artifact_value,
                        contract,
                        token_usage,
                        completed_at: timestamp,
                    },
                    timestamp,
                );
                let parent_scope_id = parent_scope_id.to_string();
                self.advance_scope_after_child(
                    &parent_scope_id,
                    &scope.node_name,
                    effects,
                    timestamp,
                )?;
            }
        }
        Ok(())
    }

    /// 子（leaf または合成子インスタンス）の成果を親スコープへ記録する。
    fn record_child_result_in_scope(
        &mut self,
        scope_id: &str,
        child_name: &str,
        child_execution_id: &str,
        artifact: RuntimeArtifact,
        timestamp: f64,
    ) {
        let Some(scope) = self.scope_mut(scope_id) else {
            return;
        };
        match &mut scope.kind {
            ScopeRuntimeKind::Sequence(sequence) => {
                sequence.artifacts.insert(child_name.to_string(), artifact);
            }
            ScopeRuntimeKind::Fanout(fanout) => {
                if let Some(slot) = fanout
                    .children
                    .iter_mut()
                    .find(|slot| slot.node_execution_id == child_execution_id)
                {
                    let _ = slot.complete(
                        artifact.result,
                        artifact.artifact,
                        artifact.contract,
                        artifact.token_usage.unwrap_or_default(),
                        timestamp,
                    );
                }
            }
        }
        self.runtime.updated_at = timestamp;
    }

    /// 失敗した子を親 fanout の slot に反映する（sequence の親は前進しない）。
    fn record_child_failure_in_parent(&mut self, failed_scope: &ScopeRuntime, timestamp: f64) {
        let Some(parent_scope_id) = failed_scope.parent_scope_id.as_deref() else {
            return;
        };
        let child_execution_id = failed_scope.node_execution_id.clone();
        if let Some(fanout) = self
            .scope_mut(parent_scope_id)
            .and_then(ScopeRuntime::fanout_mut)
        {
            if let Some(slot) = fanout
                .children
                .iter_mut()
                .find(|slot| slot.node_execution_id == child_execution_id)
            {
                let _ = slot.fail(
                    NodeExecutionFailureKind::ValidationFailure,
                    FailureDisposition::Terminal,
                    timestamp,
                );
            }
        }
    }

    /// leaf の完了確定と、そこからの前進の伝播。
    fn apply_leaf_completion(
        &mut self,
        node_execution_id: &str,
        effects: &mut AdvanceEffects<'_>,
        timestamp: f64,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        let node = self
            .node_execution(node_execution_id)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "node execution '{node_execution_id}' disappeared"
                ))
            })?;
        let parent_scope_id = node.parent.as_ref().map(|parent| parent.parent_id.clone());
        // submit 済みの成果（スコープ / slot に記録済み）を完了値として読む。
        let (artifact, contract, result, token_usage) =
            self.pending_leaf_result(&node, parent_scope_id.as_deref());
        if self
            .node_execution(node_execution_id)
            .is_some_and(|execution| {
                execution.status == RuntimeNodeExecutionStatus::WaitingApproval
            })
        {
            let _ = self.mark_node_running(node_execution_id, timestamp);
        }
        if self.complete_node_execution(
            node_execution_id,
            artifact.clone(),
            token_usage.clone(),
            timestamp,
        ) != TransitionOutcome::Applied
        {
            return Err(crate::domain::workflow::WorkflowError::invalid_state(
                format!("node execution '{node_execution_id}' cannot complete"),
            ));
        }
        effects.emit(WorkflowEvent::NodeCompleted {
            execution_id: self.runtime.id.clone(),
            node_execution_id: node_execution_id.to_string(),
            node_name: node.node_name.clone(),
            attempt: node.attempt,
            result_summary: result.clone(),
            token_usage: token_usage.clone(),
            timestamp,
        });
        if !node.session_id.clone().unwrap_or_default().is_empty() {
            if let Some(session_id) = node.session_id.as_deref() {
                self.clear_stalls_for_session(session_id, timestamp);
            }
        }
        self.runtime.node_history.push(NodeHistoryEntry {
            node_name: node.node_name.clone(),
            completed_at: timestamp,
            result: result.clone(),
            session_id: node.session_id.clone(),
            token_usage: token_usage.clone(),
            artifact: artifact.clone(),
            attempt: node.attempt,
            fanout_children: None,
            state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
        });
        match parent_scope_id.as_deref() {
            None => {
                // 単独実行の root leaf: workflow 完了。
                let _ = self.complete();
                effects.emit(WorkflowEvent::ExecutionCompleted {
                    execution_id: self.runtime.id.clone(),
                    total_token_usage:
                        crate::domain::workflow::services::projection::total_token_usage(
                            &self.runtime.node_history,
                        ),
                    timestamp,
                });
                self.runtime.updated_at = timestamp;
            }
            Some(scope_id) => {
                self.record_child_result_in_scope(
                    scope_id,
                    &node.node_name,
                    node_execution_id,
                    RuntimeArtifact {
                        node_name: node.node_name.clone(),
                        attempt: node.attempt,
                        session_id: node.session_id.clone(),
                        result,
                        artifact,
                        contract,
                        token_usage,
                        completed_at: timestamp,
                    },
                    timestamp,
                );
                let scope_id = scope_id.to_string();
                self.advance_scope_after_child(&scope_id, &node.node_name, effects, timestamp)?;
            }
        }
        Ok(())
    }

    /// 完了前の leaf に紐づく成果（submit / command 出力で記録済み）を読む。
    fn pending_leaf_result(
        &self,
        node: &RuntimeNodeExecution,
        parent_scope_id: Option<&str>,
    ) -> (
        Option<serde_json::Value>,
        Option<String>,
        Option<String>,
        Option<TokenUsage>,
    ) {
        let scope = parent_scope_id.and_then(|id| self.scope(id));
        match scope.map(|scope| &scope.kind) {
            Some(ScopeRuntimeKind::Fanout(fanout)) => fanout
                .children
                .iter()
                .find(|slot| slot.node_execution_id == node.id)
                .map(|slot| {
                    (
                        slot.artifact.clone().or_else(|| node.artifact.clone()),
                        slot.contract.clone(),
                        slot.result.clone().or_else(|| node.result_summary.clone()),
                        (slot.token_usage != TokenUsage::default())
                            .then(|| slot.token_usage.clone())
                            .or_else(|| node.token_usage.clone()),
                    )
                })
                .unwrap_or((
                    node.artifact.clone(),
                    None,
                    node.result_summary.clone(),
                    node.token_usage.clone(),
                )),
            Some(ScopeRuntimeKind::Sequence(sequence)) => sequence
                .artifacts
                .get(&node.node_name)
                .filter(|output| output.attempt == node.attempt)
                .map(|output| {
                    (
                        output.artifact.clone().or_else(|| node.artifact.clone()),
                        output.contract.clone(),
                        output
                            .result
                            .clone()
                            .or_else(|| node.result_summary.clone()),
                        output
                            .token_usage
                            .clone()
                            .or_else(|| node.token_usage.clone()),
                    )
                })
                .unwrap_or((
                    node.artifact.clone(),
                    None,
                    node.result_summary.clone(),
                    node.token_usage.clone(),
                )),
            None => (
                node.artifact.clone(),
                None,
                node.result_summary.clone(),
                node.token_usage.clone(),
            ),
        }
    }

    /// leaf の入力束縛を現在のスコープ状態から解決し直す（再起動 / 復旧用）。
    pub fn leaf_start_for(
        &self,
        node_execution_id: &str,
    ) -> Result<LeafStart, crate::domain::workflow::WorkflowError> {
        let node_execution = self.node_execution(node_execution_id).ok_or_else(|| {
            crate::domain::workflow::WorkflowError::invalid_state(format!(
                "node execution '{node_execution_id}' was not found"
            ))
        })?;
        let node = self
            .runtime
            .workflow
            .node_by_name(&node_execution.node_name)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "node '{}' is undefined",
                    node_execution.node_name
                ))
            })?;
        let parent_scope_id = node_execution
            .parent
            .as_ref()
            .map(|parent| parent.parent_id.clone());
        let item = parent_scope_id
            .as_deref()
            .and_then(|scope_id| self.scope(scope_id))
            .and_then(ScopeRuntime::fanout)
            .and_then(|fanout| {
                fanout
                    .children
                    .iter()
                    .find(|slot| slot.node_execution_id == node_execution_id)
            })
            .and_then(|slot| slot.item.clone());
        let bindings =
            self.resolve_child_bindings(parent_scope_id.as_deref(), &node, item.as_ref());
        Ok(LeafStart {
            node_execution_id: node_execution_id.to_string(),
            node_name: node.name.clone(),
            kind: node.kind_name(),
            bindings,
            item,
        })
    }

    /// leaf attempt の再実行（retry / paused command の再開）。
    pub fn restart_node_attempt_at(
        &mut self,
        node_execution_id: &str,
        new_node_execution_id: String,
        timestamp: f64,
        mode: NodeRestartMode,
    ) -> Option<RestartedNodeAttempt> {
        let admission = match mode {
            NodeRestartMode::ExplicitRetry => RuntimeNodeExecution::can_retry,
            NodeRestartMode::CommandResume => RuntimeNodeExecution::can_restart_paused_command,
        };
        let target = self
            .node_execution(node_execution_id)
            .filter(|execution| !execution.kind.is_composite_kind())
            .cloned()?;
        if !admission(&target) {
            return None;
        }
        if self.request_node_restart_with(node_execution_id, timestamp, admission)
            != TransitionOutcome::Applied
        {
            return None;
        }
        self.state = RuntimeExecutionState::Running;
        self.interruption_reason = None;
        let parent_scope_id = target
            .parent
            .as_ref()
            .map(|parent| parent.parent_id.clone());
        let new_attempt = match parent_scope_id.as_deref() {
            Some(scope_id) => match self.scope_mut(scope_id)?.sequence_mut() {
                Some(sequence) => sequence.record_child_start(&target.node_name),
                // fanout: attempt は slot（lane）が所有する。
                None => target.attempt.saturating_add(1),
            },
            None => target.attempt.saturating_add(1),
        };
        let fanout_child = target.is_fanout_child();
        // fanout slot は新しい attempt の slot へ差し替える。
        if fanout_child {
            if let Some(fanout) = parent_scope_id
                .as_deref()
                .and_then(|scope_id| self.scope_mut(scope_id))
                .and_then(ScopeRuntime::fanout_mut)
            {
                if let Some(slot) = fanout
                    .children
                    .iter_mut()
                    .find(|slot| slot.node_execution_id == node_execution_id)
                {
                    slot.node_execution_id = new_node_execution_id.clone();
                    slot.session_id = String::new();
                    slot.state = FanoutChildRuntimeState::Running;
                    slot.result = None;
                    slot.artifact = None;
                    slot.failure_kind = None;
                    slot.failure_disposition = None;
                    slot.token_usage = TokenUsage::default();
                    slot.attempt = new_attempt;
                    slot.completed_at = None;
                }
            }
        }
        let node_def = self
            .runtime
            .workflow
            .node_by_name(&target.node_name)
            .cloned()?;
        let mut events = Vec::new();
        self.push_started_node(
            &node_def,
            new_attempt,
            target.parent.clone(),
            new_node_execution_id.clone(),
            timestamp,
            &mut events,
        );
        self.runtime
            .retry_predecessors
            .insert(new_node_execution_id.clone(), node_execution_id.to_string());
        let attempt = self.node_execution(&new_node_execution_id).cloned()?;
        let leaf = self.leaf_start_for(&new_node_execution_id).ok()?;
        Some(RestartedNodeAttempt {
            attempt,
            fanout_child,
            leaf,
        })
    }

    /// 承認対象の解決。WaitingApproval のアクティブな NodeExecution を、
    /// 実行木上の位置に関係なく承認対象にできる。
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
            .node_by_name(node_name)
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
        Ok(ApprovalAttemptTarget {
            node_execution_id: target.id.clone(),
            node_name: target.node_name.clone(),
            kind: target.kind,
            session_id: target.session_id.clone(),
            attempt: target.attempt,
            parent: target.parent.clone(),
            artifact: target.artifact.clone(),
        })
    }

    /// 承認の適用: 対象の完了を確定し、実行木を前進させる。
    /// ApprovalResolved イベント自体は（comment を持つため）呼び出し側が積む。
    pub fn apply_approval(
        &mut self,
        target_node_execution_id: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> Result<AppliedAdvance, crate::domain::workflow::WorkflowError> {
        let target = self
            .node_execution(target_node_execution_id)
            .cloned()
            .ok_or_else(|| {
                crate::domain::workflow::WorkflowError::invalid_state(format!(
                    "node execution '{target_node_execution_id}' disappeared"
                ))
            })?;
        if target.status != RuntimeNodeExecutionStatus::WaitingApproval {
            return Err(crate::domain::workflow::WorkflowError::invalid_state(
                format!("NodeExecution '{target_node_execution_id}' is not waiting for approval"),
            ));
        }
        let mut events = Vec::new();
        let mut leaves = Vec::new();
        match target.kind {
            NodeKindName::Command | NodeKindName::Session => {
                self.apply_leaf_completion(
                    target_node_execution_id,
                    &mut AdvanceEffects::Live {
                        new_id,
                        events: &mut events,
                        leaves: &mut leaves,
                    },
                    timestamp,
                )?;
            }
            NodeKindName::Sequence | NodeKindName::Fanout => {
                self.complete_scope(
                    target_node_execution_id,
                    true,
                    &mut AdvanceEffects::Live {
                        new_id,
                        events: &mut events,
                        leaves: &mut leaves,
                    },
                    timestamp,
                )?;
            }
        }
        self.runtime.updated_at = timestamp;
        Ok(AppliedAdvance {
            decision: if leaves.is_empty() {
                ExecutionAdvanceDecision::Persist
            } else {
                ExecutionAdvanceDecision::StartLeaves(leaves)
            },
            events,
        })
    }

    pub fn begin_node_attempt(
        &mut self,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        parent: Option<ExecutionParentRef>,
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
            result_summary: None,
            token_usage: None,
            failure: None,
            parent,
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
        let fanout_child = execution.is_fanout_child();
        let parent_scope_id = execution
            .parent
            .as_ref()
            .map(|parent| parent.parent_id.clone());
        if execution.session_id.as_deref() == Some(session_id.as_str())
            && (fanout_child
                || self.runtime.current_session_id.as_deref() == Some(session_id.as_str()))
        {
            return TransitionOutcome::AlreadyApplied;
        }
        if !execution.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        execution.session_id = Some(session_id.clone());
        if fanout_child {
            if let Some(slot) = parent_scope_id
                .as_deref()
                .and_then(|scope_id| self.scope_mut(scope_id))
                .and_then(ScopeRuntime::fanout_mut)
                .and_then(|fanout| {
                    fanout
                        .children
                        .iter_mut()
                        .find(|slot| slot.node_execution_id == node_execution_id)
                })
            {
                slot.session_id = session_id;
            }
        } else {
            self.runtime.current_session_id = Some(session_id);
        }
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

    /// 完了を保留したまま node の成果を記録する（submit / command 出力）。
    ///
    /// 記録先は node の親スコープ（sequence の兄弟空間 / fanout の slot）。
    /// 同じ attempt への既存記録とはフィールド単位でマージする（Artifact と
    /// 結果 summary / token usage が別の事実として届くため）。
    #[allow(clippy::too_many_arguments)]
    pub fn record_pending_result(
        &mut self,
        node_execution_id: &str,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
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
        if let Some(value) = &artifact {
            execution.artifact = Some(value.clone());
        }
        if let Some(value) = &result {
            execution.result_summary = Some(value.clone());
        }
        if let Some(usage) = &token_usage {
            execution.token_usage = Some(usage.clone());
        }
        let node_name = execution.node_name.clone();
        let attempt = execution.attempt;
        let session_id = execution.session_id.clone();
        let parent_scope_id = execution
            .parent
            .as_ref()
            .map(|parent| parent.parent_id.clone());
        let Some(scope_id) = parent_scope_id else {
            self.runtime.updated_at = timestamp;
            return TransitionOutcome::Applied;
        };
        let Some(scope) = self.scope_mut(&scope_id) else {
            return TransitionOutcome::NotApplicable;
        };
        match &mut scope.kind {
            ScopeRuntimeKind::Fanout(fanout) => {
                let Some(slot) = fanout
                    .children
                    .iter_mut()
                    .find(|slot| slot.node_execution_id == node_execution_id)
                else {
                    return TransitionOutcome::NotApplicable;
                };
                if result.is_some() {
                    slot.result = result;
                }
                if artifact.is_some() {
                    slot.artifact = artifact;
                }
                if contract.is_some() {
                    slot.contract = contract;
                }
                if let Some(usage) = token_usage {
                    slot.token_usage = usage;
                }
            }
            ScopeRuntimeKind::Sequence(sequence) => {
                let existing = sequence
                    .artifacts
                    .get(&node_name)
                    .filter(|entry| entry.attempt == attempt);
                let merged = RuntimeArtifact {
                    node_name: node_name.clone(),
                    attempt,
                    session_id,
                    result: result.or_else(|| existing.and_then(|entry| entry.result.clone())),
                    artifact: artifact
                        .or_else(|| existing.and_then(|entry| entry.artifact.clone())),
                    contract: contract
                        .or_else(|| existing.and_then(|entry| entry.contract.clone())),
                    token_usage: token_usage
                        .or_else(|| existing.and_then(|entry| entry.token_usage.clone())),
                    completed_at: timestamp,
                };
                sequence.artifacts.insert(node_name, merged);
            }
        }
        self.runtime.updated_at = timestamp;
        TransitionOutcome::Applied
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
            .iter()
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
        if execution.kind.is_composite_kind() {
            // 合成子インスタンスの成果（fanout 集約 / sequence output）は
            // NodeCompleted の replay で親へ渡る。ここでは自身に記録するだけ。
            if let Some(execution) = self
                .runtime
                .node_executions
                .iter_mut()
                .find(|execution| execution.id == node_execution_id)
            {
                execution.artifact = Some(value);
            }
            self.runtime.updated_at = timestamp;
            return TransitionOutcome::Applied;
        }
        self.record_pending_result(
            node_execution_id,
            None,
            Some(value),
            contract,
            None,
            timestamp,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_submitted_output(
        &mut self,
        _node_name: String,
        node_execution_id: &str,
        _attempt: u32,
        _session_id: Option<String>,
        contract: String,
        output: serde_json::Value,
        result: Option<String>,
        timestamp: f64,
    ) -> TransitionOutcome {
        let Some(execution) = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
        else {
            return TransitionOutcome::NotApplicable;
        };
        if !execution.status.is_active() {
            return TransitionOutcome::NotApplicable;
        }
        self.record_pending_result(
            node_execution_id,
            result,
            Some(output),
            Some(contract),
            None,
            timestamp,
        )
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
        // replay 用の restart 対応付け: 直後の NodeStarted を fresh visit ではなく
        // restart として適用させる（NodeRetryRequested と NodeStarted は同一
        // バッチで追記されるため、対応は常に直後の 1 件）。
        self.pending_restart =
            self.node_execution(node_execution_id)
                .map(|target| PendingRestart {
                    node_execution_id: target.id.clone(),
                    node_name: target.node_name.clone(),
                    parent_scope_id: target
                        .parent
                        .as_ref()
                        .map(|parent| parent.parent_id.clone()),
                });
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
        let target_node_name = target.node_name.clone();
        let target_attempt = target.attempt;
        let parent_scope_id = target
            .parent
            .as_ref()
            .map(|parent| parent.parent_id.clone());
        let target_is_fanout_child = target.is_fanout_child();
        if target_is_active {
            let _ = self.abort_node_execution(node_execution_id, timestamp);
            if target_is_fanout_child {
                if let Some(slot) = parent_scope_id
                    .as_deref()
                    .and_then(|scope_id| self.scope_mut(scope_id))
                    .and_then(ScopeRuntime::fanout_mut)
                    .and_then(|fanout| {
                        fanout
                            .children
                            .iter_mut()
                            .find(|slot| slot.node_execution_id == node_execution_id)
                    })
                {
                    let _ = slot.interrupt(timestamp);
                }
            }
        }
        // 同スコープに残る古い成果を消す（新しい attempt が上書きする前提を保つ）。
        if let Some(sequence) = parent_scope_id
            .as_deref()
            .and_then(|scope_id| self.scope_mut(scope_id))
            .and_then(ScopeRuntime::sequence_mut)
        {
            sequence.artifacts.remove(&target_node_name);
        }
        if !target_is_fanout_child {
            self.runtime.current_session_id = None;
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
        let execution = self
            .runtime
            .node_executions
            .iter()
            .find(|execution| execution.id == node_execution_id)
            .ok_or(NodeSubmitRejection::NodeExecutionNotFound)?;
        if !execution.status.is_active() {
            return Err(NodeSubmitRejection::AttemptNotCurrent);
        }
        // 実行木上でこの attempt が現行であること: 同じ node のより新しい
        // attempt が既に開始されていれば、この attempt は過去のもの。
        let superseded = self.runtime.node_executions.iter().any(|candidate| {
            candidate.node_name == execution.node_name
                && candidate.parent == execution.parent
                && candidate.attempt > execution.attempt
        });
        if superseded {
            return Err(NodeSubmitRejection::AttemptNotCurrent);
        }
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
        new_id: &mut dyn FnMut() -> String,
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
                    .node_execution(node_execution_id)
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
            NodeCompletionHandshakeDecision::CompleteAuto => {
                let mut events = Vec::new();
                let mut leaves = Vec::new();
                self.apply_leaf_completion(
                    node_execution_id,
                    &mut AdvanceEffects::Live {
                        new_id,
                        events: &mut events,
                        leaves: &mut leaves,
                    },
                    timestamp,
                )?;
                Ok(AppliedNodeCompletionHandshake {
                    advance: Some(if leaves.is_empty() {
                        ExecutionAdvanceDecision::Persist
                    } else {
                        ExecutionAdvanceDecision::StartLeaves(leaves)
                    }),
                    events,
                })
            }
        }
    }

    /// command など「二信号を持たない leaf」の完了確定と前進。
    /// 成果は事前に `record_pending_result` で記録しておく。
    pub fn complete_leaf_and_advance(
        &mut self,
        node_execution_id: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> Result<AppliedAdvance, crate::domain::workflow::WorkflowError> {
        let mut events = Vec::new();
        let mut leaves = Vec::new();
        self.apply_leaf_completion(
            node_execution_id,
            &mut AdvanceEffects::Live {
                new_id,
                events: &mut events,
                leaves: &mut leaves,
            },
            timestamp,
        )?;
        self.runtime.updated_at = timestamp;
        Ok(AppliedAdvance {
            decision: if leaves.is_empty() {
                ExecutionAdvanceDecision::Persist
            } else {
                ExecutionAdvanceDecision::StartLeaves(leaves)
            },
            events,
        })
    }

    /// leaf の失敗確定。fanout の子なら slot にも反映する（実行は前進しない:
    /// 失敗は直すべきもの、が既定）。
    pub fn fail_leaf_execution(
        &mut self,
        node_execution_id: &str,
        reason: String,
        kind: NodeExecutionFailureKind,
        disposition: FailureDisposition,
        timestamp: f64,
    ) -> TransitionOutcome {
        let outcome = self.fail_node_execution(node_execution_id, reason, kind, timestamp);
        if outcome != TransitionOutcome::Applied {
            return outcome;
        }
        // 合成子インスタンスの失敗はスコープも畳む。
        if self
            .node_execution(node_execution_id)
            .is_some_and(|execution| execution.kind.is_composite_kind())
        {
            self.runtime
                .scopes
                .retain(|scope| scope.node_execution_id != node_execution_id);
        }
        let parent_scope_id = self.parent_scope_id_of(node_execution_id);
        if let Some(slot) = parent_scope_id
            .as_deref()
            .and_then(|scope_id| self.scope_mut(scope_id))
            .and_then(ScopeRuntime::fanout_mut)
            .and_then(|fanout| {
                fanout
                    .children
                    .iter_mut()
                    .find(|slot| slot.node_execution_id == node_execution_id)
            })
        {
            let _ = slot.fail(kind, disposition, timestamp);
            slot.result = Some(kind.as_str().to_string());
            slot.artifact = None;
        }
        outcome
    }

    /// node の children エントリの on_failure 宣言（親スコープの定義から解決）。
    fn on_failure_treatment_for(&self, target: &RuntimeNodeExecution) -> Option<OnFailure> {
        let parent = target.parent.as_ref()?;
        let scope = self.scope(&parent.parent_id)?;
        let owner = self.runtime.workflow.node_by_name(&scope.node_name)?;
        match parent.fanout_slot {
            Some(slot) => owner
                .fanout()?
                .children
                .get(slot.child_index)
                .and_then(|entry| entry.on_failure),
            None => owner
                .sequence()?
                .child_entry(&target.node_name)
                .and_then(|entry| entry.on_failure),
        }
    }

    /// fanout 集約時に結果配列から除外する slot（`on_failure: ignore` 宣言 child の失敗）。
    fn is_ignored_failed_fanout_slot(
        &self,
        owner: &NodeDefinition,
        slot: &FanoutChildRuntime,
    ) -> bool {
        if slot.state != FanoutChildRuntimeState::Failed {
            return false;
        }
        let Some(spec) = owner.fanout() else {
            return false;
        };
        self.node_execution(&slot.node_execution_id)
            .and_then(|execution| execution.parent.as_ref())
            .and_then(|parent| parent.fanout_slot)
            .and_then(|coords| spec.children.get(coords.child_index))
            .and_then(|entry| entry.on_failure)
            == Some(OnFailure::Ignore)
    }

    /// ignore の前進を親 fanout に適用してよいか。`on_failure` 宣言なしで失敗した
    /// slot が残っている場合は既定（中断・Retry 待ち）を優先し、前進させない。
    fn ignore_advance_blocked_by_halted_sibling(&self, parent_scope_id: &str) -> bool {
        let Some(scope) = self.scope(parent_scope_id) else {
            return false;
        };
        let Some(fanout) = scope.fanout() else {
            return false;
        };
        let Some(owner) = self.runtime.workflow.node_by_name(&scope.node_name) else {
            return false;
        };
        fanout.children.iter().any(|slot| {
            slot.state == FanoutChildRuntimeState::Failed
                && !self.is_ignored_failed_fanout_slot(owner, slot)
        })
    }

    /// `retry: n` の予算判定: 失敗した attempt が同一 visit 内で n 番目以内なら
    /// 自動再実行できる。fanout の子は lane（slot）が visit（attempt は 1 始まり・
    /// 再訪なし）、sequence の子は visit_bases が基点（手動 Retry でも予算は
    /// 消化され、復活しない）。
    fn auto_retry_budget_left(&self, target: &RuntimeNodeExecution, max_retries: u32) -> bool {
        let attempts_in_visit = match target.parent.as_ref() {
            Some(parent) if parent.fanout_slot.is_none() => {
                let base = self
                    .scope(&parent.parent_id)
                    .and_then(ScopeRuntime::sequence)
                    .and_then(|sequence| sequence.visit_bases.get(&target.node_name).copied())
                    .unwrap_or(0);
                target.attempt.saturating_sub(base)
            }
            _ => target.attempt,
        };
        attempts_in_visit <= max_retries
    }

    /// 失敗確定済み node への on_failure 処遇の適用。
    ///
    /// - `retry: n`: 予算が残っていれば、手動の Node 単位 Retry と同じ attempt
    ///   機構・同じ記録形式（NodeRetryRequested + NodeStarted）で自動再実行する。
    /// - `ignore`: 失敗のまま親スコープを前進させる（sequence は artifact なしの
    ///   辺評価、fanout は全子決着判定へ）。
    /// - 宣言なし・予算切れ: None（現行既定 = 中断して resume / Retry 待ち）。
    pub fn apply_on_failure_treatment(
        &mut self,
        node_execution_id: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> Result<Option<FailureTreatmentOutcome>, crate::domain::workflow::WorkflowError> {
        let Some(target) = self.node_execution(node_execution_id).cloned() else {
            return Ok(None);
        };
        if target.status != RuntimeNodeExecutionStatus::Failed {
            return Ok(None);
        }
        let Some(treatment) = self.on_failure_treatment_for(&target) else {
            return Ok(None);
        };
        match treatment {
            OnFailure::Retry(max_retries) => {
                if !self.auto_retry_budget_left(&target, max_retries) {
                    return Ok(None);
                }
                let Some(restarted) = self.restart_node_attempt_at(
                    node_execution_id,
                    new_id(),
                    timestamp,
                    NodeRestartMode::ExplicitRetry,
                ) else {
                    return Ok(None);
                };
                let events = vec![
                    WorkflowEvent::NodeRetryRequested {
                        execution_id: self.runtime.id.clone(),
                        node_execution_id: node_execution_id.to_string(),
                        timestamp,
                    },
                    WorkflowEvent::NodeStarted {
                        execution_id: self.runtime.id.clone(),
                        node_execution_id: restarted.attempt.id.clone(),
                        node_name: restarted.attempt.node_name.clone(),
                        kind: restarted.attempt.kind,
                        attempt: restarted.attempt.attempt,
                        parent: restarted.attempt.parent.clone(),
                        timestamp,
                    },
                ];
                Ok(Some(FailureTreatmentOutcome {
                    events,
                    leaves: vec![restarted.leaf],
                }))
            }
            OnFailure::Ignore => {
                let Some(parent_scope_id) = target
                    .parent
                    .as_ref()
                    .map(|parent| parent.parent_id.clone())
                else {
                    return Ok(None);
                };
                if self.ignore_advance_blocked_by_halted_sibling(&parent_scope_id) {
                    return Ok(None);
                }
                let mut events = Vec::new();
                let mut leaves = Vec::new();
                self.advance_scope_after_child(
                    &parent_scope_id,
                    &target.node_name,
                    &mut AdvanceEffects::Live {
                        new_id,
                        events: &mut events,
                        leaves: &mut leaves,
                    },
                    timestamp,
                )?;
                Ok(Some(FailureTreatmentOutcome { events, leaves }))
            }
        }
    }

    /// fold: 完了二信号の充足から session leaf の決着を導出する。
    ///
    /// 完了規則（Submit + Stop 揃いで完了・`completion: approval` は human
    /// 承認まで完了しない）は live 経路と共有する
    /// `decide_node_completion_handshake` だけが知る。前進（次 leaf の起動）は
    /// 事実列自身が started として語るため行わない。
    pub fn derive_session_settlement(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> Result<(), String> {
        match self.decide_node_completion_handshake(node_execution_id) {
            NodeCompletionHandshakeDecision::AwaitingSignal
            | NodeCompletionHandshakeDecision::AlreadySettled
            | NodeCompletionHandshakeDecision::NotApplicable => Ok(()),
            NodeCompletionHandshakeDecision::RequestApproval => {
                let _ = self.mark_node_waiting_approval(node_execution_id, timestamp);
                Ok(())
            }
            NodeCompletionHandshakeDecision::CompleteAuto => self
                .apply_leaf_completion(node_execution_id, &mut AdvanceEffects::Derive, timestamp)
                .map_err(|error| error.to_string()),
        }
    }

    /// fold: leaf 完了の導出（command の exit 0 等）。決着済みへの遅延事実は
    /// 何もしない。
    pub fn derive_leaf_completed(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> Result<(), String> {
        if !self
            .node_execution(node_execution_id)
            .is_some_and(|node| node.status.is_active())
        {
            return Ok(());
        }
        self.apply_leaf_completion(node_execution_id, &mut AdvanceEffects::Derive, timestamp)
            .map_err(|error| error.to_string())
    }

    /// fold: leaf 失敗の導出。`on_failure: ignore` の親前進もここで導出する
    /// （retry は行動なので、後続の retry_requested / started 事実が語る）。
    pub fn derive_leaf_failed(
        &mut self,
        node_execution_id: &str,
        reason: String,
        kind: NodeExecutionFailureKind,
        timestamp: f64,
    ) -> Result<(), String> {
        let decision = self.apply_turn_completion(CanonicalNodeFact::Failed {
            reason: reason.clone(),
            kind,
        });
        if decision.application == TurnCompletionApplication::Superseded {
            return Ok(());
        }
        let _ = self.fail_leaf_execution(
            node_execution_id,
            reason,
            kind,
            FailureDisposition::Terminal,
            timestamp,
        );
        let Some(target) = self.node_execution(node_execution_id).cloned() else {
            return Ok(());
        };
        if self.on_failure_treatment_for(&target) == Some(OnFailure::Ignore) {
            if let Some(parent_scope_id) = target
                .parent
                .as_ref()
                .map(|parent| parent.parent_id.clone())
            {
                if !self.ignore_advance_blocked_by_halted_sibling(&parent_scope_id) {
                    self.advance_scope_after_child(
                        &parent_scope_id,
                        &target.node_name,
                        &mut AdvanceEffects::Derive,
                        timestamp,
                    )
                    .map_err(|error| error.to_string())?;
                }
            }
        }
        Ok(())
    }

    /// fold: human 承認による完了の導出。
    pub fn derive_approval_completion(
        &mut self,
        node_execution_id: &str,
        timestamp: f64,
    ) -> Result<(), String> {
        let Some(target) = self.node_execution(node_execution_id).cloned() else {
            return Ok(());
        };
        if target.status != RuntimeNodeExecutionStatus::WaitingApproval {
            return Ok(());
        }
        match target.kind {
            NodeKindName::Command | NodeKindName::Session => self
                .apply_leaf_completion(node_execution_id, &mut AdvanceEffects::Derive, timestamp)
                .map_err(|error| error.to_string()),
            NodeKindName::Sequence | NodeKindName::Fanout => self
                .complete_scope(
                    node_execution_id,
                    true,
                    &mut AdvanceEffects::Derive,
                    timestamp,
                )
                .map_err(|error| error.to_string()),
        }
    }

    /// reconciliation: 導出状態から「まだ実行していない前進」を検出する。
    ///
    /// 前進の実行と事実の追記の間でプロセスが落ちた場合、fold の導出は
    /// 「進むべきなのにアクティブな子が居ないスコープ」を残す。このメソッドは
    /// その差分を列挙し、engine の冪等 reconciliation ループが
    /// `apply_pending_advance` で実行する。
    pub fn derive_pending_advances(&self) -> Vec<PendingAdvance> {
        if !self.is_active() {
            return Vec::new();
        }
        let mut pending = Vec::new();
        for scope in &self.runtime.scopes {
            let scope_id = &scope.node_execution_id;
            let Some(scope_node) = self.node_execution(scope_id) else {
                continue;
            };
            // 承認待ちの合成子は human の行動待ち。決着済みスコープは対象外。
            if scope_node.status != RuntimeNodeExecutionStatus::Running {
                continue;
            }
            let has_active_child = self.runtime.node_executions.iter().any(|node| {
                node.parent
                    .as_ref()
                    .is_some_and(|parent| parent.parent_id == *scope_id)
                    && node.status.is_active()
            });
            if has_active_child {
                continue;
            }
            match &scope.kind {
                ScopeRuntimeKind::Sequence(sequence) => match &sequence.current_child {
                    None => pending.push(PendingAdvance::StartEntry {
                        scope_id: scope_id.clone(),
                    }),
                    Some(child_name) => {
                        let latest_settled = self
                            .runtime
                            .node_executions
                            .iter()
                            .filter(|node| {
                                node.node_name == *child_name
                                    && node
                                        .parent
                                        .as_ref()
                                        .is_some_and(|parent| parent.parent_id == *scope_id)
                            })
                            .max_by_key(|node| node.attempt)
                            .map(|node| node.status);
                        // 完了済みの子からの前進だけが「未実行の行動」。失敗・中断は
                        // 既定の停止（human の retry / resume 待ち）であり行動ではない。
                        if latest_settled == Some(RuntimeNodeExecutionStatus::Succeeded) {
                            pending.push(PendingAdvance::AfterChild {
                                scope_id: scope_id.clone(),
                                child_name: child_name.clone(),
                            });
                        }
                    }
                },
                ScopeRuntimeKind::Fanout(fanout) => {
                    // 展開途中（宣言された座標より slot が少ない）は展開の続き。
                    // 全 slot 決着で未完のケースは fold が畳んでいるため残らない
                    // （on_failure 既定の停止は除く）。
                    let Some(expected) = self
                        .runtime
                        .workflow
                        .node_by_name(&scope.node_name)
                        .and_then(|node| node.fanout())
                        .map(|spec| {
                            fanout
                                .items
                                .as_ref()
                                .map(|items| items.len() * spec.children.len())
                                .unwrap_or(spec.children.len())
                        })
                    else {
                        continue;
                    };
                    if fanout.children.len() < expected || fanout.children.is_empty() {
                        pending.push(PendingAdvance::ExpandFanout {
                            scope_id: scope_id.clone(),
                        });
                    }
                }
            }
        }
        pending
    }

    /// reconciliation: 検出された未実行の前進を live 経路と同じ機構で実行する。
    /// 返り値の events は追記すべき事実、leaves は起動すべき leaf。
    pub fn apply_pending_advance(
        &mut self,
        advance: &PendingAdvance,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> Result<AppliedAdvance, crate::domain::workflow::WorkflowError> {
        let mut events = Vec::new();
        let mut leaves = Vec::new();
        match advance {
            PendingAdvance::AfterChild {
                scope_id,
                child_name,
            } => {
                self.advance_scope_after_child(
                    scope_id,
                    child_name,
                    &mut AdvanceEffects::Live {
                        new_id,
                        events: &mut events,
                        leaves: &mut leaves,
                    },
                    timestamp,
                )?;
            }
            PendingAdvance::StartEntry { scope_id } => {
                let entry_child = self
                    .scope(scope_id)
                    .and_then(|scope| self.runtime.workflow.node_by_name(&scope.node_name))
                    .and_then(|node| node.sequence())
                    .and_then(|sequence| sequence.entry_child_name())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        crate::domain::workflow::WorkflowError::invalid_state(format!(
                            "sequence scope '{scope_id}' has no effective entry"
                        ))
                    })?;
                self.start_node_instance(
                    Some(scope_id),
                    &entry_child,
                    new_id,
                    timestamp,
                    &mut events,
                    &mut leaves,
                )?;
            }
            PendingAdvance::ExpandFanout { scope_id } => {
                self.expand_fanout_scope(scope_id, new_id, timestamp, &mut events, &mut leaves)?;
            }
        }
        self.runtime.updated_at = timestamp;
        Ok(AppliedAdvance {
            decision: if leaves.is_empty() {
                ExecutionAdvanceDecision::Persist
            } else {
                ExecutionAdvanceDecision::StartLeaves(leaves)
            },
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

    /// replay: NodeStarted の適用。インスタンスを生やし、スコープを再構築する。
    ///
    /// 合成子ならスコープを push（パラメータは開始時点のスコープ状態から
    /// 決定論的に再束縛）、子なら親スコープのカーソル・カウント・slot を進める。
    pub fn replay_node_started(
        &mut self,
        node_execution_id: &str,
        node_name: &str,
        kind: NodeKindName,
        attempt: u32,
        parent: Option<ExecutionParentRef>,
        timestamp: f64,
    ) -> Result<(), String> {
        let node = self
            .runtime
            .workflow
            .node_by_name(node_name)
            .cloned()
            .ok_or_else(|| format!("node '{node_name}' is undefined"))?;
        // 直前の NodeRetryRequested に対応する restart の start か。
        let retry_predecessor = self.pending_restart.take().and_then(|pending| {
            (pending.node_name == node_name
                && pending.parent_scope_id.as_deref()
                    == parent.as_ref().map(|parent| parent.parent_id.as_str()))
            .then_some(pending.node_execution_id)
        });
        let is_restart = retry_predecessor.is_some();
        // 親スコープの進行を再現する。
        if let Some(parent_ref) = &parent {
            let scope_id = parent_ref.parent_id.clone();
            let slot_item = parent_ref.fanout_slot.and_then(|slot| {
                slot.item_index.and_then(|index| {
                    self.scope(&scope_id)
                        .and_then(ScopeRuntime::fanout)
                        .and_then(|fanout| fanout.items.as_ref())
                        .and_then(|items| items.get(index).cloned())
                })
            });
            // retry の replay で旧 slot を差し替えるため、slot の展開座標を
            // node_executions の親参照から引けるようにしておく。
            let slot_coordinates: HashMap<String, crate::domain::workflow::FanoutSlot> = self
                .runtime
                .node_executions
                .iter()
                .filter_map(|execution| {
                    let parent = execution.parent.as_ref()?;
                    if parent.parent_id != scope_id {
                        return None;
                    }
                    Some((execution.id.clone(), parent.fanout_slot?))
                })
                .collect();
            let contract = node.artifact.clone();
            let scope = self
                .scope_mut(&scope_id)
                .ok_or_else(|| format!("parent scope '{scope_id}' is not active"))?;
            match (&mut scope.kind, parent_ref.fanout_slot) {
                (ScopeRuntimeKind::Sequence(sequence), _) => {
                    sequence.raise_child_count_to(node_name, attempt);
                    if !is_restart {
                        // fresh visit: retry 予算の基点を live 経路と同じ規則で張る。
                        sequence
                            .visit_bases
                            .insert(node_name.to_string(), attempt.saturating_sub(1));
                    }
                    sequence.current_child = Some(node_name.to_string());
                    sequence.artifacts.remove(node_name);
                }
                (ScopeRuntimeKind::Fanout(fanout), slot_ref) => {
                    // retry の replay では同じ展開座標の旧 slot を新しい attempt へ
                    // 差し替える。
                    let existing = slot_ref.and_then(|slot_ref| {
                        fanout.children.iter_mut().find(|slot| {
                            slot.node_execution_id != node_execution_id
                                && slot.state != FanoutChildRuntimeState::Completed
                                && slot_coordinates.get(&slot.node_execution_id) == Some(&slot_ref)
                        })
                    });
                    match existing {
                        Some(slot) => {
                            slot.node_execution_id = node_execution_id.to_string();
                            slot.session_id = String::new();
                            slot.state = FanoutChildRuntimeState::Running;
                            slot.result = None;
                            slot.artifact = None;
                            slot.failure_kind = None;
                            slot.failure_disposition = None;
                            slot.token_usage = TokenUsage::default();
                            slot.attempt = attempt;
                            slot.completed_at = None;
                        }
                        None => {
                            fanout.children.push(FanoutChildRuntime {
                                node_execution_id: node_execution_id.to_string(),
                                node_name: node_name.to_string(),
                                session_id: String::new(),
                                state: FanoutChildRuntimeState::Running,
                                result: None,
                                artifact: None,
                                contract,
                                failure_kind: None,
                                failure_disposition: None,
                                token_usage: TokenUsage::default(),
                                attempt,
                                completed_at: None,
                                item: slot_item,
                            });
                        }
                    }
                }
            }
        }
        self.begin_node_attempt(
            node_name.to_string(),
            kind,
            attempt,
            parent.clone(),
            node_execution_id.to_string(),
            timestamp,
        )
        .map_err(|reason| format!("node_started was rejected: {reason:?}"))?;
        if let Some(predecessor) = retry_predecessor {
            self.runtime
                .retry_predecessors
                .insert(node_execution_id.to_string(), predecessor);
        }
        // 合成子ならスコープを生やす。
        if kind.is_composite_kind() {
            let parent_scope_id = parent.as_ref().map(|parent| parent.parent_id.clone());
            let slot_item = parent
                .as_ref()
                .and_then(|parent| parent.fanout_slot)
                .and_then(|slot| slot.item_index)
                .and_then(|index| {
                    parent_scope_id
                        .as_deref()
                        .and_then(|scope_id| self.scope(scope_id))
                        .and_then(ScopeRuntime::fanout)
                        .and_then(|fanout| fanout.items.as_ref())
                        .and_then(|items| items.get(index).cloned())
                });
            let parameters =
                self.resolve_child_bindings(parent_scope_id.as_deref(), &node, slot_item.as_ref());
            let scope_kind = match kind {
                NodeKindName::Sequence => {
                    ScopeRuntimeKind::Sequence(SequenceScopeRuntime::default())
                }
                NodeKindName::Fanout => ScopeRuntimeKind::Fanout(FanoutScopeRuntime::default()),
                _ => unreachable!("composite kinds are sequence and fanout"),
            };
            self.runtime.scopes.push(ScopeRuntime {
                node_execution_id: node_execution_id.to_string(),
                node_name: node_name.to_string(),
                parent_scope_id,
                parameters,
                kind: scope_kind,
            });
            if kind == NodeKindName::Fanout {
                // items を開始時点のスコープ状態から再解決して保持する
                // （子 slot の item 復元に使う）。
                let items = {
                    let scope = self
                        .scope(node_execution_id)
                        .expect("the fanout scope was just pushed");
                    let spec = node
                        .fanout()
                        .ok_or_else(|| format!("node '{node_name}' is not a fanout"))?;
                    self.resolve_fanout_items_in_scope(scope, spec)
                        .map_err(|error| error.to_string())?
                };
                if let Some(fanout) = self
                    .scope_mut(node_execution_id)
                    .and_then(ScopeRuntime::fanout_mut)
                {
                    fanout.items = items;
                }
            }
        }
        self.runtime.updated_at = timestamp;
        Ok(())
    }

    /// 表示用の「現在の node」。承認待ちを優先し、無ければ最後にアクティブな
    /// leaf、それも無ければ最後にアクティブな node。
    pub fn display_current_node(&self) -> Option<String> {
        if let Some(waiting) = self
            .runtime
            .node_executions
            .iter()
            .rev()
            .find(|execution| execution.status == RuntimeNodeExecutionStatus::WaitingApproval)
        {
            return Some(waiting.node_name.clone());
        }
        self.runtime
            .node_executions
            .iter()
            .rev()
            .find(|execution| execution.status.is_active() && !execution.kind.is_composite_kind())
            .or_else(|| {
                self.runtime
                    .node_executions
                    .iter()
                    .rev()
                    .find(|execution| execution.status.is_active())
            })
            // アクティブが無い（失敗停止した Running など）場合も「現在の node」
            // は空にしない: 最後に開始された node（失敗した node）を指す。
            .or_else(|| self.runtime.node_executions.last())
            .map(|execution| execution.node_name.clone())
    }

    /// 全スコープの Artifact をフラットな node 名マップへ導出する
    /// （CLI の `output get` と表示のための互換 read。並走で同名が重なる場合は
    /// 後に開始されたスコープの値が勝つ）。
    pub fn flattened_artifacts(&self) -> HashMap<String, RuntimeArtifact> {
        let mut artifacts = HashMap::new();
        if let Some(request) = &self.runtime.request {
            artifacts.insert(
                workflow_reference::REQUEST_ARTIFACT.to_string(),
                RuntimeArtifact {
                    node_name: workflow_reference::REQUEST_ARTIFACT.to_string(),
                    attempt: 0,
                    session_id: None,
                    result: None,
                    artifact: Some(serde_json::Value::String(request.clone())),
                    contract: Some("string".to_string()),
                    token_usage: None,
                    completed_at: self.runtime.started_at,
                },
            );
        }
        for scope in &self.runtime.scopes {
            if let Some(sequence) = scope.sequence() {
                for (name, artifact) in &sequence.artifacts {
                    artifacts.insert(name.clone(), artifact.clone());
                }
            }
        }
        // 完了済み合成子・単独 leaf の成果も node_executions から補完する。
        for execution in &self.runtime.node_executions {
            if execution.status == RuntimeNodeExecutionStatus::Succeeded {
                if let Some(value) = &execution.artifact {
                    artifacts
                        .entry(execution.node_name.clone())
                        .or_insert_with(|| RuntimeArtifact {
                            node_name: execution.node_name.clone(),
                            attempt: execution.attempt,
                            session_id: execution.session_id.clone(),
                            result: None,
                            artifact: Some(value.clone()),
                            contract: None,
                            token_usage: execution.token_usage.clone(),
                            completed_at: execution.completed_at.unwrap_or(self.runtime.updated_at),
                        });
                }
            }
        }
        artifacts
    }

    pub fn record_history_entry(&mut self, entry: NodeHistoryEntry, timestamp: f64) {
        self.runtime.node_history.push(entry);
        self.runtime.updated_at = timestamp;
    }

    /// abort 時: アクティブな leaf 全件を "aborted" として node_history へ記録する。
    ///
    /// attempt はスコープごとに採番されるため、並走 lane の同名 node は同じ
    /// (node_name, attempt) を持ちうる。アクティブな leaf は定義上まだ history に
    /// 記録されていないので、重複判定は行わず全件を記録する。
    pub fn record_aborted_history_for_active_leaves(&mut self, timestamp: f64) {
        let active_leaves: Vec<_> = self
            .runtime
            .node_executions
            .iter()
            .filter(|node| node.status.is_active() && !node.kind.is_composite_kind())
            .map(|node| {
                (
                    node.node_name.clone(),
                    node.attempt,
                    node.session_id.clone(),
                    node.token_usage.clone().unwrap_or_default(),
                )
            })
            .collect();
        for (node_name, attempt, session_id, token_usage) in active_leaves {
            let entry = crate::domain::workflow::services::history::aborted_node_history_entry(
                node_name,
                attempt,
                session_id,
                token_usage,
                timestamp,
            );
            self.record_history_entry(entry, timestamp);
        }
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

    pub fn apply_turn_completion(&self, fact: CanonicalNodeFact) -> TurnCompletionDecision {
        let application = match self.state_set() {
            ExecutionStateSet::Active => TurnCompletionApplication::Live,
            #[cfg(test)]
            ExecutionStateSet::Resumable => TurnCompletionApplication::RecordOnly,
            ExecutionStateSet::Finished => TurnCompletionApplication::Superseded,
        };
        TurnCompletionDecision { application, fact }
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

    pub fn replay_aborted(&mut self) -> ReplayOutcome {
        transition_to_replay(self.abort())
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
    fn interrupt_cells_are_closed() {
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
    fn newly_terminal_sessions_activeから終端への初回遷移だけを導出する() {
        for active in [
            RuntimeNodeExecutionStatus::Running,
            RuntimeNodeExecutionStatus::Paused,
            RuntimeNodeExecutionStatus::WaitingApproval,
        ] {
            for terminal in [
                RuntimeNodeExecutionStatus::Succeeded,
                RuntimeNodeExecutionStatus::Failed,
                RuntimeNodeExecutionStatus::Aborted,
            ] {
                let mut before = restored_execution(RuntimeExecutionState::Running);
                before
                    .begin_node_attempt(
                        "implement".to_string(),
                        NodeKindName::Session,
                        1,
                        None,
                        "node-execution-1".to_string(),
                        10.0,
                    )
                    .unwrap();
                before.attach_node_session("node-execution-1", "agent-session-1".to_string(), 11.0);
                before.node_executions[0].status = active;
                let mut after = before.clone();
                after.node_executions[0].status = terminal;

                assert_eq!(
                    after.newly_terminal_sessions_since(&before),
                    vec![NewlyTerminalSession {
                        node_execution_id: "node-execution-1".to_string(),
                        agent_session_id: "agent-session-1".to_string(),
                    }],
                    "{active:?} -> {terminal:?}"
                );
            }
        }
    }

    #[test]
    fn newly_terminal_sessions_active維持と既終端と非sessionと参照なしを除外する() {
        let mut before = restored_execution(RuntimeExecutionState::Running);
        for (id, kind, session_id, status) in [
            (
                "running",
                NodeKindName::Session,
                Some("running-session"),
                RuntimeNodeExecutionStatus::Running,
            ),
            (
                "paused",
                NodeKindName::Session,
                Some("paused-session"),
                RuntimeNodeExecutionStatus::Paused,
            ),
            (
                "waiting-approval",
                NodeKindName::Session,
                Some("waiting-approval-session"),
                RuntimeNodeExecutionStatus::WaitingApproval,
            ),
            (
                "terminal",
                NodeKindName::Session,
                Some("terminal-session"),
                RuntimeNodeExecutionStatus::Succeeded,
            ),
            (
                "command",
                NodeKindName::Command,
                Some("command-session"),
                RuntimeNodeExecutionStatus::Running,
            ),
            (
                "unattached",
                NodeKindName::Session,
                None,
                RuntimeNodeExecutionStatus::Running,
            ),
        ] {
            before
                .begin_node_attempt("implement".to_string(), kind, 1, None, id.to_string(), 10.0)
                .unwrap();
            if let Some(session_id) = session_id {
                before.attach_node_session(id, session_id.to_string(), 11.0);
            }
            before
                .node_executions
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap()
                .status = status;
        }
        let mut after = before.clone();
        for id in ["terminal", "command", "unattached"] {
            after
                .node_executions
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap()
                .status = RuntimeNodeExecutionStatus::Aborted;
        }

        assert!(after.newly_terminal_sessions_since(&before).is_empty());
        after.id = "different-execution".to_string();
        assert!(after.newly_terminal_sessions_since(&before).is_empty());
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
    fn routing_failure_surfaces_an_error_without_workflow_terminal_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution
            .runtime
            .workflow
            .nodes
            .push(crate::domain::workflow::NodeDefinition {
                name: "main".to_string(),
                kind: crate::domain::workflow::NodeKind::Sequence(
                    crate::domain::workflow::SequenceSpec {
                        entry: None,
                        output: None,
                        children: vec![crate::domain::workflow::ChildEntry {
                            on_failure: None,
                            name: "implement".to_string(),
                            inputs: Vec::new(),
                            rules: Some(vec![crate::domain::workflow::Rule::Next(
                                "missing-node".to_string(),
                            )]),
                        }],
                    },
                ),
                ..Default::default()
            });
        execution.runtime.workflow.entry = "main".to_string();
        execution
            .replay_node_started("main-1", "main", NodeKindName::Sequence, 1, None, 9.0)
            .unwrap();
        execution
            .replay_node_started(
                "node-execution-1",
                "implement",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::sequence_child("main-1")),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Submit,
            10.5,
        );
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Stop,
            10.75,
        );

        let mut new_id = || "next-node".to_string();
        let result =
            execution.apply_node_completion_handshake("node-execution-1", &mut new_id, 11.0);

        assert!(result.is_err());
        assert_ne!(execution.state(), &RuntimeExecutionState::Completed);
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
                        children: vec![crate::domain::workflow::ChildEntry::reference("worker")],
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
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        execution
            .replay_node_started(
                "child-execution-1",
                "worker",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::fanout_child(
                    "parent-execution-1",
                    None,
                    0,
                )),
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

        let mut new_id = || "node-execution-next".to_string();
        let result = execution
            .apply_node_completion_handshake("child-execution-1", &mut new_id, 13.0)
            .unwrap();

        assert_eq!(
            result.advance,
            Some(ExecutionAdvanceDecision::Persist),
            "承認まで次 node へ進まない"
        );
        assert!(
            result.events.iter().any(|event| matches!(
                event,
                WorkflowEvent::ApprovalRequested { node_execution_id, node_name, .. }
                    if node_execution_id == "parent-execution-1" && node_name == "fanout"
            )),
            "親の ApprovalRequested が発行される: {:?}",
            result.events
        );
        let parent = execution
            .node_executions()
            .iter()
            .find(|node| node.id == "parent-execution-1")
            .unwrap();
        assert_eq!(
            parent.status,
            RuntimeNodeExecutionStatus::WaitingApproval,
            "親は承認待ちで完了しない"
        );
        assert!(
            execution.scope("parent-execution-1").is_some(),
            "承認時の artifact 集約のため fanout スコープは保持される"
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
        execution
            .runtime
            .workflow
            .nodes
            .push(crate::domain::workflow::NodeDefinition {
                name: "main".to_string(),
                kind: crate::domain::workflow::NodeKind::Sequence(
                    crate::domain::workflow::SequenceSpec {
                        entry: None,
                        output: None,
                        children: vec![
                            crate::domain::workflow::ChildEntry::reference("implement"),
                            crate::domain::workflow::ChildEntry::reference("verify"),
                        ],
                    },
                ),
                ..Default::default()
            });
        execution.runtime.workflow.entry = "main".to_string();
        execution
            .replay_node_started("main-1", "main", NodeKindName::Sequence, 1, None, 9.0)
            .unwrap();
        execution
            .replay_node_started(
                "node-execution-1",
                "implement",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::sequence_child("main-1")),
                10.0,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Submit,
            11.0,
        );
        execution.record_node_completion_signal(
            "node-execution-1",
            NodeCompletionSignal::Stop,
            12.0,
        );

        let mut new_id = || "node-execution-2".to_string();
        let result = execution
            .apply_node_completion_handshake("node-execution-1", &mut new_id, 13.0)
            .unwrap();

        assert_eq!(
            result.advance,
            Some(ExecutionAdvanceDecision::StartLeaves(vec![LeafStart {
                node_execution_id: "node-execution-2".to_string(),
                node_name: "verify".to_string(),
                kind: NodeKindName::Command,
                bindings: Vec::new(),
                item: None,
            }]))
        );
        assert_eq!(
            execution.node_executions().last().unwrap().id,
            "node-execution-2"
        );
        assert!(result.events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeStarted { node_execution_id, node_name, .. }
                if node_execution_id == "node-execution-2" && node_name == "verify"
        )));
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
                    Some(ExecutionParentRef::fanout_child(
                        "parent-execution-1",
                        None,
                        child_index,
                    )),
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
        assert_eq!(target.parent.unwrap().fanout_slot.unwrap().child_index, 1);
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

        execution.restart_node_attempt_at(
            &previous_id,
            "node-execution-2".to_string(),
            20.0,
            NodeRestartMode::ExplicitRetry,
        );

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
            execution
                .runtime
                .retry_predecessors
                .get("node-execution-2")
                .map(String::as_str),
            Some(previous_id.as_str())
        );
        assert_eq!(
            current.completion_signals,
            NodeCompletionSignalState::Pending
        );
        assert!(current.session_id.is_none());
        assert!(current.artifact.is_none());

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
        assert!(pending
            .restart_node_attempt_at(
                "pending-attempt",
                "pending-retry".to_string(),
                11.0,
                NodeRestartMode::ExplicitRetry,
            )
            .is_none());
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
        waiting
            .restart_node_attempt_at(
                &waiting_id,
                "waiting-retry".to_string(),
                12.0,
                NodeRestartMode::ExplicitRetry,
            )
            .unwrap();
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
        failed
            .restart_node_attempt_at(
                &failed_id,
                "failed-retry".to_string(),
                12.0,
                NodeRestartMode::ExplicitRetry,
            )
            .unwrap();
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
            execution.interrupt(ExecutionInterruptionReason::Crash),
            TransitionOutcome::Applied
        );

        execution
            .derive_leaf_failed(
                &node_execution_id,
                "exit 1".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                12.0,
            )
            .unwrap();

        assert_eq!(execution.state(), &RuntimeExecutionState::Interrupted);
        assert_eq!(
            execution.node_executions()[0].status,
            RuntimeNodeExecutionStatus::Failed
        );
    }

    #[test]
    fn fanout_child_completion_updates_slot_and_node_as_one_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        children: vec![
                            crate::domain::workflow::ChildEntry::reference("implement"),
                            crate::domain::workflow::ChildEntry::reference("verify"),
                        ],
                        items: None,
                    },
                ),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "implement".to_string(),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "verify".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.entry = "fanout".to_string();
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        for (id, name, child_index) in [("child-1", "implement", 0), ("child-2", "verify", 1)] {
            execution
                .replay_node_started(
                    id,
                    name,
                    NodeKindName::Session,
                    1,
                    Some(ExecutionParentRef::fanout_child(
                        "parent-execution-1",
                        None,
                        child_index,
                    )),
                    10.5,
                )
                .unwrap();
        }
        execution.record_node_completion_signal("child-1", NodeCompletionSignal::Submit, 11.25);
        execution.record_node_completion_signal("child-1", NodeCompletionSignal::Stop, 11.5);
        execution.record_pending_result(
            "child-1",
            Some("done".to_string()),
            Some(serde_json::json!({"ok": true})),
            Some("result".to_string()),
            None,
            11.9,
        );

        let mut new_id = || "unused".to_string();
        let applied = execution
            .complete_leaf_and_advance("child-1", &mut new_id, 12.0)
            .unwrap();

        assert_eq!(applied.decision, ExecutionAdvanceDecision::Persist);
        let fanout = execution
            .scope("parent-execution-1")
            .unwrap()
            .fanout()
            .unwrap();
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Completed);
        assert_eq!(
            fanout.children[0].artifact,
            Some(serde_json::json!({"ok": true}))
        );
        assert_eq!(fanout.children[1].state, FanoutChildRuntimeState::Running);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == "child-1")
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::Succeeded
        );
        assert_eq!(execution.state(), &RuntimeExecutionState::Running);
    }

    #[test]
    fn fanout_child_retry_replaces_only_the_current_logical_child_attempt() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        children: vec![crate::domain::workflow::ChildEntry::reference("implement")],
                        items: None,
                    },
                ),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "implement".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.entry = "fanout".to_string();
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        execution
            .replay_node_started(
                "child-execution-1",
                "implement",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::fanout_child(
                    "parent-execution-1",
                    Some(0),
                    0,
                )),
                10.5,
            )
            .unwrap();
        execution.record_node_completion_signal(
            "child-execution-1",
            NodeCompletionSignal::Stop,
            12.0,
        );

        let restarted = execution
            .restart_node_attempt_at(
                "child-execution-1",
                "child-execution-2".to_string(),
                13.0,
                NodeRestartMode::ExplicitRetry,
            )
            .unwrap();

        assert!(restarted.fanout_child);
        assert_eq!(execution.node_executions().len(), 3);
        let old = execution
            .node_executions()
            .iter()
            .find(|node| node.id == "child-execution-1")
            .unwrap();
        assert_eq!(old.status, RuntimeNodeExecutionStatus::Aborted);
        assert_eq!(
            old.completion_signals,
            NodeCompletionSignalState::StopReceived
        );
        let current = execution
            .node_executions()
            .iter()
            .find(|node| node.id == "child-execution-2")
            .unwrap();
        assert_eq!(current.attempt, 2);
        assert_eq!(
            current.completion_signals,
            NodeCompletionSignalState::Pending
        );
        let fanout = execution
            .scope("parent-execution-1")
            .unwrap()
            .fanout()
            .unwrap();
        assert_eq!(fanout.children.len(), 1);
        assert_eq!(fanout.children[0].node_execution_id, "child-execution-2");
        assert_eq!(fanout.children[0].attempt, 2);
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Running);
    }

    #[test]
    fn fanout_child_failure_updates_slot_and_node_as_one_transition() {
        let mut execution = restored_execution(RuntimeExecutionState::Running);
        execution.runtime.workflow.nodes = vec![
            crate::domain::workflow::NodeDefinition {
                name: "fanout".to_string(),
                kind: crate::domain::workflow::NodeKind::Fanout(
                    crate::domain::workflow::FanoutSpec {
                        children: vec![crate::domain::workflow::ChildEntry::reference("implement")],
                        items: None,
                    },
                ),
                ..Default::default()
            },
            crate::domain::workflow::NodeDefinition {
                name: "implement".to_string(),
                ..Default::default()
            },
        ];
        execution.runtime.workflow.entry = "fanout".to_string();
        execution
            .replay_node_started(
                "parent-execution-1",
                "fanout",
                NodeKindName::Fanout,
                1,
                None,
                10.0,
            )
            .unwrap();
        for (index, child_id) in ["child-1", "child-2"].into_iter().enumerate() {
            execution
                .replay_node_started(
                    child_id,
                    "implement",
                    NodeKindName::Session,
                    (index + 1) as u32,
                    Some(ExecutionParentRef::fanout_child(
                        "parent-execution-1",
                        Some(index),
                        0,
                    )),
                    11.0 + index as f64,
                )
                .unwrap();
        }
        assert_eq!(
            execution.fail_leaf_execution(
                "child-1",
                "child failed".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
                13.0,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.fail_leaf_execution(
                "child-1",
                "child failed".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
                14.0,
            ),
            TransitionOutcome::AlreadyApplied
        );
        let fanout = execution
            .scope("parent-execution-1")
            .unwrap()
            .fanout()
            .unwrap();
        assert_eq!(fanout.children[0].state, FanoutChildRuntimeState::Failed);
        assert_eq!(fanout.children[1].state, FanoutChildRuntimeState::Running);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == "child-1")
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::Failed
        );
    }

    // --- 実行木（#1463）: 合成子の再帰実行 -----------------------------------

    use crate::domain::workflow::{
        ChildEntry, CommandSpec, FanoutSpec, NodeCompletion, NodeKind, Rule, SequenceSpec,
    };

    fn tree_command_node(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: format!("printf {name}"),
            }),
            ..Default::default()
        }
    }

    fn tree_sequence_node(
        name: &str,
        output: Option<&str>,
        children: Vec<ChildEntry>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Sequence(SequenceSpec {
                entry: None,
                output: output.map(str::to_string),
                children,
            }),
            ..Default::default()
        }
    }

    fn tree_execution(nodes: Vec<NodeDefinition>) -> WorkflowExecution {
        WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "tree".to_string(),
                entry: "main".to_string(),
                nodes,
                ..Default::default()
            },
            ..WorkflowExecutionRestore::default()
        })
    }

    fn tree_id_source() -> impl FnMut() -> String {
        let mut counter = 0;
        move || {
            counter += 1;
            format!("id-{counter}")
        }
    }

    fn started_names(events: &[WorkflowEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                WorkflowEvent::NodeStarted { node_name, .. } => Some(node_name.clone()),
                _ => None,
            })
            .collect()
    }

    fn execution_id_of(execution: &WorkflowExecution, node_name: &str) -> String {
        execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == node_name)
            .unwrap_or_else(|| panic!("node execution '{node_name}' must exist"))
            .id
            .clone()
    }

    /// 起動済み leaf 群を先入れ先出しで完了させ続け、実行を終端まで進める。
    /// 完了させた leaf の (node_name, node_execution_id) を完了順で返す。
    fn drive_leaves_to_end(
        execution: &mut WorkflowExecution,
        initial: Vec<LeafStart>,
        new_id: &mut dyn FnMut() -> String,
    ) -> Vec<(String, String)> {
        let mut queue: std::collections::VecDeque<LeafStart> = initial.into();
        let mut completed = Vec::new();
        let mut now = 10.0;
        while let Some(leaf) = queue.pop_front() {
            now += 1.0;
            let applied = execution
                .complete_leaf_and_advance(&leaf.node_execution_id, new_id, now)
                .unwrap();
            completed.push((leaf.node_name, leaf.node_execution_id));
            if let ExecutionAdvanceDecision::StartLeaves(next) = applied.decision {
                queue.extend(next);
            }
        }
        completed
    }

    fn settle_session_leaf(
        execution: &mut WorkflowExecution,
        node_execution_id: &str,
        new_id: &mut dyn FnMut() -> String,
        timestamp: f64,
    ) -> AppliedNodeCompletionHandshake {
        assert_eq!(
            execution.record_node_completion_signal(
                node_execution_id,
                NodeCompletionSignal::Submit,
                timestamp,
            ),
            TransitionOutcome::Applied
        );
        assert_eq!(
            execution.record_node_completion_signal(
                node_execution_id,
                NodeCompletionSignal::Stop,
                timestamp,
            ),
            TransitionOutcome::Applied
        );
        execution
            .apply_node_completion_handshake(node_execution_id, new_id, timestamp)
            .unwrap()
    }

    #[test]
    fn fanout_child_sequence_runs_recursively_and_completes_bottom_up() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![ChildEntry::reference("fan"), ChildEntry::reference("after")],
            ),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("part"), ChildEntry::reference("solo")],
                    items: None,
                }),
                ..Default::default()
            },
            tree_sequence_node(
                "part",
                None,
                vec![ChildEntry::reference("s1"), ChildEntry::reference("s2")],
            ),
            tree_command_node("s1"),
            tree_command_node("s2"),
            tree_command_node("solo"),
            tree_command_node("after"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        assert_eq!(
            started_names(&applied.events),
            ["main", "fan", "part", "s1", "solo"]
        );
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("nested start must yield leaves");
        };
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| leaf.node_name.as_str())
                .collect::<Vec<_>>(),
            ["s1", "solo"]
        );

        // 親参照が実行木を成す: s1 → part（sequence の子）、part → fan（fanout の子）。
        let main_id = execution_id_of(&execution, "main");
        let fan_id = execution_id_of(&execution, "fan");
        let part_id = execution_id_of(&execution, "part");
        let s1 = execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "s1")
            .unwrap();
        assert_eq!(
            s1.parent,
            Some(ExecutionParentRef::sequence_child(&part_id))
        );
        let part = execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "part")
            .unwrap();
        let part_parent = part.parent.clone().unwrap();
        assert_eq!(part_parent.parent_id, fan_id);
        assert!(part_parent.fanout_slot.is_some());
        let fan = execution
            .node_executions()
            .iter()
            .find(|node| node.node_name == "fan")
            .unwrap();
        assert_eq!(
            fan.parent,
            Some(ExecutionParentRef::sequence_child(&main_id))
        );

        let completed = drive_leaves_to_end(&mut execution, leaves, &mut new_id);
        assert_eq!(
            completed
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["s1", "solo", "s2", "after"]
        );
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        for name in ["main", "fan", "part"] {
            assert_eq!(
                execution
                    .node_executions()
                    .iter()
                    .find(|node| node.node_name == name)
                    .unwrap()
                    .status,
                RuntimeNodeExecutionStatus::Succeeded,
                "composite instance '{name}' must complete bottom-up"
            );
        }
    }

    #[test]
    fn canonical_example_starts_nested_sequence_inside_fanout() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../workflows/examples/full-cycle-development.yml");
        let source = std::fs::read_to_string(source_path).unwrap();
        let workflow: WorkflowDefinition = serde_saphyr::from_str(&source).unwrap();
        let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
            id: "canonical-example-execution".to_string(),
            workflow,
            ..WorkflowExecutionRestore::default()
        });

        execution
            .replay_node_started("main", "main", NodeKindName::Sequence, 1, None, 1.0)
            .unwrap();
        execution
            .replay_node_started(
                "implementation",
                "implementation",
                NodeKindName::Sequence,
                1,
                Some(ExecutionParentRef::sequence_child("main")),
                2.0,
            )
            .unwrap();
        execution
            .replay_node_started(
                "create-detailed-design",
                "create_detailed_design",
                NodeKindName::Session,
                1,
                Some(ExecutionParentRef::sequence_child("implementation")),
                3.0,
            )
            .unwrap();

        let tasks = serde_json::json!({
            "tasks": [
                {
                    "task_id": "task-1",
                    "requirements": [],
                    "depends_on": [],
                    "parallel": true,
                    "files": [],
                    "outputs": [],
                    "verify": []
                },
                {
                    "task_id": "task-2",
                    "requirements": [],
                    "depends_on": [],
                    "parallel": true,
                    "files": [],
                    "outputs": [],
                    "verify": []
                }
            ]
        });
        assert_eq!(
            execution.record_pending_result(
                "create-detailed-design",
                Some("created two tasks".to_string()),
                Some(tasks),
                Some("implement-tasks".to_string()),
                None,
                4.0,
            ),
            TransitionOutcome::Applied
        );

        let mut new_id = tree_id_source();
        let applied =
            settle_session_leaf(&mut execution, "create-detailed-design", &mut new_id, 5.0);
        assert_eq!(
            started_names(&applied.events),
            [
                "implement_all",
                "implement_and_verify",
                "implement_task",
                "implement_and_verify",
                "implement_task",
            ]
        );
        let Some(ExecutionAdvanceDecision::StartLeaves(implement_leaves)) = applied.advance else {
            panic!("canonical example must start one implementation leaf per task");
        };
        assert_eq!(implement_leaves.len(), 2);

        let mut verify_leaves = Vec::new();
        for (index, leaf) in implement_leaves.iter().enumerate() {
            let applied = settle_session_leaf(
                &mut execution,
                &leaf.node_execution_id,
                &mut new_id,
                6.0 + index as f64,
            );
            let Some(ExecutionAdvanceDecision::StartLeaves(leaves)) = applied.advance else {
                panic!("implement_and_verify must advance from implement_task to verify_task");
            };
            assert_eq!(
                leaves
                    .iter()
                    .map(|leaf| leaf.node_name.as_str())
                    .collect::<Vec<_>>(),
                ["verify_task"]
            );
            verify_leaves.extend(leaves);
        }

        let mut final_started = Vec::new();
        for (index, leaf) in verify_leaves.iter().enumerate() {
            assert_eq!(
                execution.record_pending_result(
                    &leaf.node_execution_id,
                    Some("verified".to_string()),
                    Some(serde_json::json!({
                        "task_id": format!("task-{}", index + 1),
                        "complete": true,
                        "reason": "ok"
                    })),
                    Some("implement-task-check-result".to_string()),
                    None,
                    8.0 + index as f64,
                ),
                TransitionOutcome::Applied
            );
            let applied = settle_session_leaf(
                &mut execution,
                &leaf.node_execution_id,
                &mut new_id,
                10.0 + index as f64,
            );
            final_started.extend(started_names(&applied.events));
        }

        assert_eq!(final_started, ["merge_implementations"]);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .filter(|node| node.node_name == "implement_and_verify")
                .map(|node| node.status)
                .collect::<Vec<_>>(),
            [
                RuntimeNodeExecutionStatus::Succeeded,
                RuntimeNodeExecutionStatus::Succeeded,
            ]
        );
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.node_name == "implement_all")
                .expect("canonical fanout must have started")
                .status,
            RuntimeNodeExecutionStatus::Succeeded
        );
    }

    #[test]
    fn nested_sequence_output_child_artifact_becomes_the_part_artifact() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry::reference("prepare"),
                    ChildEntry::reference("part"),
                    ChildEntry::reference("report"),
                ],
            ),
            NodeDefinition {
                artifact: Some("part-result".to_string()),
                ..tree_sequence_node(
                    "part",
                    Some("inner-b"),
                    vec![
                        ChildEntry::reference("inner-a"),
                        ChildEntry::reference("inner-b"),
                    ],
                )
            },
            tree_command_node("prepare"),
            tree_command_node("inner-a"),
            tree_command_node("inner-b"),
            tree_command_node("report"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield the prepare leaf");
        };
        let prepare_id = leaves[0].node_execution_id.clone();
        let applied = execution
            .complete_leaf_and_advance(&prepare_id, &mut new_id, 2.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("main must advance into part");
        };
        assert_eq!(leaves[0].node_name, "inner-a");
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 3.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("part must advance into inner-b");
        };
        assert_eq!(leaves[0].node_name, "inner-b");
        let inner_b_id = leaves[0].node_execution_id.clone();

        let output_value = serde_json::json!({"verdict": "ok"});
        assert_eq!(
            execution.record_pending_result(
                &inner_b_id,
                Some("done".to_string()),
                Some(output_value.clone()),
                Some("part-result".to_string()),
                None,
                4.0,
            ),
            TransitionOutcome::Applied
        );
        let part_id = execution_id_of(&execution, "part");
        let applied = execution
            .complete_leaf_and_advance(&inner_b_id, &mut new_id, 5.0)
            .unwrap();

        // 部品 sequence の完了が output 子の Artifact を part の Artifact として発行する。
        let produced = applied
            .events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_execution_id,
                    node_name,
                    contract,
                    value,
                    ..
                } if node_execution_id == &part_id => {
                    Some((node_name.clone(), contract.clone(), value.clone()))
                }
                _ => None,
            })
            .expect("part completion must produce its artifact");
        assert_eq!(
            produced,
            (
                "part".to_string(),
                Some("part-result".to_string()),
                output_value.clone()
            )
        );
        assert_eq!(
            execution
                .flattened_artifacts()
                .get("part")
                .and_then(|artifact| artifact.artifact.clone()),
            Some(output_value)
        );
        // main は report へ前進している。
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("main must advance into report");
        };
        assert_eq!(leaves[0].node_name, "report");
    }

    #[test]
    fn nested_sequence_without_output_artifact_fails_as_validation_failure() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry::reference("part"),
                    ChildEntry::reference("report"),
                ],
            ),
            NodeDefinition {
                artifact: Some("part-result".to_string()),
                ..tree_sequence_node("part", Some("inner"), vec![ChildEntry::reference("inner")])
            },
            tree_command_node("inner"),
            tree_command_node("report"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield the inner leaf");
        };
        let part_id = execution_id_of(&execution, "part");
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 2.0)
            .unwrap();

        assert!(applied.events.iter().any(|event| matches!(
            event,
            WorkflowEvent::NodeFailed {
                node_execution_id,
                failure_kind: NodeExecutionFailureKind::ValidationFailure,
                ..
            } if node_execution_id == &part_id
        )));
        assert_eq!(applied.decision, ExecutionAdvanceDecision::Persist);
        assert_eq!(*execution.state(), RuntimeExecutionState::Running);
        assert!(
            !execution
                .node_executions()
                .iter()
                .any(|node| node.node_name == "report"),
            "a failed part must not advance the parent sequence"
        );
    }

    #[test]
    fn nested_approval_pauses_inside_the_tree_and_resumes_in_place() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry::reference("part"),
                    ChildEntry::reference("report"),
                ],
            ),
            NodeDefinition {
                completion: NodeCompletion::Approval,
                ..tree_sequence_node("part", None, vec![ChildEntry::reference("inner")])
            },
            tree_command_node("inner"),
            tree_command_node("report"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield the inner leaf");
        };
        let part_id = execution_id_of(&execution, "part");
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 2.0)
            .unwrap();

        // ネスト内で承認待ち停止: part は WaitingApproval、前進しない。
        assert!(applied.events.iter().any(|event| matches!(
            event,
            WorkflowEvent::ApprovalRequested { node_execution_id, .. }
                if node_execution_id == &part_id
        )));
        assert_eq!(applied.decision, ExecutionAdvanceDecision::Persist);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == part_id)
                .unwrap()
                .status,
            RuntimeNodeExecutionStatus::WaitingApproval
        );
        assert_eq!(execution.display_current_node(), Some("part".to_string()));

        // 承認でネスト位置から再開し、親 sequence が report へ前進する。
        let applied = execution
            .apply_approval(&part_id, &mut new_id, 3.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("approval must resume the parent sequence");
        };
        assert_eq!(leaves[0].node_name, "report");
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 4.0)
            .unwrap();
        assert!(applied
            .events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionCompleted { .. })));
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn replay_restores_the_nested_position_for_resume() {
        let nodes = vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry::reference("part"),
                    ChildEntry::reference("report"),
                ],
            ),
            tree_sequence_node(
                "part",
                None,
                vec![
                    ChildEntry::reference("inner-a"),
                    ChildEntry::reference("inner-b"),
                ],
            ),
            tree_command_node("inner-a"),
            tree_command_node("inner-b"),
            tree_command_node("report"),
        ];
        let mut live = tree_execution(nodes.clone());
        let mut new_id = tree_id_source();
        let applied = live.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield the inner-a leaf");
        };
        let advanced = live
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 2.0)
            .unwrap();
        let mut events = applied.events;
        events.extend(advanced.events);

        // 事実列だけからスコープ木を再構築する（inner-b 実行中の位置）。
        let mut replayed = tree_execution(nodes);
        for event in &events {
            match event {
                WorkflowEvent::NodeStarted {
                    node_execution_id,
                    node_name,
                    kind,
                    attempt,
                    parent,
                    timestamp,
                    ..
                } => replayed
                    .replay_node_started(
                        node_execution_id,
                        node_name,
                        *kind,
                        *attempt,
                        parent.clone(),
                        *timestamp,
                    )
                    .unwrap(),
                WorkflowEvent::NodeCompleted {
                    node_execution_id,
                    timestamp,
                    ..
                } => replayed
                    .derive_session_settlement(node_execution_id, *timestamp)
                    .unwrap(),
                _ => {}
            }
        }

        let main_id = execution_id_of(&live, "main");
        let part_id = execution_id_of(&live, "part");
        let inner_b_id = execution_id_of(&live, "inner-b");
        assert!(replayed.scope(&main_id).is_some());
        assert!(replayed.scope(&part_id).is_some());
        assert_eq!(
            replayed
                .scope(&part_id)
                .and_then(ScopeRuntime::sequence)
                .and_then(|sequence| sequence.current_child.clone()),
            Some("inner-b".to_string())
        );
        assert_eq!(replayed.display_current_node(), Some("inner-b".to_string()));
        let leaf = replayed
            .leaf_start_for(&inner_b_id)
            .expect("the interrupted leaf must be restartable in place");
        assert_eq!(leaf.node_name, "inner-b");
        assert_eq!(
            replayed
                .node_executions()
                .iter()
                .find(|node| node.id == inner_b_id)
                .unwrap()
                .parent,
            Some(ExecutionParentRef::sequence_child(&part_id))
        );
    }

    #[test]
    fn parallel_fanout_lanes_keep_independent_loop_guard_counts() {
        // fan は同じ部品 sequence "part" を items 2 件で並走させる。part 内の
        // fix は loop_guard(2) で自己ループする。lane 0 が予算を使い切っても
        // lane 1 の fix は自分のスコープの予算で 2 回目に入れる。
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", None, vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("part")],
                    items: Some(crate::domain::workflow::ItemsSource::Literal(vec![
                        serde_json::json!("a"),
                        serde_json::json!("b"),
                    ])),
                }),
                ..Default::default()
            },
            tree_sequence_node(
                "part",
                None,
                vec![
                    ChildEntry {
                        on_failure: None,
                        name: "fix".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "exit".to_string(),
                            },
                            Rule::Next("fix".to_string()),
                        ]),
                    },
                    ChildEntry::reference("exit"),
                ],
            ),
            tree_command_node("fix"),
            tree_command_node("exit"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield one fix leaf per lane");
        };
        assert_eq!(
            leaves
                .iter()
                .map(|leaf| leaf.node_name.as_str())
                .collect::<Vec<_>>(),
            ["fix", "fix"]
        );
        let lane_parts: Vec<String> = leaves
            .iter()
            .map(|leaf| {
                execution
                    .node_executions()
                    .iter()
                    .find(|node| node.id == leaf.node_execution_id)
                    .and_then(|node| node.parent.clone())
                    .expect("a lane fix must hang under its part instance")
                    .parent_id
            })
            .collect();
        assert_ne!(
            lane_parts[0], lane_parts[1],
            "each lane must run its own part instance"
        );

        // lane 0 が fix の予算 2 回を使い切り exit へ抜ける。
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 2.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(lane0_second) = applied.decision else {
            panic!("lane 0 must revisit fix");
        };
        assert_eq!(lane0_second[0].node_name, "fix");
        let applied = execution
            .complete_leaf_and_advance(&lane0_second[0].node_execution_id, &mut new_id, 3.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(lane0_exit) = applied.decision else {
            panic!("lane 0 must exhaust into exit");
        };
        assert_eq!(lane0_exit[0].node_name, "exit");

        // lane 1 の fix はカウント独立: lane 0 が 2 回消費済みでも 2 回目に入れる。
        let applied = execution
            .complete_leaf_and_advance(&leaves[1].node_execution_id, &mut new_id, 4.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(lane1_second) = applied.decision else {
            panic!("lane 1 must revisit fix with its own budget");
        };
        assert_eq!(lane1_second[0].node_name, "fix");
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == lane1_second[0].node_execution_id)
                .and_then(|node| node.parent.clone())
                .unwrap()
                .parent_id,
            lane_parts[1],
            "the second fix of lane 1 must stay in lane 1's part instance"
        );

        // 残りを流し切ると全体が完了する。
        let mut queue = vec![lane0_exit[0].clone(), lane1_second[0].clone()];
        let mut now = 5.0;
        while let Some(leaf) = queue.pop() {
            now += 1.0;
            let applied = execution
                .complete_leaf_and_advance(&leaf.node_execution_id, &mut new_id, now)
                .unwrap();
            if let ExecutionAdvanceDecision::StartLeaves(next) = applied.decision {
                queue.extend(next);
            }
        }
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .filter(|node| node.node_name == "fix")
                .count(),
            4,
            "each lane must have run fix twice"
        );
    }

    #[test]
    fn part_sequence_input_parameters_feed_child_bindings() {
        // main は prepare の Artifact を part の input `target` に配線し、
        // part 内の worker は `target` を自分のパラメータ `data` として受け取る。
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry::reference("prepare"),
                    ChildEntry {
                        on_failure: None,
                        name: "part".to_string(),
                        inputs: vec![(
                            "target".to_string(),
                            crate::domain::workflow::value_objects::InputSourceRef::new("prepare"),
                        )],
                        rules: None,
                    },
                ],
            ),
            NodeDefinition {
                input: vec![crate::domain::workflow::InputParam {
                    name: "target".to_string(),
                    contract: None,
                }],
                ..tree_sequence_node(
                    "part",
                    None,
                    vec![ChildEntry {
                        on_failure: None,
                        name: "worker".to_string(),
                        inputs: vec![(
                            "data".to_string(),
                            crate::domain::workflow::value_objects::InputSourceRef::new("target"),
                        )],
                        rules: None,
                    }],
                )
            },
            tree_command_node("prepare"),
            tree_command_node("worker"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield the prepare leaf");
        };
        let prepared_value = serde_json::json!({"path": "src/lib.rs"});
        assert_eq!(
            execution.record_pending_result(
                &leaves[0].node_execution_id,
                Some("done".to_string()),
                Some(prepared_value.clone()),
                None,
                None,
                2.0,
            ),
            TransitionOutcome::Applied
        );
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 3.0)
            .unwrap();

        // part スコープは input `target` を prepare の Artifact で束縛し、
        // worker の起動束縛は `target` から `data` を受け取る。
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("main must advance into part");
        };
        assert_eq!(leaves[0].node_name, "worker");
        assert_eq!(
            leaves[0].bindings,
            vec![("data".to_string(), prepared_value.clone())]
        );
        let part_id = execution_id_of(&execution, "part");
        assert_eq!(
            execution
                .scope(&part_id)
                .map(|scope| scope.parameters.clone()),
            Some(vec![("target".to_string(), prepared_value)])
        );
    }

    #[test]
    fn abort_records_every_active_lane_leaf_even_with_equal_name_and_attempt() {
        // fanout の並走 lane は同じ部品 sequence を走らせるため、同名 node が
        // 同一 attempt（スコープ採番）でアクティブになる。abort は全 lane の
        // leaf を記録する。
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", None, vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("part")],
                    items: Some(crate::domain::workflow::ItemsSource::Literal(vec![
                        serde_json::json!("a"),
                        serde_json::json!("b"),
                    ])),
                }),
                ..Default::default()
            },
            tree_sequence_node("part", None, vec![ChildEntry::reference("fix")]),
            tree_command_node("fix"),
        ]);
        let mut new_id = tree_id_source();
        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield one fix leaf per lane");
        };
        assert_eq!(leaves.len(), 2);
        let fixes: Vec<_> = execution
            .node_executions()
            .iter()
            .filter(|node| node.node_name == "fix")
            .collect();
        assert_eq!(
            (fixes[0].attempt, fixes[1].attempt),
            (1, 1),
            "both lanes must carry the same scope-local attempt"
        );

        execution.record_aborted_history_for_active_leaves(2.0);

        let aborted: Vec<_> = execution
            .node_history
            .iter()
            .filter(|entry| {
                entry.node_name == "fix"
                    && entry.state == crate::domain::workflow::value_objects::NODE_STATUS_ABORTED
            })
            .collect();
        assert_eq!(
            aborted.len(),
            2,
            "every active lane leaf must get its own aborted entry"
        );
    }

    #[test]
    fn direct_fanout_child_lanes_each_start_at_attempt_one_and_retry_independently() {
        // items 2 件の直接 fanout 子（leaf）は lane ごとに attempt 1 で始まり、
        // 片方の retry だけがその lane の attempt 2 になる。
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", None, vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![ChildEntry::reference("worker")],
                    items: Some(crate::domain::workflow::ItemsSource::Literal(vec![
                        serde_json::json!("a"),
                        serde_json::json!("b"),
                    ])),
                }),
                ..Default::default()
            },
            tree_command_node("worker"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield one worker leaf per lane");
        };
        let attempts: Vec<u32> = leaves
            .iter()
            .map(|leaf| {
                execution
                    .node_executions()
                    .iter()
                    .find(|node| node.id == leaf.node_execution_id)
                    .unwrap()
                    .attempt
            })
            .collect();
        assert_eq!(attempts, [1, 1], "each lane must start at attempt 1");

        // lane 0 を失敗させて retry すると、その lane だけ attempt 2 になる。
        let lane0 = leaves[0].node_execution_id.clone();
        assert_eq!(
            execution.fail_leaf_execution(
                &lane0,
                "exit 1".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
                2.0,
            ),
            TransitionOutcome::Applied
        );
        let restarted = execution
            .restart_node_attempt_at(
                &lane0,
                "retry-1".to_string(),
                3.0,
                NodeRestartMode::ExplicitRetry,
            )
            .expect("a failed lane leaf must be retryable");
        assert_eq!(restarted.attempt.attempt, 2);
        // lane 1 は attempt 1 のまま。
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .find(|node| node.id == leaves[1].node_execution_id)
                .unwrap()
                .attempt,
            1
        );
    }

    #[test]
    fn revisited_part_sequence_gets_a_fresh_loop_guard_budget() {
        // main は part を loop_guard(2) で再訪し、part 内部の fix も
        // loop_guard(2) で自己ループする。カウントの範囲はスコープなので、
        // part の再訪ごとに内部カウントはフレッシュになる。
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry {
                        on_failure: None,
                        name: "part".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "finish".to_string(),
                            },
                            Rule::Next("part".to_string()),
                        ]),
                    },
                    ChildEntry::reference("finish"),
                ],
            ),
            tree_sequence_node(
                "part",
                None,
                vec![
                    ChildEntry {
                        on_failure: None,
                        name: "fix".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![
                            Rule::LoopGuard {
                                max_iterations: 2,
                                on_exhausted: "exit".to_string(),
                            },
                            Rule::Next("fix".to_string()),
                        ]),
                    },
                    ChildEntry::reference("exit"),
                ],
            ),
            tree_command_node("fix"),
            tree_command_node("exit"),
            tree_command_node("finish"),
        ]);
        let mut new_id = tree_id_source();

        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield the first fix leaf");
        };
        let completed = drive_leaves_to_end(&mut execution, leaves, &mut new_id);

        // part 2 訪問 × 内部 fix 2 回ずつ。1 回目の消費が持ち越されるなら
        // 2 回目の fix は 1 回で exhausted になり、この列は崩れる。
        assert_eq!(
            completed
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["fix", "fix", "exit", "fix", "fix", "exit", "finish"]
        );
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
        assert_eq!(
            execution
                .node_executions()
                .iter()
                .filter(|node| node.node_name == "part")
                .count(),
            2,
            "part must have one execution instance per visit"
        );
    }

    // --- children エントリの on_failure（#1465） -----------------------------

    use crate::domain::workflow::OnFailure;

    fn entry_with_on_failure(name: &str, on_failure: OnFailure) -> ChildEntry {
        ChildEntry {
            on_failure: Some(on_failure),
            ..ChildEntry::reference(name)
        }
    }

    fn fail_leaf(execution: &mut WorkflowExecution, node_execution_id: &str, timestamp: f64) {
        assert_eq!(
            execution.fail_leaf_execution(
                node_execution_id,
                "exit 1".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                FailureDisposition::Terminal,
                timestamp,
            ),
            TransitionOutcome::Applied
        );
    }

    fn start_single_leaf(
        execution: &mut WorkflowExecution,
        new_id: &mut dyn FnMut() -> String,
    ) -> String {
        let applied = execution.start_root(new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield a leaf");
        };
        assert_eq!(leaves.len(), 1);
        leaves[0].node_execution_id.clone()
    }

    #[test]
    fn sequence_child_auto_retry_consumes_budget_then_falls_back_to_halt() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    entry_with_on_failure("flaky", OnFailure::Retry(2)),
                    ChildEntry::reference("after"),
                ],
            ),
            tree_command_node("flaky"),
            tree_command_node("after"),
        ]);
        let mut new_id = tree_id_source();
        let mut current = start_single_leaf(&mut execution, &mut new_id);

        // 自動 retry は手動 Retry と同じ記録形式（NodeRetryRequested + NodeStarted）
        // で attempt を進める。retry: 2 は2回まで。
        for expected_attempt in [2u32, 3u32] {
            fail_leaf(&mut execution, &current, 2.0);
            let outcome = execution
                .apply_on_failure_treatment(&current, &mut new_id, 3.0)
                .unwrap()
                .expect("budget must allow an automatic retry");
            assert!(matches!(
                &outcome.events[0],
                WorkflowEvent::NodeRetryRequested { node_execution_id, .. }
                    if *node_execution_id == current
            ));
            let WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                attempt,
                ..
            } = &outcome.events[1]
            else {
                panic!("automatic retry must record NodeStarted");
            };
            assert_eq!(node_name, "flaky");
            assert_eq!(*attempt, expected_attempt);
            assert_eq!(outcome.leaves.len(), 1);
            assert_eq!(outcome.leaves[0].node_execution_id, *node_execution_id);
            current = node_execution_id.clone();
        }

        // 2回の自動再実行後の失敗は既定（中断・Retry 待ち）へ落ちる。
        fail_leaf(&mut execution, &current, 8.0);
        assert_eq!(
            execution
                .apply_on_failure_treatment(&current, &mut new_id, 9.0)
                .unwrap(),
            None
        );
        assert_eq!(*execution.state(), RuntimeExecutionState::Running);

        // 手動 Retry でも予算は復活しない。
        let restarted = execution
            .restart_node_attempt_at(&current, new_id(), 10.0, NodeRestartMode::ExplicitRetry)
            .expect("failed attempt must accept a manual retry");
        assert_eq!(restarted.attempt.attempt, 4);
        let manual = restarted.attempt.id.clone();
        fail_leaf(&mut execution, &manual, 11.0);
        assert_eq!(
            execution
                .apply_on_failure_treatment(&manual, &mut new_id, 12.0)
                .unwrap(),
            None,
            "manual retries must not refill the automatic budget"
        );
    }

    #[test]
    fn sequence_revisit_restores_the_auto_retry_budget() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry {
                        on_failure: Some(OnFailure::Retry(1)),
                        name: "flaky".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![Rule::Next("back".to_string())]),
                    },
                    ChildEntry {
                        on_failure: None,
                        name: "back".to_string(),
                        inputs: Vec::new(),
                        rules: Some(vec![Rule::Next("flaky".to_string())]),
                    },
                ],
            ),
            tree_command_node("flaky"),
            tree_command_node("back"),
        ]);
        let mut new_id = tree_id_source();
        let first = start_single_leaf(&mut execution, &mut new_id);

        // visit 1: 自動 retry 1回で予算切れ。
        fail_leaf(&mut execution, &first, 2.0);
        let auto = execution
            .apply_on_failure_treatment(&first, &mut new_id, 3.0)
            .unwrap()
            .expect("first failure must auto-retry");
        let second = auto.leaves[0].node_execution_id.clone();
        fail_leaf(&mut execution, &second, 4.0);
        assert_eq!(
            execution
                .apply_on_failure_treatment(&second, &mut new_id, 5.0)
                .unwrap(),
            None
        );

        // 手動 Retry で成功し、back を経由して flaky を再訪する。
        let restarted = execution
            .restart_node_attempt_at(&second, new_id(), 6.0, NodeRestartMode::ExplicitRetry)
            .unwrap();
        let third = restarted.attempt.id.clone();
        let applied = execution
            .complete_leaf_and_advance(&third, &mut new_id, 7.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("flaky completion must start back");
        };
        assert_eq!(leaves[0].node_name, "back");
        let applied = execution
            .complete_leaf_and_advance(&leaves[0].node_execution_id, &mut new_id, 8.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("back completion must revisit flaky");
        };
        let revisit = leaves[0].node_execution_id.clone();
        assert_eq!(
            execution.node_execution(&revisit).unwrap().attempt,
            4,
            "revisit must continue the scope-wide attempt numbering"
        );

        // 再訪 = 新しい visit なので予算はフレッシュ。
        fail_leaf(&mut execution, &revisit, 9.0);
        let outcome = execution
            .apply_on_failure_treatment(&revisit, &mut new_id, 10.0)
            .unwrap()
            .expect("a fresh visit must restore the auto-retry budget");
        assert_eq!(outcome.leaves.len(), 1);
        assert_eq!(
            execution
                .node_execution(&outcome.leaves[0].node_execution_id)
                .unwrap()
                .attempt,
            5
        );
    }

    #[test]
    fn sequence_ignored_child_failure_advances_to_the_next_entry() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    entry_with_on_failure("optional", OnFailure::Ignore),
                    ChildEntry::reference("after"),
                ],
            ),
            tree_command_node("optional"),
            tree_command_node("after"),
        ]);
        let mut new_id = tree_id_source();
        let optional = start_single_leaf(&mut execution, &mut new_id);

        fail_leaf(&mut execution, &optional, 2.0);
        let outcome = execution
            .apply_on_failure_treatment(&optional, &mut new_id, 3.0)
            .unwrap()
            .expect("ignored failure must continue the sequence");
        assert_eq!(started_names(&outcome.events), ["after"]);
        assert_eq!(outcome.leaves.len(), 1);
        assert_eq!(outcome.leaves[0].node_name, "after");
        let main_id = execution_id_of(&execution, "main");
        let sequence = execution.scope(&main_id).unwrap().sequence().unwrap();
        assert_eq!(sequence.current_child.as_deref(), Some("after"));
        assert!(!sequence.artifacts.contains_key("optional"));
    }

    #[test]
    fn sequence_ignored_child_failure_at_the_terminal_completes_the_scope() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    ChildEntry::reference("work"),
                    entry_with_on_failure("optional", OnFailure::Ignore),
                ],
            ),
            tree_command_node("work"),
            tree_command_node("optional"),
        ]);
        let mut new_id = tree_id_source();
        let work = start_single_leaf(&mut execution, &mut new_id);
        let applied = execution
            .complete_leaf_and_advance(&work, &mut new_id, 2.0)
            .unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("work completion must start optional");
        };
        let optional = leaves[0].node_execution_id.clone();

        fail_leaf(&mut execution, &optional, 3.0);
        let outcome = execution
            .apply_on_failure_treatment(&optional, &mut new_id, 4.0)
            .unwrap()
            .expect("ignored terminal failure must complete the sequence");
        assert!(outcome.leaves.is_empty());
        assert!(outcome
            .events
            .iter()
            .any(|event| matches!(event, WorkflowEvent::ExecutionCompleted { .. })));
        assert_eq!(*execution.state(), RuntimeExecutionState::Completed);
    }

    #[test]
    fn fanout_ignored_failed_child_is_excluded_from_the_aggregate_array() {
        let mut execution = tree_execution(vec![
            tree_sequence_node(
                "main",
                None,
                vec![ChildEntry::reference("fan"), ChildEntry::reference("after")],
            ),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![
                        ChildEntry::reference("steady"),
                        entry_with_on_failure("flaky", OnFailure::Ignore),
                    ],
                    items: None,
                }),
                ..Default::default()
            },
            tree_command_node("steady"),
            tree_command_node("flaky"),
            tree_command_node("after"),
        ]);
        let mut new_id = tree_id_source();
        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield both fanout children");
        };
        let steady = leaves[0].node_execution_id.clone();
        let flaky = leaves[1].node_execution_id.clone();

        // ignore の失敗が先に決着しても、残りの子が走っている間は前進しない。
        fail_leaf(&mut execution, &flaky, 2.0);
        let outcome = execution
            .apply_on_failure_treatment(&flaky, &mut new_id, 3.0)
            .unwrap()
            .expect("ignored fanout failure must be treated");
        assert!(outcome.events.is_empty());
        assert!(outcome.leaves.is_empty());

        // 最後の子の完了で fanout が完了し、失敗子は結果配列から除かれる。
        let applied = execution
            .complete_leaf_and_advance(&steady, &mut new_id, 4.0)
            .unwrap();
        let fan_id = execution_id_of(&execution, "fan");
        let aggregated = applied
            .events
            .iter()
            .find_map(|event| match event {
                WorkflowEvent::ArtifactProduced {
                    node_execution_id,
                    value,
                    ..
                } if *node_execution_id == fan_id => Some(value.clone()),
                _ => None,
            })
            .expect("fanout completion must aggregate an array");
        let serde_json::Value::Array(values) = aggregated else {
            panic!("fanout aggregate must be an array");
        };
        assert_eq!(
            values.len(),
            1,
            "the ignored failed child must be excluded from the aggregate"
        );
        assert!(started_names(&applied.events).contains(&"after".to_string()));
    }

    #[test]
    fn fanout_ignore_advance_is_blocked_while_an_undeclared_failure_halts() {
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", None, vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![
                        ChildEntry::reference("steady"),
                        entry_with_on_failure("flaky", OnFailure::Ignore),
                    ],
                    items: None,
                }),
                ..Default::default()
            },
            tree_command_node("steady"),
            tree_command_node("flaky"),
        ]);
        let mut new_id = tree_id_source();
        let applied = execution.start_root(&mut new_id, 1.0).unwrap();
        let ExecutionAdvanceDecision::StartLeaves(leaves) = applied.decision else {
            panic!("start must yield both fanout children");
        };
        let steady = leaves[0].node_execution_id.clone();
        let flaky = leaves[1].node_execution_id.clone();

        // 宣言なしの失敗は現行どおり中断（treatment なし）。
        fail_leaf(&mut execution, &steady, 2.0);
        assert_eq!(
            execution
                .apply_on_failure_treatment(&steady, &mut new_id, 3.0)
                .unwrap(),
            None
        );

        // ignore の失敗決着でも、中断待ちの失敗 slot がある間は完了させない。
        fail_leaf(&mut execution, &flaky, 4.0);
        assert_eq!(
            execution
                .apply_on_failure_treatment(&flaky, &mut new_id, 5.0)
                .unwrap(),
            None
        );
        let fan_id = execution_id_of(&execution, "fan");
        assert!(execution.scope(&fan_id).is_some(), "fanout must stay open");
        assert_eq!(*execution.state(), RuntimeExecutionState::Running);
    }

    #[test]
    fn fanout_child_auto_retry_uses_lane_attempt_numbering() {
        let mut execution = tree_execution(vec![
            tree_sequence_node("main", None, vec![ChildEntry::reference("fan")]),
            NodeDefinition {
                name: "fan".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    children: vec![entry_with_on_failure("flaky", OnFailure::Retry(1))],
                    items: None,
                }),
                ..Default::default()
            },
            tree_command_node("flaky"),
        ]);
        let mut new_id = tree_id_source();
        let first = start_single_leaf(&mut execution, &mut new_id);

        fail_leaf(&mut execution, &first, 2.0);
        let outcome = execution
            .apply_on_failure_treatment(&first, &mut new_id, 3.0)
            .unwrap()
            .expect("lane failure must auto-retry once");
        let second = outcome.leaves[0].node_execution_id.clone();
        assert_eq!(execution.node_execution(&second).unwrap().attempt, 2);
        let fan_id = execution_id_of(&execution, "fan");
        let slot = &execution.scope(&fan_id).unwrap().fanout().unwrap().children[0];
        assert_eq!(slot.attempt, 2);
        assert_eq!(slot.node_execution_id, second);

        fail_leaf(&mut execution, &second, 4.0);
        assert_eq!(
            execution
                .apply_on_failure_treatment(&second, &mut new_id, 5.0)
                .unwrap(),
            None,
            "the lane budget must be exhausted after one automatic retry"
        );
    }

    #[test]
    fn replayed_events_restore_the_same_visit_budget_as_the_live_run() {
        let nodes = vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    entry_with_on_failure("flaky", OnFailure::Retry(2)),
                    ChildEntry::reference("after"),
                ],
            ),
            tree_command_node("flaky"),
            tree_command_node("after"),
        ];

        // live: 開始 → 失敗 → 自動 retry。
        let mut live = tree_execution(nodes.clone());
        let mut new_id = tree_id_source();
        let first = start_single_leaf(&mut live, &mut new_id);
        fail_leaf(&mut live, &first, 2.0);
        let outcome = live
            .apply_on_failure_treatment(&first, &mut new_id, 3.0)
            .unwrap()
            .unwrap();
        let second = outcome.leaves[0].node_execution_id.clone();

        // replay: 同じ事実列（NodeStarted×2 → NodeFailed → NodeRetryRequested →
        // NodeStarted）の適用。
        let mut replayed = tree_execution(nodes);
        let main_id = execution_id_of(&live, "main");
        replayed
            .replay_node_started(&main_id, "main", NodeKindName::Sequence, 1, None, 1.0)
            .unwrap();
        replayed
            .replay_node_started(
                &first,
                "flaky",
                NodeKindName::Command,
                1,
                Some(ExecutionParentRef::sequence_child(&main_id)),
                1.0,
            )
            .unwrap();
        replayed
            .derive_leaf_failed(
                &first,
                "exit 1".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                2.0,
            )
            .unwrap();
        assert_eq!(
            replayed.request_node_retry(&first, 3.0),
            TransitionOutcome::Applied
        );
        replayed
            .replay_node_started(
                &second,
                "flaky",
                NodeKindName::Command,
                2,
                Some(ExecutionParentRef::sequence_child(&main_id)),
                3.0,
            )
            .unwrap();

        // スコープの visit 予算が live と一致する。
        let live_sequence = live.scope(&main_id).unwrap().sequence().unwrap().clone();
        let replayed_sequence = replayed
            .scope(&main_id)
            .unwrap()
            .sequence()
            .unwrap()
            .clone();
        assert_eq!(live_sequence, replayed_sequence);
        assert_eq!(replayed_sequence.child_counts.get("flaky"), Some(&2));
        assert_eq!(replayed_sequence.visit_bases.get("flaky"), Some(&0));
        assert_eq!(
            live.runtime.retry_predecessors,
            replayed.runtime.retry_predecessors
        );
        assert_eq!(
            replayed.runtime.retry_predecessors.get(&second),
            Some(&first)
        );

        // 予算判定の導出も一致する: 次の失敗はどちらも自動 retry できる。
        let mut next_attempt_id = || "next-attempt".to_string();
        fail_leaf(&mut live, &second, 4.0);
        fail_leaf(&mut replayed, &second, 4.0);
        let live_next = live
            .apply_on_failure_treatment(&second, &mut next_attempt_id, 5.0)
            .unwrap();
        let replayed_next = replayed
            .apply_on_failure_treatment(&second, &mut next_attempt_id, 5.0)
            .unwrap();
        assert!(live_next.is_some());
        assert_eq!(live_next, replayed_next);
    }

    #[test]
    fn replayed_ignore_advancement_facts_restore_the_same_tree_as_the_live_run() {
        let nodes = vec![
            tree_sequence_node(
                "main",
                None,
                vec![
                    entry_with_on_failure("optional", OnFailure::Ignore),
                    ChildEntry::reference("after"),
                ],
            ),
            tree_command_node("optional"),
            tree_command_node("after"),
        ];

        // live: 開始 → 失敗 → ignore 前進（次エントリの NodeStarted が事実として残る）。
        let mut live = tree_execution(nodes.clone());
        let mut new_id = tree_id_source();
        let optional = start_single_leaf(&mut live, &mut new_id);
        fail_leaf(&mut live, &optional, 2.0);
        let outcome = live
            .apply_on_failure_treatment(&optional, &mut new_id, 3.0)
            .unwrap()
            .unwrap();
        let after = outcome.leaves[0].node_execution_id.clone();

        // replay: NodeStarted×2 → NodeFailed → NodeStarted（前進の事実）。
        let mut replayed = tree_execution(nodes);
        let main_id = execution_id_of(&live, "main");
        replayed
            .replay_node_started(&main_id, "main", NodeKindName::Sequence, 1, None, 1.0)
            .unwrap();
        replayed
            .replay_node_started(
                &optional,
                "optional",
                NodeKindName::Command,
                1,
                Some(ExecutionParentRef::sequence_child(&main_id)),
                1.0,
            )
            .unwrap();
        replayed
            .derive_leaf_failed(
                &optional,
                "exit 1".to_string(),
                NodeExecutionFailureKind::ValidationFailure,
                2.0,
            )
            .unwrap();
        replayed
            .replay_node_started(
                &after,
                "after",
                NodeKindName::Command,
                1,
                Some(ExecutionParentRef::sequence_child(&main_id)),
                3.0,
            )
            .unwrap();

        // スコープ状態（カーソル・カウント・visit 基点）が live と一致する。
        assert_eq!(
            live.scope(&main_id).unwrap().sequence().unwrap(),
            replayed.scope(&main_id).unwrap().sequence().unwrap()
        );
        // 実行木の node 状態も一致する（optional は Failed のまま、after が実行中）。
        for id in [&optional, &after] {
            assert_eq!(
                live.node_execution(id).unwrap().status,
                replayed.node_execution(id).unwrap().status
            );
        }
    }
}
