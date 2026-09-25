use super::*;

#[test]
fn test_失敗分類_workflow_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (WorkflowError::External("reason".into()), F::Internal),
        (
            WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
                "launch".into(),
            )),
            F::StateRequired,
        ),
        (
            WorkflowError::Editor(crate::domain::external_editor::EditorError::InvalidInput(
                "input".into(),
            )),
            F::InvalidInput,
        ),
        (
            WorkflowError::Editor(crate::domain::external_editor::EditorError::Settings(
                crate::domain::app_config::AppConfigError::Repository("settings".into()),
            )),
            F::Internal,
        ),
        (
            WorkflowError::CorruptStoredState("reason".into()),
            F::Corrupt,
        ),
        (
            WorkflowError::IncompatibleStoredEvent("reason".into()),
            F::StateRequired,
        ),
        (
            WorkflowError::InvalidState("reason".into()),
            F::StateRequired,
        ),
        (WorkflowError::Validation("reason".into()), F::InvalidInput),
        (WorkflowError::Conflict("reason".into()), F::RestartRequired),
        (WorkflowError::NotFound("reason".into()), F::Missing),
        (
            WorkflowError::UnauthorizedApprovalTarget("reason".into()),
            F::Permission,
        ),
        (WorkflowError::Store(F::Expired), F::Expired),
        (
            WorkflowError::StorageUnavailable {
                kind: crate::domain::failure::FailureKind::Temporary,
                message: "busy".into(),
            },
            F::Temporary,
        ),
        (
            WorkflowError::StorageUnavailable {
                kind: crate::domain::failure::FailureKind::StateRequired,
                message: "repair".into(),
            },
            F::StateRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_書込失敗_workflow変換後も分類を保持する() {
    use crate::domain::failure::ClassifiedFailure;
    use crate::domain::local_event::CommitBatchError;
    // Given
    for error in [
        CommitBatchError::QueueBusy,
        CommitBatchError::CapacityExceeded,
        CommitBatchError::TreeHeadConflict,
        CommitBatchError::AppendOutcomeUnknown,
    ] {
        let expected = error.failure_kind();
        // When / Then
        assert_eq!(WorkflowError::from(error).failure_kind(), expected);
    }
}

#[test]
fn test_workflow停止_分類とメッセージを保持する() {
    use crate::domain::failure::ClassifiedFailure;
    use crate::domain::failure::{FailureKind, TechnicalFailure};
    // Given
    for stopped in [
        TechnicalFailure {
            kind: FailureKind::Expired,
            message: "Operation deadline exceeded".into(),
        },
        TechnicalFailure {
            kind: FailureKind::Cancelled,
            message: "Operation cancelled".into(),
        },
    ] {
        // When
        let error = WorkflowError::Technical(stopped.clone());
        // Then
        assert_eq!(error.failure_kind(), stopped.failure_kind());
        assert_eq!(error.to_string(), stopped.to_string());
    }
}
