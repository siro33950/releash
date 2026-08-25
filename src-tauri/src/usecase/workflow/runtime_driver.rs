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

use crate::domain::workflow::entities::workflow_execution::{
    ExecutionAdvanceDecision, LeafStart, TransitionOutcome, WorkflowExecution,
};
use crate::domain::workflow::WorkflowEvent;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

pub(crate) enum NodeOutcome {
    /// 起動すべき runtime は無い（完了・承認待ち・並走子待ち）。
    Persist(RuntimeCommitSnapshot),
    /// 起動すべき leaf 群。
    StartLeaves(RuntimeCommitSnapshot, Vec<LeafStart>),
}

impl NodeOutcome {
    pub(crate) fn snapshot(&self) -> &RuntimeCommitSnapshot {
        match self {
            Self::Persist(snapshot) | Self::StartLeaves(snapshot, _) => snapshot,
        }
    }
}

pub(crate) fn node_outcome_from_advance(
    execution: &WorkflowExecution,
    decision: ExecutionAdvanceDecision,
) -> Result<NodeOutcome, crate::usecase::workflow::runtime_error::WorkflowRuntimeError> {
    let snapshot = RuntimeCommitSnapshot::from_execution(execution)?;
    Ok(match decision {
        ExecutionAdvanceDecision::Persist => NodeOutcome::Persist(snapshot),
        ExecutionAdvanceDecision::StartLeaves(leaves) => NodeOutcome::StartLeaves(snapshot, leaves),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkflowRuntimeEffect {
    BroadcastState,
    StopWorkflowAgentSession {
        node_execution_id: String,
        agent_session_id: String,
    },
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
    /// the durable commit and publication boundary.
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
        mut decision: WorkflowRuntimeDecision,
    ) -> Result<Self, WorkflowTransactionPreparationError> {
        if before == after
            && decision.outcome == TransitionOutcome::Applied
            && !decision.events.is_empty()
        {
            return Err(WorkflowTransactionPreparationError::EventWithoutStateChange);
        }
        decision.effects.extend(
            after
                .newly_terminal_sessions_since(&before)
                .into_iter()
                .map(|target| WorkflowRuntimeEffect::StopWorkflowAgentSession {
                    node_execution_id: target.node_execution_id,
                    agent_session_id: target.agent_session_id,
                }),
        );
        Ok(Self {
            before,
            after,
            decision,
        })
    }

    pub(crate) fn events(&self) -> &[WorkflowEvent] {
        &self.decision.events
    }

    /// Persists canonical facts and publishes the aggregate candidate only
    /// after persistence succeeds. Effects stay inaccessible until then.
    pub(crate) fn persist<E, P>(
        self,
        current: &mut WorkflowExecution,
        persist: P,
    ) -> Result<DurableWorkflowTransaction, WorkflowTransactionCommitError<E>>
    where
        P: FnOnce(&[WorkflowEvent]) -> Result<(), E>,
    {
        if current != &self.before {
            return Err(WorkflowTransactionCommitError::StaleCandidate);
        }
        persist(self.events()).map_err(WorkflowTransactionCommitError::Persistence)?;
        *current = self.after;
        Ok(DurableWorkflowTransaction {
            #[cfg(test)]
            outcome: self.decision.outcome,
            effects: self.decision.effects,
        })
    }

    pub(crate) async fn persist_async<E, P, Fut>(
        self,
        current: &mut WorkflowExecution,
        persist: P,
    ) -> Result<DurableWorkflowTransaction, WorkflowTransactionCommitError<E>>
    where
        P: FnOnce(Vec<WorkflowEvent>) -> Fut,
        Fut: std::future::Future<Output = Result<(), E>>,
    {
        if current != &self.before {
            return Err(WorkflowTransactionCommitError::StaleCandidate);
        }
        persist(self.decision.events.clone())
            .await
            .map_err(WorkflowTransactionCommitError::Persistence)?;
        *current = self.after;
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
    use crate::domain::workflow::{
        ExecutionInterruptionReason, NodeExecutionFailureKind, NodeKindName, RuntimeExecutionState,
    };

    fn execution_with_attached_session() -> WorkflowExecution {
        let mut execution = WorkflowExecution::restore(RuntimeExecutionState::Running, None);
        execution
            .begin_node_attempt(
                "session".to_string(),
                NodeKindName::Session,
                1,
                None,
                "node-execution".to_string(),
                1.0,
            )
            .unwrap();
        execution.attach_node_session("node-execution", "agent-session".to_string(), 1.0);
        execution
    }

    fn interrupted_event() -> WorkflowEvent {
        WorkflowEvent::ExecutionInterrupted {
            execution_id: "execution".into(),
            reason: ExecutionInterruptionReason::Stop,
            timestamp: 1.0,
        }
    }

    #[test]
    fn persistence_failure_keeps_exact_pre_commit_aggregate_and_releases_no_effects() {
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
        let durable = prepared.persist(&mut live, |_| Ok::<_, ()>(())).unwrap();

        assert_eq!(durable.outcome(), TransitionOutcome::Applied);
        assert_eq!(
            durable.into_effects(),
            vec![WorkflowRuntimeEffect::BroadcastState]
        );
        assert_eq!(live.state(), &RuntimeExecutionState::Interrupted);
    }

    #[test]
    fn newly_terminal_session_stop_effect_becomes_available_after_persistence() {
        let mut live = execution_with_attached_session();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.fail_node_execution(
                "node-execution",
                "provider failed".to_string(),
                NodeExecutionFailureKind::InfrastructureCrash,
                2.0,
            );
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![interrupted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();

        let durable = prepared.persist(&mut live, |_| Ok::<_, ()>(())).unwrap();

        assert_eq!(
            durable.into_effects(),
            vec![
                WorkflowRuntimeEffect::BroadcastState,
                WorkflowRuntimeEffect::StopWorkflowAgentSession {
                    node_execution_id: "node-execution".to_string(),
                    agent_session_id: "agent-session".to_string(),
                },
            ]
        );
    }

    #[test]
    fn newly_terminal_session_persistence_failure_keeps_active_aggregate() {
        let mut live = execution_with_attached_session();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.fail_node_execution(
                "node-execution",
                "provider failed".to_string(),
                NodeExecutionFailureKind::InfrastructureCrash,
                2.0,
            );
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![interrupted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();

        let result = prepared.persist(&mut live, |_| Err("disk"));

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::Persistence("disk"))
        ));
        assert!(live
            .node_execution("node-execution")
            .unwrap()
            .status
            .is_active());
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

    #[tokio::test]
    async fn persist_async_updates_current_only_after_persistence_succeeds() {
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

        let durable = prepared
            .persist_async(&mut live, |_| async { Ok::<_, ()>(()) })
            .await
            .unwrap();

        assert_eq!(live.state(), &RuntimeExecutionState::Interrupted);
        assert_eq!(durable.outcome(), TransitionOutcome::Applied);
        assert_eq!(
            durable.into_effects(),
            vec![WorkflowRuntimeEffect::BroadcastState]
        );
    }

    #[tokio::test]
    async fn persist_async_rejects_stale_candidate_without_persistence() {
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
        let persisted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = persisted.clone();

        let result = prepared
            .persist_async(&mut stale, move |_| async move {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<_, ()>(())
            })
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::StaleCandidate)
        ));
        assert!(!persisted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn persist_async_propagates_persistence_failure_without_updating_current() {
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

        let result = prepared
            .persist_async(&mut live, |_| async { Err("disk") })
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::Persistence("disk"))
        ));
        assert_eq!(live, before);
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
