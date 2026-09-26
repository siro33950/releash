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
            UsecaseError::Gateway("internal PTY failure".to_string()),
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
    let gateway_error = terminal_write_error(UsecaseError::Gateway(
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
fn test_ターミナル画面変形_resize失敗をtransport共通の固定文言へ変換する() {
    // Given / When
    let gateway_error = terminal_resize_error(UsecaseError::Gateway(
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
        UsecaseError::PtySpawn {
            error: "openpty failed".to_string(),
        },
        UsecaseError::OtherSpawnFailure {
            error: "checkpoint failed".to_string(),
        },
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

#[test]
fn test_ターミナル起動性能計測_commandは匿名phaseとdurationだけを返してdrainする() {
    let _guard = crate::infrastructure::telemetry::metrics::lock_test_telemetry();
    crate::infrastructure::telemetry::metrics::reset_test_metrics();
    crate::infrastructure::telemetry::metrics::set_performance_configured(true);
    crate::infrastructure::telemetry::metrics::set_performance_enabled(true);

    start_terminal_launch_performance_collection_shared();
    crate::infrastructure::telemetry::metrics::record_terminal_launch(
        crate::infrastructure::telemetry::metrics::TerminalLaunch::PtyOpenAndSpawn,
        std::time::Duration::from_millis(7),
    );

    assert_eq!(
        take_terminal_launch_performance_samples_shared(),
        vec![
            crate::adaptor::protocol::terminal::TerminalLaunchPerformanceSampleV1 {
                phase: "terminal.launch.pty_open_and_spawn".to_string(),
                duration_ms: 7.0,
            }
        ]
    );
    assert!(take_terminal_launch_performance_samples_shared().is_empty());
    crate::infrastructure::telemetry::metrics::reset_test_metrics();
}

#[test]
fn test_ターミナル起動性能計測_rendererは許可したphaseと有限durationだけを記録する() {
    let _guard = crate::infrastructure::telemetry::metrics::lock_test_telemetry();
    start_terminal_launch_performance_collection_shared();

    assert!(record_terminal_launch_renderer_phase_shared("provider_id".to_string(), 1.0).is_err());
    assert!(record_terminal_launch_renderer_phase_shared(
        "first_xterm_parsed".to_string(),
        f64::NAN,
    )
    .is_err());
    assert!(record_terminal_launch_renderer_phase_shared(
        "first_xterm_parsed".to_string(),
        f64::MAX,
    )
    .is_err());
    record_terminal_launch_renderer_phase_shared("first_xterm_parsed".to_string(), 8.0).unwrap();
    record_terminal_launch_renderer_phase_shared("first_paint".to_string(), 13.0).unwrap();

    assert_eq!(
        take_terminal_launch_performance_samples_shared(),
        vec![
            crate::adaptor::protocol::terminal::TerminalLaunchPerformanceSampleV1 {
                phase: "terminal.launch.first_xterm_parsed".to_string(),
                duration_ms: 8.0,
            },
            crate::adaptor::protocol::terminal::TerminalLaunchPerformanceSampleV1 {
                phase: "terminal.launch.first_paint".to_string(),
                duration_ms: 13.0,
            },
        ]
    );
}
