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
pub enum NodeOutcome {
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
pub enum WorkflowRuntimeEffect {
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
#[path = "runtime_driver_test.rs"]
mod runtime_driver_tests;
