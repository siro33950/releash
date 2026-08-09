use std::sync::Arc;

use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventReceiveError, TerminalSurfaceEventSink,
    TerminalSurfaceEventSource, TerminalSurfaceGateway,
};
use crate::domain::terminal_surface::{
    entities::TerminalSurface, TerminalProcessState, TerminalSurfaceCheckpoint,
    TerminalSurfaceOwner,
};
use crate::domain::workspace_tree::WorkspaceIdentity;

#[test]
fn test_ターミナル画面_アプリケーション_ドメインポートだけに依存する() {
    let source = include_str!("application.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();

    assert!(!production.contains(&["tokio", "::"].concat()));
    assert!(!production.contains("parking_lot"));
    assert!(!production.contains("struct TerminalSurfaceEventHub"));
    assert!(!production.contains("RwLock<bool>"));
    assert!(!production.contains("accepting_mutations"));
}

#[test]
fn test_ターミナル画面_所有者概要lookup_不在とowner不整合を区別する() {
    let owner =
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), "agent-session-1").unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::<
            tauri::test::MockRuntime,
        >::default(),
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
    assert!(!summary.is_exited);
    assert_eq!(gateway.snapshot_materialization_count(), 0);
}

#[tokio::test]
async fn test_ターミナル画面_イベント配信_出力欠落を隠さず遅延欠落を返す() {
    let hub = TerminalSurfaceEventHub::with_capacity(2);
    let mut subscription = hub
        .subscribe_owner("surface-1", "attachment-1")
        .subscription;

    for sequence in 1..=3 {
        hub.publish(TerminalSurfaceEvent::Output {
            session_key: "surface-1".to_string(),
            data: format!("chunk-{sequence}").into(),
            sequence,
        });
    }

    assert_eq!(
        subscription.recv().await,
        Err(TerminalSurfaceEventReceiveError::Lagged(1))
    );
}

#[tokio::test]
async fn test_ターミナル画面_イベント配信_owner購読へ別ownerの負荷を流さない() {
    let hub = TerminalSurfaceEventHub::with_capacity(2);
    let mut subscription = hub
        .subscribe_owner("surface-a", "attachment-a")
        .subscription;

    for sequence in 1..=3 {
        hub.publish(TerminalSurfaceEvent::Output {
            session_key: "surface-b".to_string(),
            data: format!("unrelated-{sequence}").into(),
            sequence,
        });
    }
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: "surface-a".to_string(),
        data: "owned".into(),
        sequence: 1,
    });

    assert!(matches!(
        subscription.recv().await,
        Ok(TerminalSurfaceEvent::Output {
            session_key,
            data,
            sequence: 1
        }) if session_key == "surface-a" && data.as_ref() == "owned"
    ));
}

#[test]
fn test_ターミナル画面_出力credit_xterm累積ackまで同一ownerのproducerを停止する() {
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let _subscription = hub.subscribe_owner("surface-a", "attachment-a");
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: "surface-a".to_string(),
        data: "a".repeat(256 * 1024).into(),
        sequence: 1,
    });
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let publisher = std::thread::spawn({
        let hub = hub.clone();
        move || {
            hub.publish(TerminalSurfaceEvent::Output {
                session_key: "surface-a".to_string(),
                data: "next".into(),
                sequence: 2,
            });
            finished_tx.send(()).unwrap();
        }
    });

    let completed_before_parse = finished_rx
        .recv_timeout(std::time::Duration::from_millis(50))
        .is_ok();
    hub.acknowledge_owner_output("surface-a", "attachment-a", 1);

    assert!(!completed_before_parse);
    assert!(finished_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok());
    publisher.join().unwrap();
}

#[test]
fn test_ターミナル画面_出力credit_切断は停止中producerを解放する() {
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let subscription = hub.subscribe_owner("surface-a", "attachment-a");
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: "surface-a".to_string(),
        data: "a".repeat(256 * 1024).into(),
        sequence: 1,
    });
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let publisher = std::thread::spawn({
        let hub = hub.clone();
        move || {
            hub.publish(TerminalSurfaceEvent::Output {
                session_key: "surface-a".to_string(),
                data: "next".into(),
                sequence: 2,
            });
            finished_tx.send(()).unwrap();
        }
    });

    assert!(finished_rx
        .recv_timeout(std::time::Duration::from_millis(50))
        .is_err());
    subscription.cancellation.cancel();

    assert!(finished_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok());
    publisher.join().unwrap();
}

#[tokio::test]
async fn test_ターミナル画面接続_エンティティ_画面写像とバックエンド_イベント配信を返す() {
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
    let application = super::TerminalSurfaceApplication::new(gateway.clone(), event_hub.clone());

    let mut attachment = application
        .attach("attachment-1", &owner)
        .expect("attach backend surface");
    assert_eq!(gateway.snapshot_materialization_count(), 1);
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "live".into(),
        sequence: 5,
    });

    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(surface))
            if surface.checkpoint.replay == "snapshot"
    ));
    assert_eq!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Output {
            session_key: owner.stable_key(),
            data: "live".into(),
            sequence: 5,
        })
    );
}

#[tokio::test]
async fn test_ターミナル画面接続_出力寸法変更終了を一つの連番で並べる() {
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::<
            tauri::test::MockRuntime,
        >::default(),
    );
    gateway.insert_surface(TerminalSurface::with_checkpoint(
        1,
        owner.clone(),
        None,
        TerminalSurfaceCheckpoint {
            replay: "snapshot".to_string(),
            sequence: 4,
            cols: 80,
            rows: 24,
        },
    ));
    let event_hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = super::TerminalSurfaceApplication::new(gateway, event_hub.clone());
    let mut attachment = application.attach("attachment-1", &owner).unwrap();

    event_hub.publish(TerminalSurfaceEvent::Resize {
        session_key: owner.stable_key(),
        cols: 70,
        rows: 20,
        sequence: 3,
    });
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "live".into(),
        sequence: 5,
    });
    event_hub.publish(TerminalSurfaceEvent::Resize {
        session_key: owner.stable_key(),
        cols: 111,
        rows: 37,
        sequence: 6,
    });
    event_hub.publish(TerminalSurfaceEvent::Exit {
        session_key: owner.stable_key(),
        runtime_generation: 1,
        exit_code: Some(0),
        sequence: 7,
    });

    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(_))
    ));
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Output { sequence: 5, .. })
    ));
    assert_eq!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Resize {
            session_key: owner.stable_key(),
            cols: 111,
            rows: 37,
            sequence: 6,
        })
    );
    assert_eq!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Exit {
            session_key: owner.stable_key(),
            exit_code: Some(0),
            sequence: 7,
        })
    );
}

#[tokio::test]
async fn test_ターミナル画面切断_対象購読だけを取消す() {
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
        checkpoint: TerminalSurfaceCheckpoint::empty(80, 24),
        latest_sequence: 0,
        last_output_at: None,
    });
    let application =
        super::TerminalSurfaceApplication::new(gateway, Arc::new(TerminalSurfaceEventHub::new()));
    let mut attachment = application.attach("attachment-1", &owner).unwrap();
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(_))
    ));

    application.detach("attachment-1");

    assert!(attachment.next().await.is_none());
    assert!(application.get(&owner).is_ok());
}

#[tokio::test]
async fn test_ターミナル画面再同期_遅延欠落後にエンティティから復元して包含済み出力を飛ばす() {
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::<
            tauri::test::MockRuntime,
        >::default(),
    );
    let surface = |sequence| TerminalSurface {
        session_key: owner.stable_key(),
        owner: owner.clone(),
        worktree_path: Some("/repo".to_string()),
        label: None,
        runtime_generation: 1.into(),
        process_state: TerminalProcessState::Running,
        checkpoint: TerminalSurfaceCheckpoint {
            replay: format!("snapshot-{sequence}"),
            sequence,
            cols: 80,
            rows: 24,
        },
        latest_sequence: sequence,
        last_output_at: None,
    };
    gateway.insert_surface(surface(4));
    let event_hub = Arc::new(TerminalSurfaceEventHub::with_capacity(2));
    let application = super::TerminalSurfaceApplication::new(gateway.clone(), event_hub.clone());
    let mut attachment = application.attach("attachment-1", &owner).unwrap();
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(snapshot))
            if snapshot.checkpoint.sequence == 4
    ));

    for sequence in 5..=7 {
        event_hub.publish(TerminalSurfaceEvent::Output {
            session_key: owner.stable_key(),
            data: format!("chunk-{sequence}").into(),
            sequence,
        });
    }
    gateway.insert_surface(surface(7));

    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(snapshot))
            if snapshot.checkpoint.sequence == 7
    ));
    let publish_after_next_poll = tokio::spawn({
        let event_hub = event_hub.clone();
        let session_key = owner.stable_key();
        async move {
            tokio::task::yield_now().await;
            event_hub.publish(TerminalSurfaceEvent::Output {
                session_key: session_key.clone(),
                data: "covered".into(),
                sequence: 7,
            });
            event_hub.publish(TerminalSurfaceEvent::Output {
                session_key,
                data: "new".into(),
                sequence: 8,
            });
        }
    });
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Output { data, sequence, .. })
            if data.as_ref() == "new" && sequence == 8
    ));
    publish_after_next_poll.await.unwrap();
}

#[tokio::test]
async fn test_ターミナル画面再接続_重複逆転を除外し連番欠落を最新画面写像へ再同期する() {
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let gateway = Arc::new(
        crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor::<
            tauri::test::MockRuntime,
        >::default(),
    );
    let surface = |sequence| {
        TerminalSurface::with_checkpoint(
            1,
            owner.clone(),
            None,
            TerminalSurfaceCheckpoint {
                replay: format!("snapshot-{sequence}"),
                sequence,
                cols: 80,
                rows: 24,
            },
        )
    };
    gateway.insert_surface(surface(4));
    let event_hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = super::TerminalSurfaceApplication::new(gateway.clone(), event_hub.clone());
    let mut attachment = application.attach("attachment-1", &owner).unwrap();
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(snapshot))
            if snapshot.checkpoint.sequence == 4
    ));

    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "duplicate".into(),
        sequence: 4,
    });
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "reversal".into(),
        sequence: 3,
    });
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "next".into(),
        sequence: 5,
    });
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Output { data, sequence, .. })
            if data.as_ref() == "next" && sequence == 5
    ));

    gateway.insert_surface(surface(7));
    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: owner.stable_key(),
        data: "gap".into(),
        sequence: 7,
    });

    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Snapshot(snapshot))
            if snapshot.checkpoint.sequence == 7
                && snapshot.checkpoint.replay == "snapshot-7"
    ));

    for (data, sequence) in [("duplicate", 7), ("reversal", 6), ("next", 8)] {
        event_hub.publish(TerminalSurfaceEvent::Output {
            session_key: owner.stable_key(),
            data: data.into(),
            sequence,
        });
    }
    assert!(matches!(
        attachment.next().await,
        Some(super::TerminalSurfaceStreamItem::Output { data, sequence, .. })
            if data.as_ref() == "next" && sequence == 8
    ));
}
