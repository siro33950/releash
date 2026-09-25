use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            WorkflowRuntimeError::ExecutionNotFound("reason".into()),
            F::Missing,
        ),
        (
            WorkflowRuntimeError::SessionNotFound("reason".into()),
            F::Missing,
        ),
        (
            WorkflowRuntimeError::InvalidWorkflow("reason".into()),
            F::InvalidInput,
        ),
        (
            WorkflowRuntimeError::InvalidState("reason".into()),
            F::StateRequired,
        ),
        (
            WorkflowRuntimeError::Conflict("reason".into()),
            F::RestartRequired,
        ),
        (
            WorkflowRuntimeError::ValidationError("reason".into()),
            F::InvalidInput,
        ),
        (
            WorkflowRuntimeError::UnauthorizedWorktree("reason".into()),
            F::Permission,
        ),
        (
            WorkflowRuntimeError::UnauthorizedApprovalTarget("reason".into()),
            F::Permission,
        ),
        (
            WorkflowRuntimeError::SessionStore("reason".into()),
            F::Internal,
        ),
        (
            WorkflowRuntimeError::AgentSession("reason".into()),
            F::StateRequired,
        ),
        (
            WorkflowRuntimeError::AlreadyActive(
                crate::domain::workflow::services::start_admission::WorktreeActiveExecution {
                    worktree_path: "/repo".into(),
                    execution_id: "execution".into(),
                    workflow_name: "workflow".into(),
                },
            ),
            F::StateRequired,
        ),
        (WorkflowRuntimeError::Store(F::Temporary), F::Temporary),
        (
            WorkflowRuntimeError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            WorkflowRuntimeError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            WorkflowRuntimeError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (WorkflowRuntimeError::Store(F::Expired), F::Expired),
        (WorkflowRuntimeError::Store(F::Missing), F::Missing),
        (
            WorkflowRuntimeError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (WorkflowRuntimeError::Store(F::Permission), F::Permission),
        (WorkflowRuntimeError::Store(F::Capacity), F::Capacity),
        (WorkflowRuntimeError::Store(F::Unsupported), F::Unsupported),
        (WorkflowRuntimeError::Store(F::Internal), F::Internal),
        (WorkflowRuntimeError::Store(F::Corrupt), F::Corrupt),
        (WorkflowRuntimeError::Store(F::Cancelled), F::Cancelled),
        (WorkflowRuntimeError::Store(F::Unknown), F::Unknown),
        (
            WorkflowRuntimeError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            WorkflowRuntimeError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_node事実追記_分類と失敗理由と実行木の失敗種別を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given
    for kind in [
        FailureKind::Temporary,
        FailureKind::Corrupt,
        FailureKind::Expired,
        FailureKind::RestartRequired,
    ] {
        // When
        let error = WorkflowRuntimeError::StorageFailure {
            kind,
            message: "append failed".into(),
        };
        // Then
        assert_eq!(error.failure_kind(), kind);
        assert_eq!(error.to_string(), "append failed");
        assert_eq!(
            error.workflow_failure_kind(),
            crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash
        );
    }
}

#[test]
fn test_managed_worktree停止_runtime境界で分類を保持する() {
    use crate::domain::failure::ClassifiedFailure;
    use crate::domain::operation_context::OperationStopped;
    // Given
    for stopped in [OperationStopped::Expired, OperationStopped::Cancelled] {
        // When
        let error = WorkflowRuntimeError::from(ManagedWorktreeResolverError::Stopped(stopped));
        // Then
        assert_eq!(error.failure_kind(), stopped.failure_kind());
        assert_eq!(error.to_string(), stopped.to_string());
    }
}
