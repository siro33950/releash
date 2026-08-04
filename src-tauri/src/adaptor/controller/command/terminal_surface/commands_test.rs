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

#[tokio::test]
async fn test_ターミナル画面接続_画面写像後は新しい出力と終了だけを送る() {
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo"));
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
    });
    let event_hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = Arc::new(TerminalSurfaceApplication::new(gateway, event_hub.clone()));
    let attachment = application.attach("attachment-1", &owner).unwrap();
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "duplicate".to_string(),
        sequence: 4,
    });
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "live".to_string(),
        sequence: 5,
    });
    event_hub.publish(TerminalSurfaceEvent::Exit {
        session_key: owner.stable_key(),
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
            if data == "live" && *sequence == 5
    ));
    assert!(matches!(
        &sent[2],
        crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1::Exit { exit_code, .. }
            if *exit_code == Some(0)
    ));
}
