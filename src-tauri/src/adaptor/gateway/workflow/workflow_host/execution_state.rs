//! Gateway bridge for retained execution aggregates and commit snapshots.
//!
//! Mutable execution state and transition decisions live in the domain
//! aggregate. This module bridges driver decisions to the gateway commit DTO.

use crate::domain::workflow::entities::workflow_execution::WorkflowExecution as WorkflowExecutionAggregate;
use crate::domain::workflow::WorkflowDefinition;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::runtime_start_guard;

pub(crate) use crate::domain::workflow::entities::workflow_execution::{
    FanoutChildRuntime, FanoutChildRuntimeState, FanoutRuntimeState,
    WorkflowExecution as DomainWorkflowExecution,
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
}
