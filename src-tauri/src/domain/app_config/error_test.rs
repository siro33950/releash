use super::*;

#[test]
fn test_失敗分類_app_config_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [
        (AppConfigError::Repository("store".into()), F::Internal),
        (
            AppConfigError::InvalidInput("input".into()),
            F::InvalidInput,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
