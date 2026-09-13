use super::*;
use crate::adaptor::protocol::application_operation_v1::*;
use prost::Message;

fn assert_error(value: impl serde::Serialize + Into<wire::CommandError>) {
    let expected = serde_json::to_value(&value).unwrap();
    let error: wire::CommandError = value.into();
    let decoded = wire::CommandError::decode(error.encode_to_vec().as_slice()).unwrap();
    assert_eq!(wire::from_value(decoded).unwrap(), expected);
}

#[test]
fn test_applicationエラー_全variantの説明と相関idとfailureをserdeと同じ意味で返す() {
    // Given / When / Then
    let failure = SafeOperationFailureDtoV1 {
        kind: "storage_unavailable".into(),
        retryable: true,
        label: "Storage unavailable".into(),
        detail: Some("Retry".into()),
        correlation_id: "failure-id".into(),
    };
    assert_error(OperationApplicationErrorDtoV1::InvalidRequest);
    assert_error(OperationApplicationErrorDtoV1::PayloadConflict);
    assert_error(OperationApplicationErrorDtoV1::ShutdownInProgress);
    assert_error(OperationApplicationErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
    assert_error(ApplicationQuitErrorDtoV1::InvalidRequest);
    assert_error(ApplicationQuitErrorDtoV1::PayloadConflict);
    assert_error(ApplicationQuitErrorDtoV1::CapacityExceeded);
    assert_error(ApplicationQuitErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
    assert_error(ApplicationQuitLookupErrorDtoV1::InvalidRequest);
    assert_error(ApplicationQuitLookupErrorDtoV1::NotFound);
    assert_error(ApplicationQuitLookupErrorDtoV1::QueryBusy);
    assert_error(ApplicationQuitLookupErrorDtoV1::DeadlineExceeded);
    assert_error(ApplicationQuitLookupErrorDtoV1::StorageUnavailable {
        failure: failure.clone(),
    });
    assert_error(ApplicationQuitLookupErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
    assert_error(CurrentShutdownErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
    assert_error(ShutdownPlanQueryErrorDtoV1::InvalidRequest);
    assert_error(ShutdownPlanQueryErrorDtoV1::NotFound);
    assert_error(ShutdownPlanQueryErrorDtoV1::DetailsCompacted);
    assert_error(ShutdownPlanQueryErrorDtoV1::CursorMismatch);
    assert_error(ShutdownPlanQueryErrorDtoV1::CursorExpired);
    assert_error(ShutdownPlanQueryErrorDtoV1::QueryBusy);
    assert_error(ShutdownPlanQueryErrorDtoV1::DeadlineExceeded);
    assert_error(ShutdownPlanQueryErrorDtoV1::ResponseTooLarge);
    assert_error(ShutdownPlanQueryErrorDtoV1::StorageUnavailable {
        failure: failure.clone(),
    });
    assert_error(ShutdownPlanQueryErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
    assert_error(ShutdownDetailsMutationErrorDtoV1::InvalidRequest);
    assert_error(ShutdownDetailsMutationErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
    assert_error(RecoveryActionCommandErrorDtoV1::InvalidRequest);
    assert_error(RecoveryActionCommandErrorDtoV1::NotFound);
    assert_error(RecoveryActionCommandErrorDtoV1::StorageUnavailable {
        failure: failure.clone(),
    });
    assert_error(RecoveryActionCommandErrorDtoV1::Internal {
        correlation_id: "correlation-id".into(),
    });
}
