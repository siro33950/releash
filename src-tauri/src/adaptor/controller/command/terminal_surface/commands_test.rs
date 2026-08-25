use std::sync::{Arc, Mutex};

use super::*;
use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::terminal_surface::entities::{
    TerminalSurface, TerminalSurfaceSpawnReservationError,
};
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceGateway,
};
use crate::domain::terminal_surface::{
    TerminalProcessState, TerminalSurfaceCheckpoint, TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

#[test]
fn test_ターミナル画面生成_上限到達を安定したコマンドコードへ変換する() {
    let worktree_error = UsecaseError::from(
        TerminalSurfaceSpawnReservationError::WorktreeCapReached("/repo".to_string()),
    );
    let total_error = UsecaseError::from(TerminalSurfaceSpawnReservationError::TotalCapReached);

    assert_eq!(
        TerminalCommandError::from(worktree_error).code,
        PTY_ERROR_CODE_CAP_REACHED
    );
    assert_eq!(
        TerminalCommandError::from(total_error).code,
        PTY_ERROR_CODE_CAP_REACHED
    );
}

#[test]
fn test_ターミナル画面生成_上限以外のspawn失敗を従来の汎用コードへ変換する() {
    let errors = [
        UsecaseError::OwnerConflict,
        UsecaseError::PtySpawn {
            error: "openpty failed".to_string(),
        },
        UsecaseError::OtherSpawnFailure {
            error: "checkpoint failed".to_string(),
        },
    ];

    for error in errors {
        assert_eq!(
            TerminalCommandError::from(error).code,
            PTY_ERROR_CODE_GENERIC
        );
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
