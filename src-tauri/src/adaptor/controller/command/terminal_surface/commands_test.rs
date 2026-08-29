use std::sync::{Arc, Mutex};

use super::*;
use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceGateway,
};
use crate::domain::terminal_surface::{
    TerminalProcessState, TerminalSurfaceCheckpoint, TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

#[test]
fn test_ターミナル接続_recovery指定を初回接続と再同期へ対応づける() {
    // Given / When / Then
    assert_eq!(
        TerminalCommandOperation::attachment(false),
        TerminalCommandOperation::Attach
    );
    assert_eq!(
        TerminalCommandOperation::attachment(true),
        TerminalCommandOperation::Resynchronize
    );
}

#[test]
fn test_ターミナル操作_gateway失敗を操作ごとの固定文言へ変換する() {
    // Given
    let cases = [
        (
            TerminalCommandOperation::Initialize,
            "Terminal initialization failed. Try again.",
        ),
        (
            TerminalCommandOperation::GetExisting,
            "Terminal attachment failed. Try again.",
        ),
        (
            TerminalCommandOperation::Attach,
            "Terminal attachment failed. Try again.",
        ),
        (
            TerminalCommandOperation::Resynchronize,
            "Terminal resynchronization failed. Try again.",
        ),
    ];

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
    let cases = [
        (
            TerminalCommandOperation::Initialize,
            "Terminal initialization failed because the request is invalid.",
        ),
        (
            TerminalCommandOperation::GetExisting,
            "Terminal attachment failed because the request is invalid.",
        ),
        (
            TerminalCommandOperation::Attach,
            "Terminal attachment failed because the request is invalid.",
        ),
        (
            TerminalCommandOperation::Resynchronize,
            "Terminal resynchronization failed because the request is invalid.",
        ),
    ];

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
        gateway_error,
        "Terminal input could not be sent. Try again."
    );
    assert_eq!(
        invalid_owner_error,
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
    assert_eq!(gateway_error, "Terminal resize failed. Try again.");
    assert_eq!(
        invalid_owner_error,
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
    let _guard = crate::other::telemetry::lock_test_telemetry();
    crate::other::telemetry::reset_test_metrics();
    crate::other::telemetry::set_performance_configured(true);
    crate::other::telemetry::set_performance_enabled(true);

    start_terminal_launch_performance_collection();
    crate::other::telemetry::record_terminal_launch(
        crate::other::telemetry::TerminalLaunch::PtyOpenAndSpawn,
        std::time::Duration::from_millis(7),
    );

    assert_eq!(
        take_terminal_launch_performance_samples(),
        vec![
            crate::adaptor::protocol::terminal::TerminalLaunchPerformanceSampleV1 {
                phase: "terminal.launch.pty_open_and_spawn".to_string(),
                duration_ms: 7.0,
            }
        ]
    );
    assert!(take_terminal_launch_performance_samples().is_empty());
    crate::other::telemetry::reset_test_metrics();
}

#[test]
fn test_ターミナル起動性能計測_rendererは許可したphaseと有限durationだけを記録する() {
    let _guard = crate::other::telemetry::lock_test_telemetry();
    start_terminal_launch_performance_collection();

    assert!(record_terminal_launch_renderer_phase("provider_id".to_string(), 1.0).is_err());
    assert!(
        record_terminal_launch_renderer_phase("first_xterm_parsed".to_string(), f64::NAN,).is_err()
    );
    assert!(
        record_terminal_launch_renderer_phase("first_xterm_parsed".to_string(), f64::MAX,).is_err()
    );
    record_terminal_launch_renderer_phase("first_xterm_parsed".to_string(), 8.0).unwrap();
    record_terminal_launch_renderer_phase("first_paint".to_string(), 13.0).unwrap();

    assert_eq!(
        take_terminal_launch_performance_samples(),
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

#[tokio::test]
async fn test_ターミナル画面接続_画面写像後は新しい出力と終了だけを送る() {
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::<
            tauri::test::MockRuntime,
        >::default(),
    );
    gateway.insert_surface(TerminalSurface {
        session_key: owner.stable_key(),
        owner: owner.clone(),
        worktree_path: Some("/repo".to_string()),
        label: None,
        runtime_generation: 1.into(),
        process_state: TerminalProcessState::Running,
        checkpoint: TerminalSurfaceCheckpoint {
            replay: "snapshot".to_string(),
            sequence: 4,
            cols: 80,
            rows: 24,
        },
        latest_sequence: 4,
        last_output_at: None,
    });
    let event_hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = Arc::new(TerminalSurfaceApplication::new(gateway, event_hub.clone()));
    let attachment = application.attach("attachment-1", &owner).unwrap();
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "duplicate".into(),
        sequence: 4,
    });
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "live".into(),
        sequence: 5,
    });
    event_hub.publish(TerminalSurfaceEvent::Exit {
        session_key: owner.stable_key(),
        runtime_generation: 1,
        exit_code: Some(0),
        sequence: 6,
    });
    let sent = Arc::new(Mutex::new(Vec::new()));
    let captured = sent.clone();

    forward_terminal_surface_attachment(
        application,
        "attachment-1".to_string(),
        attachment,
        move |item| {
            captured.lock().unwrap().push(item);
            Ok(())
        },
    )
    .await;

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert!(matches!(
        &sent[0],
        crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1::Snapshot { surface }
            if surface.terminal_surface.sequence == 4
    ));
    assert!(matches!(
        &sent[1],
        crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1::Output { data, sequence, .. }
            if data.as_ref() == "live" && *sequence == 5
    ));
    assert!(matches!(
        &sent[2],
        crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1::Exit { exit_code, .. }
            if *exit_code == Some(0)
    ));
}
