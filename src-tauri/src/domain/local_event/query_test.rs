use super::*;

#[test]
fn test_失敗分類_local_event_query_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (LocalEventQueryError::InvalidRequest, F::InvalidInput),
        (LocalEventQueryError::QueryBusy, F::Temporary),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Expired,
                message: "deadline exceeded".into(),
            }),
            F::Expired,
        ),
        (LocalEventQueryError::ResponseTooLarge, F::Capacity),
        (
            LocalEventQueryError::IncompatibleStoredEvent {
                correlation_id: "id".into(),
            },
            F::StateRequired,
        ),
        (
            LocalEventQueryError::Corrupt {
                correlation_id: "id".into(),
            },
            F::Corrupt,
        ),
        (
            LocalEventQueryError::Internal {
                correlation_id: "id".into(),
            },
            F::Internal,
        ),
        (
            LocalEventQueryError::StorageUnavailable {
                failure: super::SafeOperationFailure::new(
                    crate::domain::local_event::SessionOperationFailureKind::StorageUnavailable,
                    F::Expired,
                    "busy",
                    "id",
                ),
            },
            F::Expired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
