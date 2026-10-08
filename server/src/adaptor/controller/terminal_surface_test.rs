use super::*;
use crate::usecase::terminal_surface::error::UsecaseError;

#[test]
fn test_ターミナル操作_gateway失敗を操作ごとの固定文言へ変換する() {
    // Given
    let cases = [(
        TerminalCommandOperation::Initialize,
        "Terminal initialization failed. Try again.",
    )];

    // When / Then
    for (operation, expected_message) in cases {
        let command_error = TerminalCommandError::from_usecase(
            UsecaseError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::Other,
                message: "internal PTY failure".to_string(),
            }),
            operation,
        );
        assert_eq!(
            serde_json::to_value(command_error).unwrap(),
            serde_json::json!({
                "code": "PTY_ERROR",
                "message": expected_message,
            })
        );
    }
}

#[test]
fn test_ターミナル操作_不正なownerを操作ごとの固定文言へ変換する() {
    // Given
    let cases = [(
        TerminalCommandOperation::Initialize,
        "Terminal initialization failed because the request is invalid.",
    )];

    // When / Then
    for (operation, expected_message) in cases {
        let command_error = invalid_owner_error(
            operation,
            "invalid Terminal Surface owner: empty workspace path".to_string(),
        );
        assert_eq!(
            serde_json::to_value(command_error).unwrap(),
            serde_json::json!({
                "code": "INVALID_REQUEST",
                "message": expected_message,
            })
        );
    }
}

#[test]
fn test_ターミナル入力_write失敗をtransport共通の固定文言へ変換する() {
    // Given / When
    let gateway_error = terminal_write_error(UsecaseError::InvalidOperation(
        "Terminal input reorder buffer is full".to_string(),
    ));
    let invalid_owner_error = invalid_terminal_write_owner_error(
        "invalid Terminal Surface owner: empty workspace path".to_string(),
    );

    // Then
    assert_eq!(
        gateway_error.to_string(),
        "Terminal input could not be sent. Try again."
    );
    assert_eq!(
        invalid_owner_error.to_string(),
        "Terminal input could not be sent because the request is invalid."
    );
}

#[test]
fn test_失効した入力attachmentだけが機械可読codeを持つ() {
    use crate::adaptor::presenter::connect::ConnectFailure;

    // Given
    let stale = terminal_write_error(UsecaseError::StaleAttachment);
    let ordinary = terminal_write_error(UsecaseError::Technical(
        crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::Other,
            message: "write failed".into(),
        },
    ));

    // When
    let stale_code = stale.connect_code();
    let response: crate::adaptor::presenter::client::CommandFailure = stale.into();

    // Then
    assert_eq!(stale_code, connectrpc::ErrorCode::FailedPrecondition);
    assert_eq!(response.kind, connectrpc::ErrorCode::FailedPrecondition);
    assert_eq!(
        crate::adaptor::presenter::client::from_value(response.detail).unwrap(),
        serde_json::json!({
            "code": "STALE_TERMINAL_ATTACHMENT",
            "message": "Terminal input could not be sent. Try again."
        })
    );
    assert_eq!(
        serde_json::to_value(ordinary).unwrap(),
        serde_json::json!("Terminal input could not be sent. Try again.")
    );
}

#[test]
fn test_ターミナル画面変形_resize失敗をtransport共通の固定文言へ変換する() {
    // Given / When
    let gateway_error = terminal_resize_error(UsecaseError::InvalidOperation(
        "Terminal runtime host is not bound".to_string(),
    ));
    let invalid_owner_error = invalid_terminal_resize_owner_error(
        "invalid Terminal Surface owner: empty workspace path".to_string(),
    );

    // Then
    assert_eq!(
        gateway_error.to_string(),
        "Terminal resize failed. Try again."
    );
    assert_eq!(
        invalid_owner_error.to_string(),
        "Terminal resize failed because the request is invalid."
    );
}

#[test]
fn test_ターミナル画面生成_spawn失敗を汎用codeと固定文言へ変換する() {
    // Given
    let errors = [
        UsecaseError::OwnerConflict,
        UsecaseError::Technical(crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::Other,
            message: "openpty failed".to_string(),
        }),
        UsecaseError::Technical(crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::Other,
            message: "checkpoint failed".to_string(),
        }),
    ];

    // When / Then
    for error in errors {
        let internal_cause = error.to_string();
        let command_error =
            TerminalCommandError::from_usecase(error, TerminalCommandOperation::Initialize);
        let wire = serde_json::to_value(command_error).unwrap();

        assert_eq!(
            wire,
            serde_json::json!({
                "code": "PTY_ERROR",
                "message": "Terminal initialization failed. Try again.",
            })
        );
        assert!(!wire.to_string().contains(&internal_cause));
    }
}
