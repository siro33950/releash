use super::*;
use prost::Message;

#[test]
fn test_起動失敗_利用不可エラーの意味をwireの往復で保持する() {
    // Given
    let error: wire::CommandError =
        crate::usecase::application_startup::ApplicationUnavailable::ApplicationUnavailable.into();
    // When
    let decoded = wire::CommandError::decode(error.encode_to_vec().as_slice()).unwrap();
    // Then
    assert_eq!(
        wire::from_value(decoded).unwrap(),
        serde_json::json!({"type": "application_unavailable"})
    );
}

#[test]
fn test_terminal失敗_通信本文は元のメッセージで表示detailは維持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    use crate::usecase::terminal_surface::error::UsecaseError;
    // Given
    let error = UsecaseError::Technical(TechnicalFailure {
        nature: TechnicalFailureNature::TimedOut,
        message: "PTY resize timed out".into(),
    });
    // When
    let presented = crate::adaptor::presenter::terminal_error::terminal_resize_error(error);
    let wire: CommandFailure = presented.into();
    let display = wire::from_value(wire.detail.clone()).unwrap();
    let connect = crate::adaptor::presenter::connect::command_error(wire);
    // Then
    assert_eq!(connect.code, connectrpc::ErrorCode::DeadlineExceeded);
    assert_eq!(connect.message.as_deref(), Some("PTY resize timed out"));
    assert_eq!(
        display,
        serde_json::json!("Terminal resize failed. Try again.")
    );
}
