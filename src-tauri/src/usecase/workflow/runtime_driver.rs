//! Workflow runtime transaction procedure.
//!
//! Domain methods decide whether an observation is admissible and create the
//! next aggregate state.  This use-case owns the ordering invariant around
//! that decision:
//!
//! 1. apply an observation to an isolated aggregate candidate;
//! 2. persist the candidate's canonical facts;
//! 3. publish the candidate as the live aggregate;
//! 4. only then release external effects.
//!
//! Concrete event stores, agent runtimes, process handles, and notification
//! transports implement the closures/ports consumed here.

use crate::domain::workflow::entities::workflow_execution::{TransitionOutcome, WorkflowExecution};
use crate::domain::workflow::WorkflowEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkflowRuntimeEffect {
    BroadcastState,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkflowRuntimeDecision {
    pub(crate) outcome: TransitionOutcome,
    pub(crate) events: Vec<WorkflowEvent>,
    pub(crate) effects: Vec<WorkflowRuntimeEffect>,
}

impl WorkflowRuntimeDecision {
    pub(crate) fn applied(events: Vec<WorkflowEvent>, effects: Vec<WorkflowRuntimeEffect>) -> Self {
        Self {
            outcome: TransitionOutcome::Applied,
            events,
            effects,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkflowTransactionPreparationError {
    EventWithoutStateChange,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WorkflowTransactionCommitError<E> {
    StaleCandidate,
    Persistence(E),
}

/// A decision prepared against an exact aggregate revision.
///
/// The before/after snapshots remain private so an outer layer cannot publish
/// the candidate without crossing `persist`.
pub(crate) struct PreparedWorkflowTransaction {
    before: WorkflowExecution,
    after: WorkflowExecution,
    decision: WorkflowRuntimeDecision,
}

impl PreparedWorkflowTransaction {
    /// Applies a live observation to a clone of the aggregate.
    #[cfg(test)]
    pub(crate) fn observe<F>(
        execution: &WorkflowExecution,
        observe: F,
    ) -> Result<Self, WorkflowTransactionPreparationError>
    where
        F: FnOnce(
            &mut WorkflowExecution,
        ) -> Result<WorkflowRuntimeDecision, WorkflowTransactionPreparationError>,
    {
        let before = execution.clone();
        let mut after = before.clone();
        let decision = observe(&mut after)?;
        Self::from_candidate(before, after, decision)
    }

    /// Captures a candidate already produced under the runtime registry lock.
    ///
    /// This is used during migration of legacy callers: mutation validity still
    /// comes exclusively from aggregate methods, while this use-case takes over
    /// the durable commit/rollback boundary.
    pub(crate) fn capture_applied(
        before: WorkflowExecution,
        after: WorkflowExecution,
        events: Vec<WorkflowEvent>,
        effects: Vec<WorkflowRuntimeEffect>,
    ) -> Result<Self, WorkflowTransactionPreparationError> {
        Self::from_candidate(
            before,
            after,
            WorkflowRuntimeDecision::applied(events, effects),
        )
    }

    fn from_candidate(
        before: WorkflowExecution,
        after: WorkflowExecution,
        decision: WorkflowRuntimeDecision,
    ) -> Result<Self, WorkflowTransactionPreparationError> {
        if before == after
            && decision.outcome == TransitionOutcome::Applied
            && !decision.events.is_empty()
        {
            return Err(WorkflowTransactionPreparationError::EventWithoutStateChange);
        }
        Ok(Self {
            before,
            after,
            decision,
        })
    }

    pub(crate) fn events(&self) -> &[WorkflowEvent] {
        &self.decision.events
    }

    /// Persists canonical facts and atomically publishes or rolls back the
    /// aggregate candidate. Effects stay inaccessible until this succeeds.
    pub(crate) fn persist<E, P>(
        self,
        current: &mut WorkflowExecution,
        persist: P,
    ) -> Result<DurableWorkflowTransaction, WorkflowTransactionCommitError<E>>
    where
        P: FnOnce(&[WorkflowEvent]) -> Result<(), E>,
    {
        if current != &self.after {
            return Err(WorkflowTransactionCommitError::StaleCandidate);
        }
        if let Err(error) = persist(self.events()) {
            *current = self.before;
            return Err(WorkflowTransactionCommitError::Persistence(error));
        }
        Ok(DurableWorkflowTransaction {
            #[cfg(test)]
            outcome: self.decision.outcome,
            effects: self.decision.effects,
        })
    }
}

/// Proof that canonical facts are durable.
pub(crate) struct DurableWorkflowTransaction {
    #[cfg(test)]
    outcome: TransitionOutcome,
    effects: Vec<WorkflowRuntimeEffect>,
}

impl DurableWorkflowTransaction {
    #[cfg(test)]
    pub(crate) fn outcome(&self) -> TransitionOutcome {
        self.outcome.clone()
    }

    pub(crate) fn into_effects(self) -> Vec<WorkflowRuntimeEffect> {
        self.effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::TransitionRejection;
    use crate::domain::workflow::{ExecutionInterruptionReason, RuntimeExecutionState};

    fn interrupted_event() -> WorkflowEvent {
        WorkflowEvent::ExecutionInterrupted {
            execution_id: "execution".into(),
            reason: ExecutionInterruptionReason::Stop,
            timestamp: 1.0,
        }
    }

    #[test]
    fn persistence_failure_restores_exact_aggregate_and_releases_no_effects() {
        let mut live = WorkflowExecution::restore(RuntimeExecutionState::Running, None);
        let before = live.clone();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.stop();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![interrupted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();
        live.stop();

        let result = prepared.persist(&mut live, |_| Err("disk"));

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::Persistence("disk"))
        ));
        assert_eq!(live, before);
    }

    #[test]
    fn effects_become_available_only_after_durable_persistence() {
        let mut live = WorkflowExecution::restore(RuntimeExecutionState::Running, None);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.stop();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![interrupted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();
        live.stop();

        let durable = prepared.persist(&mut live, |_| Ok::<_, ()>(())).unwrap();

        assert_eq!(durable.outcome(), TransitionOutcome::Applied);
        assert_eq!(
            durable.into_effects(),
            vec![WorkflowRuntimeEffect::BroadcastState]
        );
        assert_eq!(live.state(), &RuntimeExecutionState::Interrupted);
    }

    #[test]
    fn stale_candidate_is_rejected_without_persistence() {
        let live = WorkflowExecution::restore(RuntimeExecutionState::Running, None);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.stop();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![interrupted_event()],
                effects: Vec::new(),
            })
        })
        .unwrap();
        let mut stale = WorkflowExecution::restore(RuntimeExecutionState::Running, None);
        stale.abort();
        let mut persisted = false;

        let result = prepared.persist(&mut stale, |_| {
            persisted = true;
            Ok::<_, ()>(())
        });

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::StaleCandidate)
        ));
        assert!(!persisted);
    }

    #[test]
    fn aggregate_rejection_is_preserved_as_a_typed_decision() {
        let live = WorkflowExecution::restore(RuntimeExecutionState::Running, None);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            Ok(WorkflowRuntimeDecision {
                outcome: candidate.resume(),
                events: Vec::new(),
                effects: Vec::new(),
            })
        })
        .unwrap();
        let mut candidate = live.clone();

        let durable = prepared
            .persist(&mut candidate, |_| Ok::<_, ()>(()))
            .unwrap();

        assert_eq!(
            durable.outcome(),
            TransitionOutcome::Rejected(TransitionRejection::NotResumable)
        );
    }
}
