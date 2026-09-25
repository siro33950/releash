use std::sync::Arc;

use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::domain::terminal_surface::{
    entities::TerminalSurface, TerminalProcessState, TerminalSurfaceCheckpoint,
    TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;

#[test]
fn test_ターミナル画面_所有者概要lookup_不在とowner不整合を区別する() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::default(),
    );
    let application = super::TerminalSurfaceApplication::new(
        gateway.clone(),
        Arc::new(TerminalSurfaceEventHub::new()),
    );

    assert_eq!(
        application.find_owned_summary(&owner),
        super::OwnedTerminalSummaryLookup::Absent
    );

    gateway.insert_surface(TerminalSurface {
        session_key: owner.stable_key(),
        owner: TerminalSurfaceOwner::session(
            WorkspaceIdentity::new("/other-repo"),
            "agent-session-1",
        )
        .unwrap(),
        worktree_path: Some("/other-repo".to_string()),
        label: None,
        runtime_generation: 1.into(),
        process_state: TerminalProcessState::Running,
        checkpoint: TerminalSurfaceCheckpoint::empty(80, 24),
        latest_sequence: 0,
        last_output_at: None,
    });

    assert_eq!(
        application.find_owned_summary(&owner),
        super::OwnedTerminalSummaryLookup::OwnerMismatch
    );
    assert!(application.get_summary(&owner).is_err());
}

#[test]
fn test_summary系読み取りはsnapshot全量再構築を伴わない() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::default(),
    );
    gateway.insert_surface(TerminalSurface {
        session_key: owner.stable_key(),
        owner: owner.clone(),
        worktree_path: Some("/repo".to_string()),
        label: None,
        runtime_generation: 1.into(),
        process_state: TerminalProcessState::Running,
        checkpoint: TerminalSurfaceCheckpoint::empty(80, 24),
        latest_sequence: 0,
        last_output_at: None,
    });
    let application = super::TerminalSurfaceApplication::new(
        gateway.clone(),
        Arc::new(TerminalSurfaceEventHub::new()),
    );

    assert!(matches!(
        application.find_owned_summary(&owner),
        super::OwnedTerminalSummaryLookup::Found(summary)
            if summary.session_key == owner.stable_key()
    ));
    let summary = application
        .get_summary(&owner)
        .expect("summary for registered owner");
    assert_eq!(summary.session_key, owner.stable_key());
    assert!(!summary.process_state.is_exited());
    assert_eq!(gateway.snapshot_materialization_count(), 0);
}

#[tokio::test]
async fn test_サイズ更新_別入口からも予約順を守り別terminalを待たせない() {
    // Given
    let gateway = Arc::new(super::super::io_usecase::io_usecase_tests::FakePtyGateway::new());
    let application = super::TerminalSurfaceApplication::new(
        gateway.clone(),
        Arc::new(TerminalSurfaceEventHub::new()),
    );
    let other_entry = application.clone();
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let other_owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/other")).unwrap();
    let first = application.prepare_resize(owner.clone(), 40, 80);
    let second = other_entry.prepare_resize(owner.clone(), 50, 100);
    // When
    let second = tokio::task::spawn_blocking(second);
    let unrelated = tokio::task::spawn_blocking(move || other_entry.resize(&other_owner, 20, 60));
    tokio::time::timeout(std::time::Duration::from_secs(1), unrelated)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(gateway.resizes.lock().len(), 1);
    tokio::task::spawn_blocking(first).await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    // Then
    assert_eq!(
        gateway
            .resizes
            .lock()
            .iter()
            .map(|(_, rows, cols)| (*rows, *cols))
            .collect::<Vec<_>>(),
        [(20, 60), (40, 80), (50, 100)]
    );
    assert!(application.resize_tails.lock().unwrap().is_empty());
}

#[test]
fn test_サイズ更新_最後の完了で待機列を解放し後続予約は保持する() {
    // Given
    let gateway = Arc::new(super::super::io_usecase::io_usecase_tests::FakePtyGateway::new());
    let application = super::TerminalSurfaceApplication::new(
        gateway.clone(),
        Arc::new(TerminalSurfaceEventHub::new()),
    );
    // When / Then
    for id in 0..10 {
        let owner =
            TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), format!("session-{id}"))
                .unwrap();
        let first = application.prepare_resize(owner.clone(), 40, 80);
        let second = application.prepare_resize(owner, 50, 100);
        first().unwrap();
        assert_eq!(application.resize_tails.lock().unwrap().len(), 1);
        second().unwrap();
        assert!(application.resize_tails.lock().unwrap().is_empty());
    }
    assert_eq!(gateway.resizes.lock().len(), 20);
}

#[test]
fn test_サイズ更新_予約の破棄と受付失敗でも待機列を解放する() {
    // Given
    let gateway = Arc::new(super::super::io_usecase::io_usecase_tests::FakePtyGateway::new());
    let application = super::TerminalSurfaceApplication::new(
        gateway.clone(),
        Arc::new(TerminalSurfaceEventHub::new()),
    );
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    // When / Then
    let resize = application.prepare_resize(owner.clone(), 40, 80);
    drop(resize);
    assert!(application.resize_tails.lock().unwrap().is_empty());
    let resize = application.prepare_resize(owner, 50, 100);
    application.shutdown().unwrap();
    assert_eq!(
        resize(),
        Err(super::UsecaseError::Gateway(
            "Terminal Surface runtime is shutting down".into()
        ))
    );
    assert!(application.resize_tails.lock().unwrap().is_empty());
    assert!(gateway.resizes.lock().is_empty());
}

#[test]
fn test_終了保存_停止と出力排出の失敗後も別terminalと保存へ進む() {
    crate::test_support::install_capturing_logger();
    for failures in [
        vec![],
        vec![("stop", 1)],
        vec![("drain", 1)],
        vec![("flush", 0)],
        vec![("stop", 1), ("drain", 2), ("flush", 0)],
    ] {
        // Given
        let mut gateway = super::super::io_usecase::io_usecase_tests::FakePtyGateway::new();
        gateway.shutdown_failures = failures.clone();
        gateway.shutdown_surfaces = (1..=3)
            .map(|generation| {
                let owner = TerminalSurfaceOwner::session(
                    WorkspaceIdentity::new("/repo"),
                    format!("session-{generation}"),
                )
                .unwrap();
                TerminalSurface {
                    session_key: owner.stable_key(),
                    owner,
                    worktree_path: Some("/repo".into()),
                    label: None,
                    runtime_generation: generation.into(),
                    process_state: TerminalProcessState::Running,
                    checkpoint: TerminalSurfaceCheckpoint::empty(80, 24),
                    latest_sequence: 0,
                    last_output_at: None,
                }
            })
            .collect();
        let gateway = Arc::new(gateway);
        let application = super::TerminalSurfaceApplication::new(
            gateway.clone(),
            Arc::new(TerminalSurfaceEventHub::new()),
        );
        // When
        let result = application.shutdown();
        // Then
        assert_eq!(result.is_err(), !failures.is_empty());
        let messages = crate::test_support::captured_error_messages()
            .into_iter()
            .filter(|message| message.contains(&gateway.shutdown_failure_id))
            .collect::<Vec<_>>();
        let expected_messages = failures
            .iter()
            .map(|(stage, generation)| {
                let context = match *stage {
                    "stop" | "drain" => format!("terminal {generation} {stage}"),
                    "flush" => "terminal checkpoint flush".into(),
                    _ => unreachable!(),
                };
                format!(
                    "application shutdown: {context} failed: {stage} failed {}",
                    gateway.shutdown_failure_id
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(messages, expected_messages);
        let mut expected = vec![("stop", 1), ("stop", 2), ("stop", 3)];
        if !failures.contains(&("stop", 1)) {
            expected.push(("drain", 1));
        }
        expected.extend([("drain", 2), ("drain", 3), ("flush", 0)]);
        assert_eq!(*gateway.shutdown_calls.lock(), expected);
    }
}
