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

#[test]
fn test_worktree削除待ちの失敗_期限と取り消しの分類を維持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    for kind in [FailureKind::Expired, FailureKind::Cancelled] {
        let error =
            super::UsecaseError::from(crate::domain::workflow::WorkflowError::StorageUnavailable {
                message: "stopped".into(),
                kind,
            });
        assert_eq!(error.failure_kind(), kind);
    }
}
