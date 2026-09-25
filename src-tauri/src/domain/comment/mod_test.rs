use super::*;

#[test]
fn test_失敗分類_review_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            ReviewError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Expired,
                message: "Operation deadline exceeded".into(),
            }),
            F::Expired,
        ),
        (
            ReviewError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Cancelled,
                message: "Operation cancelled".into(),
            }),
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
    use crate::domain::failure::{FailureKind, TechnicalFailure};
    // Given
    for (stopped, code) in [
        (FailureKind::Expired, ReviewErrorCode::Expired),
        (FailureKind::Cancelled, ReviewErrorCode::Cancelled),
    ] {
        // When / Then
        assert_eq!(
            ReviewError::Technical(TechnicalFailure {
                kind: stopped,
                message: "technical failure".into()
            })
            .code(),
            code
        );
    }
}
