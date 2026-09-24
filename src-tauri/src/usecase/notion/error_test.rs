use super::*;

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (NotionUsecaseError::ConfigNotFound, F::StateRequired),
        (
            NotionUsecaseError::AppConfig(AppConfigError::InvalidInput("input".into())),
            F::InvalidInput,
        ),
        (
            NotionUsecaseError::AppConfig(AppConfigError::Repository("io".into())),
            F::Internal,
        ),
        (
            NotionUsecaseError::Notion(NotionError::RequestFailed("network".into())),
            F::Temporary,
        ),
        (
            NotionUsecaseError::Notion(NotionError::ApiError("api".into())),
            F::StateRequired,
        ),
        (
            NotionUsecaseError::Notion(NotionError::ParseError("parse".into())),
            F::Internal,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
