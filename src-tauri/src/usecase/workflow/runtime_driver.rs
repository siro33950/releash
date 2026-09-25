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
    ExecutionAdvanceDecision, ExecutionTree, NodeStart, TransitionOutcome,
};
use crate::domain::workflow::WorkflowEvent;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

#[derive(Clone)]
pub(crate) enum NodeOutcome {
    /// 起動すべき runtime は無い（完了・承認待ち・並走子待ち）。
    Persist,
    /// 合成子の準備要求と葉 runtime の起動要求。
    StartNodes(Box<RuntimeCommitSnapshot>, Vec<NodeStart>),
}

pub(crate) fn node_outcome_from_advance(
    execution: &ExecutionTree,
    decision: ExecutionAdvanceDecision,
) -> Result<NodeOutcome, crate::usecase::workflow::runtime_error::WorkflowRuntimeError> {
    Ok(match decision {
        ExecutionAdvanceDecision::Persist => NodeOutcome::Persist,
        ExecutionAdvanceDecision::StartNodes(leaves) => NodeOutcome::StartNodes(
            Box::new(RuntimeCommitSnapshot::from_execution(execution)?),
            leaves,
        ),
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
    before: ExecutionTree,
    after: ExecutionTree,
    decision: WorkflowRuntimeDecision,
}

impl PreparedWorkflowTransaction {
    /// Applies a live observation to a clone of the aggregate.
    #[cfg(test)]
    pub(crate) fn observe<F>(
        execution: &ExecutionTree,
        observe: F,
    ) -> Result<Self, WorkflowTransactionPreparationError>
    where
        F: FnOnce(
            &mut ExecutionTree,
        ) -> Result<WorkflowRuntimeDecision, WorkflowTransactionPreparationError>,
    {
        let before = execution.clone();
        let mut after = before.clone();
        let decision = observe(&mut after)?;
        Self::from_candidate(before, after, decision)
    }

    pub(crate) fn capture_with_outcome(
        before: ExecutionTree,
        after: ExecutionTree,
        outcome: TransitionOutcome,
        events: Vec<WorkflowEvent>,
        effects: Vec<WorkflowRuntimeEffect>,
    ) -> Result<Self, WorkflowTransactionPreparationError> {
        Self::from_candidate(
            before,
            after,
            WorkflowRuntimeDecision {
                outcome,
                events,
                effects,
            },
        )
    }

    fn from_candidate(
        before: ExecutionTree,
        after: ExecutionTree,
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

    /// Persists canonical facts and publishes the aggregate candidate only
    /// after persistence succeeds. Effects stay inaccessible until then.
    pub(crate) async fn persist<E, P, Fut>(
        self,
        current: &mut ExecutionTree,
        persist: P,
    ) -> Result<DurableWorkflowTransaction, WorkflowTransactionCommitError<E>>
    where
        P: FnOnce(Vec<WorkflowEvent>) -> Fut,
        Fut: std::future::Future<Output = Result<(), E>>,
    {
        if current != &self.before {
            return Err(WorkflowTransactionCommitError::StaleCandidate);
        }
        persist(self.decision.events)
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
    use crate::domain::workflow::{NodeKindName, RuntimeExecutionState};

    fn execution_with_attached_session() -> ExecutionTree {
        let mut execution = ExecutionTree::restore(RuntimeExecutionState::Running);
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

    fn aborted_event() -> WorkflowEvent {
        WorkflowEvent::ExecutionAborted {
            execution_id: "execution".into(),
            aborted_node: None,
            timestamp: 1.0,
        }
    }

    #[tokio::test]
    async fn persistence_failure_keeps_exact_pre_commit_aggregate_and_releases_no_effects() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let before = live.clone();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();
        let result = prepared
            .persist(&mut live, |_| std::future::ready(Err("disk")))
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::Persistence("disk"))
        ));
        assert_eq!(live, before);
    }

    #[tokio::test]
    async fn effects_become_available_only_after_durable_persistence() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();
        let durable = prepared
            .persist(&mut live, |_| std::future::ready(Ok::<_, ()>(())))
            .await
            .unwrap();

        assert_eq!(durable.outcome(), TransitionOutcome::Applied);
        assert_eq!(
            durable.into_effects(),
            vec![WorkflowRuntimeEffect::BroadcastState]
        );
        assert_eq!(live.state(), &RuntimeExecutionState::Aborted);
    }

    #[tokio::test]
    async fn already_applied_observation_persists_event_without_changing_aggregate() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let before = live.clone();
        let event = aborted_event();
        let prepared = PreparedWorkflowTransaction::capture_with_outcome(
            before.clone(),
            before.clone(),
            TransitionOutcome::AlreadyApplied,
            vec![event.clone()],
            Vec::new(),
        )
        .unwrap();
        let mut persisted = Vec::new();

        let durable = prepared
            .persist(&mut live, |events| {
                persisted.extend(events);
                std::future::ready(Ok::<_, ()>(()))
            })
            .await
            .unwrap();

        assert_eq!(durable.outcome(), TransitionOutcome::AlreadyApplied);
        assert_eq!(persisted, vec![event]);
        assert_eq!(live, before);
    }

    #[tokio::test]
    async fn newly_terminal_session_stop_effect_becomes_available_after_persistence() {
        let mut live = execution_with_attached_session();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort_node_execution("node-execution", 2.0);
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();

        let durable = prepared
            .persist(&mut live, |_| std::future::ready(Ok::<_, ()>(())))
            .await
            .unwrap();

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

    #[tokio::test]
    async fn newly_terminal_session_persistence_failure_keeps_active_aggregate() {
        let mut live = execution_with_attached_session();
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort_node_execution("node-execution", 2.0);
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: vec![WorkflowRuntimeEffect::BroadcastState],
            })
        })
        .unwrap();

        let result = prepared
            .persist(&mut live, |_| std::future::ready(Err("disk")))
            .await;

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

    #[tokio::test]
    async fn stale_candidate_is_rejected_without_persistence() {
        let live = ExecutionTree::restore(RuntimeExecutionState::Running);
        let prepared = PreparedWorkflowTransaction::observe(&live, |candidate| {
            let outcome = candidate.abort();
            Ok(WorkflowRuntimeDecision {
                outcome,
                events: vec![aborted_event()],
                effects: Vec::new(),
            })
        })
        .unwrap();
        let mut stale = ExecutionTree::restore(RuntimeExecutionState::Running);
        stale.abort();
        let mut persisted = false;

        let result = prepared
            .persist(&mut stale, |_| {
                persisted = true;
                std::future::ready(Ok::<_, ()>(()))
            })
            .await;

        assert!(matches!(
            result,
            Err(WorkflowTransactionCommitError::StaleCandidate)
        ));
        assert!(!persisted);
    }

    #[tokio::test]
    async fn aggregate_rejection_is_preserved_as_a_typed_decision() {
        let mut live = ExecutionTree::restore(RuntimeExecutionState::Completed);
        let rejection = match live.replay_started() {
            crate::domain::workflow::entities::workflow_execution::ReplayOutcome::Rejected(
                reason,
            ) => reason,
            outcome => panic!("unexpected replay outcome: {outcome:?}"),
        };
        let prepared = PreparedWorkflowTransaction::observe(&live, |_candidate| {
            Ok(WorkflowRuntimeDecision {
                outcome: TransitionOutcome::Rejected(rejection),
                events: Vec::new(),
                effects: Vec::new(),
            })
        })
        .unwrap();
        let mut candidate = live.clone();

        let durable = prepared
            .persist(&mut candidate, |_| std::future::ready(Ok::<_, ()>(())))
            .await
            .unwrap();

        assert_eq!(
            durable.outcome(),
            TransitionOutcome::Rejected(TransitionRejection::NotActive)
        );
    }
}
