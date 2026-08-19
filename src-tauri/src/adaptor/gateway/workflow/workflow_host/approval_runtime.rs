//! Approval admission and session coordination.

use std::sync::Arc;

use tauri::Manager;

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

pub(crate) fn workflow_approval_auto_approve_enabled<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> bool {
    app.try_state::<Arc<dyn crate::domain::app_config::ConfigRepository>>()
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
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecution;
    use crate::domain::workflow::WorkflowDefinition;
    use crate::domain::workflow::{NodeKindName, RuntimeExecutionState, TokenUsage};
    use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

    fn commit_snapshot_fixture(node_is_waiting_approval: bool) -> RuntimeCommitSnapshot {
        RuntimeCommitSnapshot {
            execution_id: "execution-1".to_string(),
            workflow_name: "wf".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            request: "ship it".to_string(),
            error_reason: None,
            state: RuntimeExecutionState::Running,
            current_node_name: Some("approval".to_string()),
            current_session_id: None,
            node_history: Vec::new(),
            workflow_definition: WorkflowDefinition::default(),
            total_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: node_is_waiting_approval
                .then(|| RuntimeNodeExecution {
                    id: "node-execution-1".to_string(),
                    execution_id: "execution-1".to_string(),
                    node_name: "approval".to_string(),
                    kind: NodeKindName::Session,
                    attempt: 1,
                    status: RuntimeNodeExecutionStatus::WaitingApproval,
                    session_id: None,
                    display_command: None,
                    artifact: None,
                    result_summary: None,
                    token_usage: None,
                    failure: None,
                    parent: None,
                    completion_signals: Default::default(),
                    started_at: 1.0,
                    completed_at: None,
                })
                .into_iter()
                .collect(),
            started_at: 1.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn auto_approve_target_for_persisted_snapshot_builds_target() {
        let snapshot = commit_snapshot_fixture(true);

        let target = auto_approve_target_for_persisted_snapshot(&snapshot, true);

        assert_eq!(
            target,
            Some(("execution-1".to_string(), "approval".to_string()))
        );
    }

    #[test]
    fn auto_approve_target_for_persisted_snapshot_requires_waiting_and_enabled() {
        let waiting = commit_snapshot_fixture(true);
        let running = commit_snapshot_fixture(false);

        assert!(auto_approve_target_for_persisted_snapshot(&waiting, false).is_none());
        assert!(auto_approve_target_for_persisted_snapshot(&running, true).is_none());
    }

    #[test]
    fn validate_approve_comment_delegates_length_rule_to_domain() {
        let approve = validate_approve_comment(Some(&"x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1)))
            .unwrap_err();
        assert!(matches!(approve, WorkflowRuntimeError::ValidationError(_)));
    }
}
