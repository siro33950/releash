use super::*;

#[test]
fn test_失敗分類_code_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            CodeError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Expired,
                message: "Operation deadline exceeded".into(),
            }),
            F::Expired,
        ),
        (
            CodeError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Cancelled,
                message: "Operation cancelled".into(),
            }),
            F::Cancelled,
        ),
        (CodeError::External("io".into()), F::Internal),
        (CodeError::Rule("rule".into()), F::StateRequired),
        (
            CodeError::StaleReviewBlobVersion {
                requested: 1,
                current: 2,
            },
            F::RestartRequired,
        ),
        (
            CodeError::StaleReviewGroupTarget {
                group_id: "group".into(),
            },
            F::RestartRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
