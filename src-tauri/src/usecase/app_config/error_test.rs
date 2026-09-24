use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (UsecaseError::InvalidInput("input".into()), F::InvalidInput),
        (
            UsecaseError::AppConfig(AppConfigError::InvalidInput("input".into())),
            F::InvalidInput,
        ),
        (
            UsecaseError::AppConfig(AppConfigError::Repository("io".into())),
            F::Internal,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
