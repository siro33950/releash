use super::*;

#[test]
fn test_失敗分類_review_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            ReviewError::Stopped(crate::domain::operation_context::OperationStopped::Expired),
            F::Expired,
        ),
        (
            ReviewError::Stopped(crate::domain::operation_context::OperationStopped::Cancelled),
            F::Cancelled,
        ),
        (ReviewError::InvalidInput("reason".into()), F::InvalidInput),
        (ReviewError::NotFound("reason".into()), F::Missing),
        (
            ReviewError::AlreadyResolved("reason".into()),
            F::StateRequired,
        ),
        (
            ReviewError::PermissionDenied("reason".into()),
            F::Permission,
        ),
        (ReviewError::Io("reason".into()), F::Internal),
        (ReviewError::Serialize("reason".into()), F::Internal),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}

#[test]
fn test_review停止_詳細コードと失敗分類が一致する() {
    use crate::domain::operation_context::OperationStopped;
    // Given
    for (stopped, code) in [
        (OperationStopped::Expired, ReviewErrorCode::Expired),
        (OperationStopped::Cancelled, ReviewErrorCode::Cancelled),
    ] {
        // When / Then
        assert_eq!(ReviewError::Stopped(stopped).code(), code);
    }
}
