use super::UsecaseError;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause,
};

#[test]
fn test_入力失敗_失効attachmentを区別する() {
    // Given
    let error = TerminalSurfaceGatewayError::input_unavailable(
        TerminalSurfaceInputUnavailableCause::StaleAttachment,
    );
    // When
    let result = UsecaseError::from(error);
    // Then
    assert_eq!(result, UsecaseError::StaleAttachment);
}
