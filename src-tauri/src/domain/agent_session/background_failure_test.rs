use super::*;

#[test]
fn test_要対応_設定と判定と解除を対象ごとに行う() {
    // Given
    let mut state = BackgroundFailureState::default();
    // When / Then
    assert!(!state.observe("cancelled", FailureKind::Cancelled));
    assert!(!state.requires_attention("cancelled"));
    assert!(state.observe("failed", FailureKind::StateRequired));
    assert!(state.requires_attention("failed"));
    assert!(!state.requires_attention("other"));
    assert!(!state.observe("failed", FailureKind::StateRequired));
    assert!(state.clear("failed"));
    assert!(!state.requires_attention("failed"));
}
