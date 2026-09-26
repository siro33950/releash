use super::*;

#[test]
fn test_workflow失敗_httpのステータスと本文コードを保持する() {
    // Given
    let cases = [
        (
            WorkflowError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            500,
            "workflow_error",
        ),
        (
            WorkflowError::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            503,
            "storage_unavailable",
        ),
        (
            WorkflowError::Store(
                crate::domain::failure::StorageFailure::from(
                    crate::domain::local_event::CommitBatchError::QueueBusy,
                )
                .with_message("failure"),
            ),
            503,
            "storage_unavailable",
        ),
        (
            WorkflowError::Validation("failure".into()),
            400,
            "validation_error",
        ),
        (
            WorkflowError::Conflict("failure".into()),
            409,
            "workflow_conflict",
        ),
        (
            WorkflowError::InvalidState("failure".into()),
            409,
            "invalid_state",
        ),
        (WorkflowError::NotFound("failure".into()), 404, "not_found"),
        (
            WorkflowError::UnauthorizedApprovalTarget("failure".into()),
            403,
            "unauthorized_approval_target",
        ),
        (
            WorkflowError::External("failure".into()),
            500,
            "workflow_error",
        ),
        (
            WorkflowError::Editor(crate::domain::external_editor::EditorError::Launch(
                "failure".into(),
            )),
            500,
            "workflow_error",
        ),
        (
            WorkflowError::CorruptStoredState("failure".into()),
            500,
            "corrupt_stored_state",
        ),
        (
            WorkflowError::IncompatibleStoredEvent("failure".into()),
            500,
            "incompatible_stored_event",
        ),
    ];
    for (error, status, code) in cases {
        // When
        let error = ApiError::from(error);
        // Then
        assert_eq!(error.status.as_u16(), status);
        assert_eq!(error.body.code, code);
        assert!(!error.body.message.is_empty());
    }
}

#[test]
fn test_provider失敗_httpのステータスと本文コードを保持する() {
    use crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError as E;
    // Given
    for (error, status, code) in [
        (E::InvalidInput, 400, "invalid_request"),
        (E::Conflict, 409, "provider_lifecycle_conflict"),
        (
            E::StorageUnavailable,
            503,
            "provider_lifecycle_storage_unavailable",
        ),
        (
            E::Store(
                (crate::domain::local_event::CommitBatchError::Corrupt {
                    correlation_id: "id".into(),
                })
                .into(),
            ),
            503,
            "provider_lifecycle_storage_unavailable",
        ),
        (E::Corrupt, 500, "internal_error"),
    ] {
        // When
        let error = ApiError::from(error);
        // Then
        assert_eq!(error.status.as_u16(), status);
        assert_eq!(error.body.code, code);
    }
}
