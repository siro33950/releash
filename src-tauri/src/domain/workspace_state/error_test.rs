use super::*;

#[test]
fn test_失敗分類_workspace_state_error_理由に対応する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    // Given
    let cases = [(WorkspaceStateError::Message("io".into()), F::Internal)];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
