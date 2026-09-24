use super::*;

#[test]
fn test_失敗分類_notion_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (NotionError::RequestFailed("network".into()), F::Temporary),
        (NotionError::ApiError("api".into()), F::StateRequired),
        (NotionError::ParseError("json".into()), F::Internal),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
