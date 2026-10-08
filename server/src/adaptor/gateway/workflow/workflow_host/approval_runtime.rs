//! Approval admission and session coordination.

use super::WorkflowRuntimeDependencies;

#[cfg(test)]
use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus;
#[cfg(test)]
use crate::domain::workflow::services::approval_rules as workflow_approval;
#[cfg(test)]
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
#[cfg(test)]
use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

#[cfg(test)]
pub(crate) const MAX_APPROVAL_COMMENT_CHARS: usize = workflow_approval::MAX_APPROVAL_COMMENT_CHARS;

#[cfg(test)]
pub(crate) fn validate_approve_comment(comment: Option<&str>) -> Result<(), WorkflowRuntimeError> {
    workflow_approval::validate_optional_comment_text(comment, "Approve comment")
        .map_err(|err| WorkflowRuntimeError::ValidationError(err.to_string()))
}

pub(crate) fn workflow_approval_auto_approve_enabled(app: &WorkflowRuntimeDependencies) -> bool {
    app.config
        .as_ref()
        .and_then(|config| config.load().ok())
        .is_some_and(|cfg| cfg.workflow.approval_auto_approve)
}

#[cfg(test)]
pub(crate) fn should_auto_approve_workflow_approval(
    snapshot: &RuntimeCommitSnapshot,
    approval_auto_approve_enabled: bool,
) -> bool {
    workflow_approval::should_auto_approve_workflow_approval(
        snapshot
            .node_executions
            .iter()
            .any(|execution| execution.status == RuntimeNodeExecutionStatus::WaitingApproval),
        approval_auto_approve_enabled,
    )
}

#[cfg(test)]
pub(crate) fn auto_approve_target_for_persisted_snapshot(
    snapshot: &RuntimeCommitSnapshot,
    approval_auto_approve_enabled: bool,
) -> Option<(String, String)> {
    if should_auto_approve_workflow_approval(snapshot, approval_auto_approve_enabled) {
        Some((
            snapshot.execution_id.clone(),
            snapshot.current_node_name.clone()?,
        ))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "approval_runtime_test.rs"]
mod approval_runtime_tests;
