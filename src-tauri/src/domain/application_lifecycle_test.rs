use super::*;

#[test]
fn test_command起動受理_終了開始後は起動を受け付けない() {
    // Given
    let mut admission = CommandAdmission::default();
    assert!(admission.accepts_start());
    // When
    admission.stop();
    // Then
    assert!(!admission.accepts_start());
    admission.stop();
    assert!(!admission.accepts_start());
}

#[test]
fn test_command完了受理_終了開始後は結果を受け付けない() {
    // Given
    let mut admission = CommandAdmission::default();
    assert!(admission.accepts_completion());
    // When
    admission.stop();
    // Then
    assert!(!admission.accepts_completion());
}

#[test]
fn test_失敗分類_内部失敗を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given
    let error = ApplicationLifecycleError("failure".into());
    // When / Then
    assert_eq!(error.failure_kind(), FailureKind::Internal);
}
