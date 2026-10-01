use super::{TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause};

#[test]
fn test_入力失敗_失効attachmentの原因を保持する() {
    // Given
    let cause = TerminalSurfaceInputUnavailableCause::StaleAttachment;
    // When
    let error = TerminalSurfaceGatewayError::input_unavailable(cause.clone());
    // Then
    assert_eq!(error, TerminalSurfaceGatewayError::InputUnavailable(cause));
}
