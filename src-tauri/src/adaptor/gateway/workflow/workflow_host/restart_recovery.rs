//! Canonical event-log hydration for active Workflow restart reconciliation.
//!
//! 事実ログを集約へ replay した実行木（`ActiveRestartProjection.aggregate`）を
//! そのまま live registry へ戻す。復旧専用の再構築経路は持たない。

use super::resume_projection::ActiveRestartProjection;
use super::*;

fn invalid(message: impl Into<String>) -> WorkflowRuntimeError {
    WorkflowRuntimeError::InvalidState(message.into())
}

pub(super) fn hydrate_restart_execution(
    checkpoint: &ActiveRestartProjection,
) -> Result<DomainWorkflowExecution, WorkflowRuntimeError> {
    match checkpoint.projected_execution.status {
        crate::domain::workflow::ExecutionStatus::Running => {}
        other => {
            return Err(invalid(format!(
                "restart reconciliation cannot hydrate workflow status {}",
                other.as_str()
            )));
        }
    }
    let has_active_node = checkpoint
        .aggregate
        .node_executions
        .iter()
        .any(|node| node.status.is_active());
    if !has_active_node {
        return Err(invalid("restart reconciliation has no active node attempt"));
    }
    Ok(checkpoint.aggregate.clone())
}
