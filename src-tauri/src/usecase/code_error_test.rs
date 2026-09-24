use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (
            CodeUsecaseError::Code(crate::domain::code::CodeError::External("io".into())),
            F::Internal,
        ),
        (
            CodeUsecaseError::Code(crate::domain::code::CodeError::Rule("state".into())),
            F::StateRequired,
        ),
        (
            CodeUsecaseError::Code(crate::domain::code::CodeError::StaleReviewBlobVersion {
                requested: 1,
                current: 2,
            }),
            F::RestartRequired,
        ),
        (
            CodeUsecaseError::Code(crate::domain::code::CodeError::StaleReviewGroupTarget {
                group_id: "id".into(),
            }),
            F::RestartRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
