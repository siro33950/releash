use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            UsecaseError::Repository(crate::domain::repository::RepositoryError::External(
                "io".into(),
            )),
            F::Internal,
        ),
        (
            UsecaseError::Repository(crate::domain::repository::RepositoryError::Rule(
                "state".into(),
            )),
            F::StateRequired,
        ),
        (UsecaseError::Rule("state".into()), F::StateRequired),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
