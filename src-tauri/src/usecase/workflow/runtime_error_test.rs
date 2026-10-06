use super::*;

#[test]
fn test_版競合_業務上の競合だけを返す() {
    // Given
    let conflict = WorkflowRuntimeError::Conflict("head advanced".into());
    let storage = WorkflowRuntimeError::Store(
        crate::domain::failure::StorageFailure::from(
            crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown,
        )
        .with_message("append outcome unknown"),
    );
    // When / Then
    assert_eq!(conflict.version_conflict(), Some("head advanced"));
    assert_eq!(storage.version_conflict(), None);
    let stored_conflict = WorkflowRuntimeError::storage(
        crate::domain::workflow::WorkflowError::Conflict("head advanced".into()),
        "reconcile",
    );
    assert_eq!(stored_conflict.version_conflict(), Some("head advanced"));
    let stored_commit_conflict = WorkflowRuntimeError::Store(
        crate::domain::failure::StorageFailure::from(
            crate::domain::local_event::CommitBatchError::TreeHeadConflict,
        )
        .with_message("creation failed"),
    );
    assert_eq!(
        stored_commit_conflict.version_conflict(),
        Some("creation failed")
    );
}

#[test]
fn test_node事実追記_分類と失敗理由と実行木の失敗種別を保持する() {
    // Given
    for failure in [
        crate::domain::local_event::CommitBatchError::QueueBusy.into(),
        crate::domain::local_event::CommitBatchError::Corrupt {
            correlation_id: "id".into(),
        }
        .into(),
        crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
            message: "timeout".into(),
        }
        .into(),
        crate::domain::local_event::CommitBatchError::AppendOutcomeUnknown.into(),
    ] {
        let failure: crate::domain::failure::StorageFailure = failure;
        let expected = failure.with_message("append failed");
        let error = WorkflowRuntimeError::Store(expected.clone());
        // Then
        assert!(matches!(&error, WorkflowRuntimeError::Store(actual) if *actual == expected));
        assert_eq!(error.to_string(), "append failed");
        assert_eq!(
            error.workflow_failure_kind(),
            crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash
        );
    }
}

#[test]
fn test_managed_worktree停止_runtime境界で分類を保持する() {
    use crate::common::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error =
            WorkflowRuntimeError::from(ManagedWorktreeResolverError::Technical(stopped.into()));
        // Then
        assert!(
            matches!(error, WorkflowRuntimeError::Technical(ref actual) if *actual == stopped.into())
        );
        assert_eq!(error.to_string(), stopped.to_string());
    }
}
pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn workflow_failure_kind_only_uses_crash_for_storage_or_process_loss() {
        let validation_errors = [
            WorkflowRuntimeError::InvalidWorkflow("missing facet".to_string()),
            WorkflowRuntimeError::ValidationError("bad output".to_string()),
            WorkflowRuntimeError::InvalidState("not accepting output".to_string()),
            WorkflowRuntimeError::UnauthorizedApprovalTarget("wrong execution".to_string()),
        ];
        for error in validation_errors {
            assert_eq!(
                error.workflow_failure_kind(),
                NodeExecutionFailureKind::ValidationFailure,
                "unexpected failure kind for {error:?}"
            );
        }

        assert_eq!(
            WorkflowRuntimeError::SessionStore("io".to_string()).workflow_failure_kind(),
            NodeExecutionFailureKind::InfrastructureCrash
        );
        assert_eq!(
            WorkflowRuntimeError::AgentSession("admission rejected".to_string())
                .workflow_failure_kind(),
            NodeExecutionFailureKind::ValidationFailure
        );
    }
}
