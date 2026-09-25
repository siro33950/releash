use super::*;

#[test]
fn test_失敗分類_repository_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            RepositoryError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Expired,
                message: "Operation deadline exceeded".into(),
            }),
            F::Expired,
        ),
        (
            RepositoryError::Technical(crate::domain::failure::TechnicalFailure {
                kind: crate::domain::failure::FailureKind::Cancelled,
                message: "Operation cancelled".into(),
            }),
            F::Cancelled,
        ),
        (RepositoryError::External("io".into()), F::Internal),
        (RepositoryError::Rule("rule".into()), F::StateRequired),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
