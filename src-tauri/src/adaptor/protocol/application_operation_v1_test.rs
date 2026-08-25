use super::*;

fn failure() -> SafeOperationFailureDtoV1 {
    SafeOperationFailureDtoV1 {
        kind: "storage_unavailable".to_string(),
        retryable: true,
        label: "Storage unavailable".to_string(),
        detail: Some("internal detail".to_string()),
        correlation_id: "failure-correlation".to_string(),
    }
}

#[test]
fn test_アプリケーション操作エラー_全variantがtypeと利用者向けmessageを直列化する() {
    // Given
    let cases = vec![
        (
            serde_json::to_value(OperationApplicationErrorDtoV1::InvalidRequest).unwrap(),
            "invalid_request",
            "Releash could not access application operation history because the request is invalid.",
        ),
        (
            serde_json::to_value(OperationApplicationErrorDtoV1::PayloadConflict).unwrap(),
            "payload_conflict",
            "The application operation request conflicts with an earlier request. Refresh and try again.",
        ),
        (
            serde_json::to_value(OperationApplicationErrorDtoV1::ShutdownInProgress).unwrap(),
            "shutdown_in_progress",
            "Releash could not update application operation history while shutdown is in progress. Try again after Releash restarts.",
        ),
        (
            serde_json::to_value(OperationApplicationErrorDtoV1::Internal {
                correlation_id: "operation-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not access application operation history. Try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitErrorDtoV1::InvalidRequest).unwrap(),
            "invalid_request",
            "Releash could not start the application quit because the request is invalid.",
        ),
        (
            serde_json::to_value(ApplicationQuitErrorDtoV1::PayloadConflict).unwrap(),
            "payload_conflict",
            "The application quit request conflicts with an earlier request. Refresh and try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitErrorDtoV1::CapacityExceeded).unwrap(),
            "capacity_exceeded",
            "Releash could not start another application quit. Wait for the current operation and try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitErrorDtoV1::Internal {
                correlation_id: "quit-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not complete the application quit. Try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitLookupErrorDtoV1::InvalidRequest).unwrap(),
            "invalid_request",
            "Releash could not check the application quit because the request is invalid.",
        ),
        (
            serde_json::to_value(ApplicationQuitLookupErrorDtoV1::NotFound).unwrap(),
            "not_found",
            "The application quit operation is no longer available.",
        ),
        (
            serde_json::to_value(ApplicationQuitLookupErrorDtoV1::QueryBusy).unwrap(),
            "query_busy",
            "Releash is still checking the application quit. Try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitLookupErrorDtoV1::DeadlineExceeded).unwrap(),
            "deadline_exceeded",
            "Checking the application quit took too long. Try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitLookupErrorDtoV1::StorageUnavailable {
                failure: failure(),
            })
            .unwrap(),
            "storage_unavailable",
            "Releash could not access the application quit operation. Try again.",
        ),
        (
            serde_json::to_value(ApplicationQuitLookupErrorDtoV1::Internal {
                correlation_id: "lookup-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not check the application quit. Try again.",
        ),
        (
            serde_json::to_value(CurrentShutdownErrorDtoV1::Internal {
                correlation_id: "shutdown-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not check the current application shutdown. Try again.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::InvalidRequest).unwrap(),
            "invalid_request",
            "Releash could not load the shutdown plan because the request is invalid.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::NotFound).unwrap(),
            "not_found",
            "The shutdown plan is no longer available.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::DetailsCompacted).unwrap(),
            "details_compacted",
            "The shutdown plan details are no longer available.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::CursorMismatch).unwrap(),
            "cursor_mismatch",
            "The shutdown plan changed while it was loading. Reload the plan and try again.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::CursorExpired).unwrap(),
            "cursor_expired",
            "The shutdown plan page expired. Reload the plan and try again.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::QueryBusy).unwrap(),
            "query_busy",
            "The shutdown plan is busy. Try again.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::DeadlineExceeded).unwrap(),
            "deadline_exceeded",
            "Loading the shutdown plan took too long. Try again.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::ResponseTooLarge).unwrap(),
            "response_too_large",
            "The shutdown plan is too large to load.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::StorageUnavailable {
                failure: failure(),
            })
            .unwrap(),
            "storage_unavailable",
            "Releash could not access the shutdown plan. Try again.",
        ),
        (
            serde_json::to_value(ShutdownPlanQueryErrorDtoV1::Internal {
                correlation_id: "plan-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not load the shutdown plan. Try again.",
        ),
        (
            serde_json::to_value(ShutdownDetailsMutationErrorDtoV1::InvalidRequest).unwrap(),
            "invalid_request",
            "Releash could not compact the shutdown details because the request is invalid.",
        ),
        (
            serde_json::to_value(ShutdownDetailsMutationErrorDtoV1::Internal {
                correlation_id: "details-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not compact the shutdown details. Try again.",
        ),
        (
            serde_json::to_value(RecoveryActionCommandErrorDtoV1::InvalidRequest).unwrap(),
            "invalid_request",
            "Releash could not resolve the shutdown target because the request is invalid.",
        ),
        (
            serde_json::to_value(RecoveryActionCommandErrorDtoV1::NotFound).unwrap(),
            "not_found",
            "The shutdown target action is no longer available. Reload the shutdown plan and try again.",
        ),
        (
            serde_json::to_value(RecoveryActionCommandErrorDtoV1::StorageUnavailable {
                failure: failure(),
            })
            .unwrap(),
            "storage_unavailable",
            "Releash could not save the shutdown target action. Try again.",
        ),
        (
            serde_json::to_value(RecoveryActionCommandErrorDtoV1::Internal {
                correlation_id: "recovery-correlation".to_string(),
            })
            .unwrap(),
            "internal",
            "Releash could not resolve the shutdown target action. Try again.",
        ),
    ];

    // When / Then
    for (wire, expected_type, expected_message) in cases {
        assert_eq!(wire["type"], expected_type);
        assert_eq!(wire["message"], expected_message);
        assert_ne!(wire["message"], "[object Object]");
    }
}

#[test]
fn test_アプリケーション操作エラー_既存の付加fieldを維持する() {
    // Given / When
    let internal = serde_json::to_value(ApplicationQuitErrorDtoV1::Internal {
        correlation_id: "quit-correlation".to_string(),
    })
    .unwrap();
    let storage = serde_json::to_value(ShutdownPlanQueryErrorDtoV1::StorageUnavailable {
        failure: failure(),
    })
    .unwrap();

    // Then
    assert_eq!(internal["type"], "internal");
    assert_eq!(internal["correlation_id"], "quit-correlation");
    assert_eq!(internal.as_object().unwrap().len(), 3);
    assert_eq!(storage["type"], "storage_unavailable");
    assert_eq!(storage["failure"]["correlation_id"], "failure-correlation");
    assert_eq!(storage.as_object().unwrap().len(), 3);
}
