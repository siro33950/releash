//! Pure event-log projection used by `ResumeExecution`.
//!
//! `NodeCompleted` is the only confirmation boundary. An artifact emitted by an unfinished node
//! is intentionally ignored, while completed fanout children are retained by their stable
//! parent/item/child coordinates.

use crate::domain::workflow::entities::workflow_execution::WorkflowExecution as WorkflowExecutionAggregate;
use crate::domain::workflow::services::event_replay::project_retained_workflow_execution;
use crate::domain::workflow::WorkflowEvent;
use crate::domain::workflow::WorkflowExecution as DomainWorkflowExecution;

/// Canonical active execution shape used for application restart reconciliation.
/// This deliberately carries only
/// event-projected state; no runtime-local session/command state is trusted.
#[derive(Debug, Clone)]
pub(crate) struct ActiveRestartProjection {
    /// 事実ログを集約へ replay した実行木そのもの。
    pub(crate) aggregate: WorkflowExecutionAggregate,
    pub(crate) projected_execution: DomainWorkflowExecution,
}

pub(crate) fn project_restart_checkpoint(
    execution_id: &str,
    events: &[WorkflowEvent],
) -> Result<ActiveRestartProjection, String> {
    let projection = project_retained_workflow_execution(execution_id, events)?
        .ok_or_else(|| format!("execution {execution_id} has no execution_started event"))?;
    Ok(ActiveRestartProjection {
        aggregate: projection.aggregate,
        projected_execution: projection.execution,
    })
}
