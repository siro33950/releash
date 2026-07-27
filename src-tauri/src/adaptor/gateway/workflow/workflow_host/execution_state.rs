//! Gateway bridge for retained execution aggregates and commit snapshots.
//!
//! Mutable execution state and transition decisions live in the domain
//! aggregate. This module bridges driver decisions to the gateway commit DTO.

use crate::adaptor::gateway::workflow::workflow_host::runtime_commit::NodeOutcome;
use crate::adaptor::gateway::workflow::workflow_host::runtime_start_guard;
use crate::domain::workflow::entities::workflow_execution::{
    ExecutionAdvanceDecision, WorkflowExecution as WorkflowExecutionAggregate,
};
use crate::domain::workflow::services::projection as workflow_projection;
use crate::domain::workflow::RuntimeExecutionState;
use crate::domain::workflow::WorkflowDefinition;
use crate::usecase::agent_session::status::current_timestamp;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

pub(crate) use crate::domain::workflow::entities::workflow_execution::{
    FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState,
    WorkflowExecution as DomainWorkflowExecution,
};
#[cfg(test)]
pub(crate) use crate::domain::workflow::entities::workflow_execution::{
    LoopGuardResult, NextNodeDecision, TurnCompleteAction,
};

macro_rules! domain_workflow_execution {
    ($($fields:tt)*) => {
        $crate::domain::workflow::entities::workflow_execution::WorkflowExecution::restore_runtime(
            $crate::domain::workflow::entities::workflow_execution::WorkflowExecutionRestore {
                $($fields)*
            },
        )
    };
}
pub(crate) use domain_workflow_execution;

/// session_id → execution_id reverse index value.
#[derive(Clone)]
pub(crate) struct SessionWorkflowRef {
    pub(crate) execution_id: String,
}

impl WorkflowExecutionAggregate {
    pub(crate) fn is_terminal(&self) -> bool {
        self.is_finished()
    }

    pub(crate) fn validate_start(
        workflow: &WorkflowDefinition,
        existing: Option<&WorkflowExecutionAggregate>,
    ) -> Result<(), WorkflowRuntimeError> {
        let existing_active_workflow_name = existing
            .filter(|existing| existing.is_active())
            .map(|existing| existing.workflow.name.as_str());
        runtime_start_guard::validate_start(workflow, existing_active_workflow_name)
    }

    pub(crate) fn to_commit_snapshot(&self) -> RuntimeCommitSnapshot {
        RuntimeCommitSnapshot {
            execution_id: self.id.clone(),
            workflow_name: self.workflow.name.clone(),
            worktree_path: self.worktree_path.clone(),
            created_from: self.created_from,
            request: self.request.clone().unwrap_or_default(),
            error_reason: match self.state() {
                RuntimeExecutionState::Failed { reason, .. } => Some(reason.clone()),
                RuntimeExecutionState::Interrupted => self.error_reason.clone(),
                _ => None,
            },
            state: self.state().clone(),
            current_node_index: self.current_node_index,
            current_node_name: self.workflow.nodes[self.current_node_index].name.clone(),
            current_session_id: self.current_session_id.clone(),
            node_history: self.node_history.clone(),
            node_execution_counts: self.node_execution_counts.clone(),
            workflow_definition: self.workflow.clone(),
            total_token_usage: workflow_projection::total_token_usage(&self.node_history),
            artifacts: self.artifacts.clone(),
            node_executions: self.node_executions.clone(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    pub(crate) fn make_node_history_entry(
        &mut self,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
    ) -> crate::domain::workflow::NodeHistoryEntry {
        self.make_node_history_entry_at(result, artifact, contract, current_timestamp())
    }

    pub(crate) fn apply_advance(&mut self) -> NodeOutcome {
        match self.apply_advance_at(current_timestamp()) {
            ExecutionAdvanceDecision::Persist => NodeOutcome::Persist(self.to_commit_snapshot()),
            ExecutionAdvanceDecision::TransitionAndStart => {
                NodeOutcome::TransitionAndStart(self.to_commit_snapshot())
            }
            ExecutionAdvanceDecision::StartFanout => {
                NodeOutcome::StartFanout(self.to_commit_snapshot())
            }
        }
    }

    pub(crate) fn retry_current_node(&mut self) -> NodeOutcome {
        let decision = self.retry_current_node_at(current_timestamp());
        NodeOutcome::RetryCurrentNode {
            snapshot: self.to_commit_snapshot(),
            completed_session_id: decision.completed_session_id,
        }
    }
}
