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
