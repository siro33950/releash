//! Gateway bridge for execution aggregates and commit snapshots.
//!
//! Mutable execution state and transition decisions live in the domain
//! aggregate. This module bridges driver decisions to the gateway commit DTO.

pub(crate) use crate::domain::workflow::entities::workflow_execution::ExecutionTree as DomainExecutionTree;
macro_rules! domain_workflow_execution {
    ($($fields:tt)*) => {
        $crate::domain::workflow::entities::workflow_execution::ExecutionTree::restore_runtime(
            $crate::domain::workflow::entities::workflow_execution::ExecutionTreeRestore {
                $($fields)*
            },
        )
    };
}
pub(crate) use domain_workflow_execution;
