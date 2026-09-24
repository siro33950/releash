use super::*;

#[test]
fn test_失敗分類_内部失敗を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given
    let error = GitHostError("failure".into());
    // When / Then
    assert_eq!(error.failure_kind(), FailureKind::Internal);
}
