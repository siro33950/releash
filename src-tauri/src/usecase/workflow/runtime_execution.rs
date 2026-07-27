//! Transaction preparation for workflow execution lifecycle changes.
//!
//! The aggregate owns transition validity and state mutation. This usecase owns
//! the application step: prepare a decision against an immutable snapshot, then
//! commit that exact decision at the caller's transaction boundary.

use crate::domain::workflow::entities::workflow_execution::{TransitionOutcome, WorkflowExecution};
use crate::domain::workflow::{
    ExecutionInterruptionReason, NodeExecutionFailureKind, RuntimeExecutionState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkflowExecutionCommand {
    Stop,
    Resume,
    Abort,
    RequestApproval,
    ResolveApproval,
    Complete,
    Fail {
        reason: String,
        kind: NodeExecutionFailureKind,
        retry_count: Option<u32>,
    },
    Interrupt(ExecutionInterruptionReason),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedWorkflowTransition {
    before: WorkflowExecution,
    after: WorkflowExecution,
    outcome: TransitionOutcome,
}

impl PreparedWorkflowTransition {
    pub(crate) fn commit(self, execution: &mut WorkflowExecution) -> TransitionOutcome {
        debug_assert_eq!(
            execution, &self.before,
            "prepared workflow transition must commit against its source snapshot"
        );
        *execution = self.after;
        self.outcome
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WorkflowExecutionUsecase;

impl WorkflowExecutionUsecase {
    pub(crate) fn prepare(
        execution: &WorkflowExecution,
        command: WorkflowExecutionCommand,
    ) -> PreparedWorkflowTransition {
        let before = execution.clone();
        let mut after = before.clone();
        let outcome = match command {
            WorkflowExecutionCommand::Stop => after.stop(),
            WorkflowExecutionCommand::Resume => after.resume(),
            WorkflowExecutionCommand::Abort => after.abort(),
            WorkflowExecutionCommand::RequestApproval => after.request_approval(),
            WorkflowExecutionCommand::ResolveApproval => after.resolve_approval(),
            WorkflowExecutionCommand::Complete => after.complete(),
            WorkflowExecutionCommand::Fail {
                reason,
                kind,
                retry_count,
            } => after.fail(reason, kind, retry_count),
            WorkflowExecutionCommand::Interrupt(reason) => after.interrupt(reason),
        };
        PreparedWorkflowTransition {
            before,
            after,
            outcome,
        }
    }

    pub(crate) fn restore(
        state: RuntimeExecutionState,
        interruption_reason: Option<ExecutionInterruptionReason>,
    ) -> WorkflowExecution {
        WorkflowExecution::restore(state, interruption_reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::{
        TransitionOutcome, TransitionRejection,
    };

    #[test]
    fn preparation_is_side_effect_free_until_commit() {
        let mut execution = WorkflowExecution::start();
        let prepared =
            WorkflowExecutionUsecase::prepare(&execution, WorkflowExecutionCommand::Stop);

        assert_eq!(execution.state(), &RuntimeExecutionState::Running);
        assert_eq!(prepared.commit(&mut execution), TransitionOutcome::Applied);
        assert_eq!(execution.state(), &RuntimeExecutionState::Interrupted);
        assert_eq!(
            execution.interruption_reason(),
            Some(ExecutionInterruptionReason::Stop)
        );
    }

    #[test]
    fn rejected_preparation_commits_no_state_change() {
        let mut execution = WorkflowExecution::start();
        let prepared =
            WorkflowExecutionUsecase::prepare(&execution, WorkflowExecutionCommand::Resume);

        let before = execution.clone();
        assert_eq!(
            prepared.commit(&mut execution),
            TransitionOutcome::Rejected(TransitionRejection::NotResumable)
        );
        assert_eq!(execution, before);
    }
}
