//! Workflow execution lifecycle aggregate.
//!
//! This aggregate is the single authority for execution lifecycle admission and
//! transitions. Runtime orchestration and event replay both call these methods;
//! neither is allowed to assign lifecycle state directly.

use crate::domain::workflow::value_objects::{
    ExecutionInterruptionReason, NodeExecutionFailureKind, RuntimeExecutionState, TokenUsage,
};
use crate::domain::workflow::FailureDisposition;

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutRuntimeState {
    pub parent_node_name: String,
    pub children: Vec<FanoutChildRuntime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FanoutChildRuntime {
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

/// Aggregate that owns the workflow execution lifecycle state.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExecution {
    state: RuntimeExecutionState,
    interruption_reason: Option<ExecutionInterruptionReason>,
}

impl WorkflowExecution {
    pub fn start() -> Self {
        Self {
            state: RuntimeExecutionState::Running,
            interruption_reason: None,
        }
    }

    /// Restores a lifecycle snapshot before replaying subsequent durable facts.
    pub fn restore(
        state: RuntimeExecutionState,
        interruption_reason: Option<ExecutionInterruptionReason>,
    ) -> Self {
        Self {
            state,
            interruption_reason,
        }
    }

    pub fn state(&self) -> &RuntimeExecutionState {
        &self.state
    }

    #[cfg(test)]
    pub fn interruption_reason(&self) -> Option<ExecutionInterruptionReason> {
        self.interruption_reason
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

    pub fn is_resumable(&self) -> bool {
        self.state_set() == ExecutionStateSet::Resumable
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

    pub fn replay_interrupted(&mut self, reason: ExecutionInterruptionReason) -> ReplayOutcome {
        transition_to_replay(self.interrupt(reason))
    }

    pub fn replay_resumed(&mut self) -> ReplayOutcome {
        transition_to_replay(self.resume())
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
        let mut active = aggregate(RuntimeExecutionState::Running);
        assert_eq!(active.stop(), TransitionOutcome::Applied);
        assert_eq!(
            active.stop(),
            TransitionOutcome::Rejected(TransitionRejection::AlreadyStopped)
        );
        let mut finished = aggregate(RuntimeExecutionState::Completed);
        assert_eq!(
            finished.stop(),
            TransitionOutcome::Rejected(TransitionRejection::NotActive)
        );

        let mut active = aggregate(RuntimeExecutionState::Running);
        assert_eq!(
            active.resume(),
            TransitionOutcome::Rejected(TransitionRejection::NotResumable)
        );
        let mut resumable = aggregate(RuntimeExecutionState::Interrupted);
        assert_eq!(resumable.resume(), TransitionOutcome::Applied);
        let mut finished = aggregate(RuntimeExecutionState::Completed);
        assert_eq!(
            finished.resume(),
            TransitionOutcome::Rejected(TransitionRejection::NotResumable)
        );

        for state in [
            RuntimeExecutionState::Running,
            RuntimeExecutionState::Interrupted,
        ] {
            assert_eq!(aggregate(state).abort(), TransitionOutcome::Applied);
        }
        assert_eq!(
            aggregate(RuntimeExecutionState::Completed).abort(),
            TransitionOutcome::NotApplicable
        );
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
                RuntimeExecutionState::Completed,
            ] {
                assert_eq!(
                    operation(&mut aggregate(state)),
                    TransitionOutcome::Rejected(TransitionRejection::NotWaitingApproval)
                );
            }
        }

        assert_eq!(
            aggregate(RuntimeExecutionState::Running).admit_artifact_submission(true),
            TransitionOutcome::Applied
        );
        assert_eq!(
            aggregate(RuntimeExecutionState::Running).admit_artifact_submission(false),
            TransitionOutcome::Rejected(TransitionRejection::ArtifactNotAccepted)
        );
        for state in [
            RuntimeExecutionState::Interrupted,
            RuntimeExecutionState::Completed,
        ] {
            assert_eq!(
                aggregate(state).admit_artifact_submission(true),
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            );
        }
    }

    #[test]
    fn operation_state_matrix_turn_completion() {
        for (state, application) in [
            (
                RuntimeExecutionState::Running,
                TurnCompletionApplication::Live,
            ),
            (
                RuntimeExecutionState::Interrupted,
                TurnCompletionApplication::RecordOnly,
            ),
            (
                RuntimeExecutionState::Completed,
                TurnCompletionApplication::Superseded,
            ),
        ] {
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
        let mut active = aggregate(RuntimeExecutionState::Running);
        assert_eq!(active.orphan_interrupt(), TransitionOutcome::Applied);
        for state in [
            RuntimeExecutionState::Interrupted,
            RuntimeExecutionState::Completed,
        ] {
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
        assert_eq!(
            aggregate(RuntimeExecutionState::Running).admit_workflow_turn(true),
            TransitionOutcome::Applied
        );
        assert_eq!(
            aggregate(RuntimeExecutionState::Running).admit_workflow_turn(false),
            TransitionOutcome::Rejected(TransitionRejection::WorkflowTurnNotAuthorized)
        );
        assert_eq!(
            aggregate(RuntimeExecutionState::Running).observe_stall(),
            TransitionOutcome::Applied
        );
    }

    #[test]
    fn fanout_and_interrupt_cells_are_closed() {
        for state in [
            RuntimeExecutionState::Running,
            RuntimeExecutionState::Interrupted,
            RuntimeExecutionState::Completed,
        ] {
            let expected = if matches!(state, RuntimeExecutionState::Running) {
                TransitionOutcome::Applied
            } else {
                TransitionOutcome::Rejected(TransitionRejection::NotActive)
            };
            assert_eq!(aggregate(state.clone()).start_fanout_child(), expected);
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
}
