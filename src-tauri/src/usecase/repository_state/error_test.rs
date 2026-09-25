use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (RepositoryStateError::ScanInvalidated, F::RestartRequired),
        (
            RepositoryStateError::Background {
                kind: F::Temporary,
                message: "worker".into(),
            },
            F::Temporary,
        ),
        (
            RepositoryStateError::Background {
                kind: F::Expired,
                message: "deadline".into(),
            },
            F::Expired,
        ),
        (RepositoryStateError::Watcher("watch".into()), F::Internal),
        (
            RepositoryStateError::Repository(UsecaseError::Rule("state".into())),
            F::StateRequired,
        ),
        (
            RepositoryStateError::Repository(UsecaseError::Repository(
                crate::domain::repository::RepositoryError::External("io".into()),
            )),
            F::Internal,
        ),
        (
            RepositoryStateError::Code(CodeUsecaseError::Code(
                crate::domain::code::CodeError::StaleReviewBlobVersion {
                    requested: 1,
                    current: 2,
                },
            )),
            F::RestartRequired,
        ),
        (
            RepositoryStateError::Code(CodeUsecaseError::Code(
                crate::domain::code::CodeError::External("io".into()),
            )),
            F::Internal,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
