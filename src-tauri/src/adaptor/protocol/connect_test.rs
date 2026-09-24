use super::*;
use crate::domain::failure::{ClassifiedFailure, FailureKind};
use crate::other::AppError;

#[test]
fn test_失敗分類を単一の対応表でconnectへ変換する() {
    // Given / When / Then
    for (kind, expected) in [
        (FailureKind::Temporary, connectrpc::ErrorCode::Unavailable),
        (FailureKind::RestartRequired, connectrpc::ErrorCode::Aborted),
        (
            FailureKind::StateRequired,
            connectrpc::ErrorCode::FailedPrecondition,
        ),
        (
            FailureKind::Expired,
            connectrpc::ErrorCode::DeadlineExceeded,
        ),
        (FailureKind::Corrupt, connectrpc::ErrorCode::DataLoss),
        (FailureKind::Internal, connectrpc::ErrorCode::Internal),
        (
            FailureKind::InvalidInput,
            connectrpc::ErrorCode::InvalidArgument,
        ),
        (FailureKind::Missing, connectrpc::ErrorCode::NotFound),
        (
            FailureKind::AlreadyPresent,
            connectrpc::ErrorCode::AlreadyExists,
        ),
        (
            FailureKind::Permission,
            connectrpc::ErrorCode::PermissionDenied,
        ),
        (
            FailureKind::Capacity,
            connectrpc::ErrorCode::ResourceExhausted,
        ),
        (
            FailureKind::Unsupported,
            connectrpc::ErrorCode::Unimplemented,
        ),
        (FailureKind::Cancelled, connectrpc::ErrorCode::Canceled),
        (FailureKind::Unknown, connectrpc::ErrorCode::Unknown),
        (FailureKind::OutsideRange, connectrpc::ErrorCode::OutOfRange),
        (
            FailureKind::AuthenticationRequired,
            connectrpc::ErrorCode::Unauthenticated,
        ),
    ] {
        let error = AppError::new("failure").with_failure_kind(kind);
        assert_eq!(classified_error(error.clone()).code, expected);
        let coded = AppError::coded("SAME_CODE", "failure", kind);
        assert_eq!(coded.failure_kind(), kind);
        assert_eq!(command_error(coded.into()).code, expected);
        let command = command_error(error.into());
        assert_eq!(command.code, expected);
        assert_eq!(command.failure_kind(), kind);
        assert_eq!(command.details.len(), 1);
    }
}

#[test]
fn test_実行中のworktreeは状態を直すまで再試行できない() {
    // Given
    let error = crate::usecase::workflow::runtime_error::WorkflowRuntimeError::AlreadyActive(
        crate::domain::workflow::services::start_admission::WorktreeActiveExecution {
            worktree_path: "/worktree".into(),
            execution_id: "execution".into(),
            workflow_name: "workflow".into(),
        },
    );
    // When
    let error = command_error(AppError::from_failure(error).into());
    // Then
    assert_eq!(error.code, connectrpc::ErrorCode::FailedPrecondition);
}
