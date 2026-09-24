use super::*;

#[test]
fn test_失敗分類_editor_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (EditorError::InvalidInput("input".into()), F::InvalidInput),
        (EditorError::Launch("launch".into()), F::StateRequired),
        (
            EditorError::Settings(crate::domain::app_config::AppConfigError::Repository(
                "store".into(),
            )),
            F::Internal,
        ),
        (
            EditorError::Settings(crate::domain::app_config::AppConfigError::InvalidInput(
                "input".into(),
            )),
            F::InvalidInput,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
