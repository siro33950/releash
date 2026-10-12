use super::*;
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

#[test]
fn test_ui状態保存失敗_版競合の通信本文と既存の表示detailを保持する() {
    use crate::domain::workspace_state::WorkspaceStateError;
    for (source, code, message, detail) in [
        (
            WorkspaceStateError::VersionConflict,
            connectrpc::ErrorCode::Aborted,
            "Workspace state version conflict",
            "Workspace state version conflict",
        ),
        (
            WorkspaceStateError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "Operation deadline exceeded".into(),
            }),
            connectrpc::ErrorCode::DeadlineExceeded,
            "Operation deadline exceeded",
            "Operation deadline exceeded",
        ),
        (
            WorkspaceStateError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Cancelled,
                message: "Operation cancelled".into(),
            }),
            connectrpc::ErrorCode::Canceled,
            "Operation cancelled",
            "Operation cancelled",
        ),
        (
            WorkspaceStateError::Message("write failed".into()),
            connectrpc::ErrorCode::Internal,
            "Command failed",
            "write failed",
        ),
    ] {
        // Given
        let error = crate::adaptor::presenter::error::AppError::from(source);
        // When
        let wire: CommandFailure = error.into();
        let display = wire::from_value(wire.detail.clone()).unwrap();
        let connect = crate::adaptor::presenter::connect::command_error(wire);
        // Then
        assert_eq!(connect.code, code);
        assert_eq!(connect.message.as_deref(), Some(message));
        assert_eq!(display, serde_json::json!(detail));
    }
}
