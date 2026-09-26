use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use crate::adaptor::presenter::error::AppError;
use connectrpc::ErrorCode;

#[test]
fn test_失敗分類を単一の対応表でconnectへ変換する() {
    // Given / When / Then
    for (kind, expected) in [
        (ErrorCode::Unavailable, connectrpc::ErrorCode::Unavailable),
        (ErrorCode::Aborted, connectrpc::ErrorCode::Aborted),
        (
            ErrorCode::FailedPrecondition,
            connectrpc::ErrorCode::FailedPrecondition,
        ),
        (
            ErrorCode::DeadlineExceeded,
            connectrpc::ErrorCode::DeadlineExceeded,
        ),
        (ErrorCode::DataLoss, connectrpc::ErrorCode::DataLoss),
        (ErrorCode::Internal, connectrpc::ErrorCode::Internal),
        (
            ErrorCode::InvalidArgument,
            connectrpc::ErrorCode::InvalidArgument,
        ),
        (ErrorCode::NotFound, connectrpc::ErrorCode::NotFound),
        (
            ErrorCode::AlreadyExists,
            connectrpc::ErrorCode::AlreadyExists,
        ),
        (
            ErrorCode::PermissionDenied,
            connectrpc::ErrorCode::PermissionDenied,
        ),
        (
            ErrorCode::ResourceExhausted,
            connectrpc::ErrorCode::ResourceExhausted,
        ),
        (
            ErrorCode::Unimplemented,
            connectrpc::ErrorCode::Unimplemented,
        ),
        (ErrorCode::Canceled, connectrpc::ErrorCode::Canceled),
        (ErrorCode::Unknown, connectrpc::ErrorCode::Unknown),
        (ErrorCode::OutOfRange, connectrpc::ErrorCode::OutOfRange),
        (
            ErrorCode::Unauthenticated,
            connectrpc::ErrorCode::Unauthenticated,
        ),
    ] {
        let error = AppError::new("failure").with_status(kind);
        assert_eq!(classified_error(error.clone()).code, expected);
        let coded = AppError::coded("SAME_CODE", "failure", kind);
        assert_eq!(coded.connect_code(), kind);
        assert_eq!(command_error(coded.into()).code, expected);
        let command = command_error(error.into());
        assert_eq!(command.code, expected);
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

#[test]
fn test_技術的な失敗_自身の性質だけから転送コードを決める() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature as N};
    use connectrpc::ErrorCode as C;
    // Given
    for (nature, code) in [
        (N::Transient, C::Unavailable),
        (N::TimedOut, C::DeadlineExceeded),
        (N::Cancelled, C::Canceled),
        (N::Other, C::Internal),
    ] {
        // When
        let error = classified_error(TechnicalFailure {
            nature,
            message: "failure".into(),
        });
        // Then
        assert_eq!(error.code, code);
        assert_eq!(error.message.as_deref(), Some("failure"));
    }
}

#[test]
fn test_失敗記録の分類_六種類を固定文字列として表示する() {
    use crate::domain::failure::{BusinessFailure, Failure, TechnicalFailureNature};
    for (failure, expected) in [
        (
            Failure::Business(BusinessFailure::VersionConflict),
            "VersionConflict",
        ),
        (Failure::Business(BusinessFailure::Other), "BusinessFailure"),
        (
            Failure::Technical(TechnicalFailureNature::Transient),
            "Transient",
        ),
        (
            Failure::Technical(TechnicalFailureNature::TimedOut),
            "TimedOut",
        ),
        (
            Failure::Technical(TechnicalFailureNature::Cancelled),
            "Cancelled",
        ),
        (
            Failure::Technical(TechnicalFailureNature::Other),
            "TechnicalFailure",
        ),
    ] {
        assert_eq!(super::failure_classification(failure), expected);
    }
}

#[test]
fn test_作業手順の失敗_storeに包んでも転送コードを保持する() {
    use crate::domain::failure::{StorageFailure, TechnicalFailure, TechnicalFailureNature};
    use crate::domain::workflow::WorkflowError;
    use crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError;

    // Given
    for (error, expected) in [
        (
            WorkflowError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::TimedOut,
                message: "expired".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (
            WorkflowError::Technical(TechnicalFailure {
                nature: TechnicalFailureNature::Cancelled,
                message: "cancelled".into(),
            }),
            ErrorCode::Canceled,
        ),
        (
            WorkflowError::External("internal".into()),
            ErrorCode::Internal,
        ),
        (
            WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
                "launch".into(),
            )),
            ErrorCode::FailedPrecondition,
        ),
        (
            WorkflowError::Store(StorageFailure::from(
                crate::domain::local_event::CommitBatchError::QueueBusy,
            )),
            ErrorCode::Unavailable,
        ),
        (
            WorkflowError::Store(StorageFailure::from(TechnicalFailure {
                nature: TechnicalFailureNature::TimedOut,
                message: "expired".into(),
            })),
            ErrorCode::DeadlineExceeded,
        ),
        (
            WorkflowError::Store(StorageFailure::from(
                crate::domain::local_event::CommitBatchError::PayloadConflict,
            )),
            ErrorCode::FailedPrecondition,
        ),
    ] {
        // When / Then
        let failure = StorageFailure::from(error);
        assert_eq!(failure.connect_code(), expected);
        assert_eq!(
            ProviderLifecycleIngressUsecaseError::Store(failure).connect_code(),
            expected
        );
    }
}
