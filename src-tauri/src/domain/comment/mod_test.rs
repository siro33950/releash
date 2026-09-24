use super::*;

#[test]
fn test_失敗分類_review_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
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
