use super::*;
use crate::adaptor::gateway::terminal_surface::{
    event_hub::TerminalSurfaceEventHub, runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor,
};
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::{TerminalSurfaceEventSink, TerminalSurfaceGateway};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

fn fixture() -> (
    StateSubscriptionUsecase,
    Arc<TerminalSurfaceRuntimeGatewayFor>,
    Arc<TerminalSurfaceEventHub>,
    TerminalSurface,
) {
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        std::path::PathBuf::new(),
        hub.clone(),
        false,
    ));
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let surface = TerminalSurface::new(1, owner, None);
    gateway.insert_surface(surface.clone());
    let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            gateway.clone(),
            hub.clone(),
        ),
    );
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_terminal(terminal);
    (subscriptions, gateway, hub, surface)
}

#[tokio::test]
async fn test_terminal購読_同じstreamでsnapshot差分と区切りを届ける() {
    // Given
    let (subscriptions, gateway, hub, mut surface) = fixture();
    let raw = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let before = gateway.snapshot_materialization_count();
    // When
    subscriptions
        .start_terminal("client", &raw, None, "input")
        .await
        .unwrap();
    subscriptions
        .start("client", "repository-paths", None)
        .unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(Version { sequence: 0, .. }, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(_))))
    );
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(target, Event::Snapshot(_, _))) if target == "repository-paths")
    );
    stream.next().await;
    stream.next().await;
    assert_eq!(gateway.snapshot_materialization_count(), before + 1);
    for sequence in 1..=100 {
        surface
            .record_output(surface.runtime_generation, std::time::Instant::now())
            .unwrap();
        gateway.insert_surface(surface.clone());
        hub.publish(TerminalSurfaceEvent::Output {
            session_key: surface.session_key.clone(),
            data: "🙂".into(),
            sequence,
        });
    }
    assert_eq!(gateway.snapshot_materialization_count(), before + 1);
    for sequence in 1..=100 {
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(version, Delivery::Delta, value))) if version.sequence == sequence && matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Output { .. })))
        );
    }
}

#[tokio::test]
async fn test_terminal再開_履歴内ならsnapshotを作らず再起動後は作る() {
    let (subscriptions, gateway, hub, mut surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    subscriptions
        .start_terminal("client", &target, None, "input")
        .await
        .unwrap();
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) = stream.next().await
    else {
        panic!("snapshot");
    };
    stream.next().await;
    let before = gateway.snapshot_materialization_count();
    subscriptions.stop("client", &target).unwrap();
    surface
        .record_output(surface.runtime_generation, std::time::Instant::now())
        .unwrap();
    gateway.insert_surface(surface.clone());
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: surface.session_key.clone(),
        data: "next".into(),
        sequence: 1,
    });
    subscriptions
        .start_terminal("client", &target, Some(&version), "input-2")
        .await
        .unwrap();
    assert_eq!(gateway.snapshot_materialization_count(), before);
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(v, Delivery::Delta, _))) if v.sequence == 1)
    );
    stream.next().await;
    subscriptions.stop("client", &target).unwrap();
    let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
    gateway.remove_surface(surface.runtime_generation.value());
    gateway.insert_surface(recreated.clone());
    subscriptions.publisher.initialize(&recreated.summary());
    subscriptions
        .start_terminal("client", &target, Some(&version), "input-3")
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(v, _))) if v.epoch != version.epoch)
    );
    assert_eq!(gateway.snapshot_materialization_count(), before + 1);
}

#[tokio::test]
async fn test_terminal購読_件数上限がなく停止と切断で流量を解放する() {
    let (subscriptions, gateway, _, surface) = fixture();
    let stream = subscriptions.open("client".into()).unwrap();
    for index in 0..20 {
        let owner =
            TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(format!("/repo-{index}")))
                .unwrap();
        let surface = TerminalSurface::new(index + 2, owner.clone(), None);
        gateway.insert_surface(surface.clone());
        subscriptions.publisher.initialize(&surface.summary());
        subscriptions
            .start_terminal(
                "client",
                &SubscriptionTarget::Terminal(owner).to_string(),
                None,
                "input",
            )
            .await
            .unwrap();
    }
    assert_eq!(subscriptions.terminal_inputs.lock().len(), 20);
    drop(stream);
    assert!(subscriptions.terminal_inputs.lock().is_empty());
    assert!(subscriptions
        .terminal_processed(
            "client",
            &SubscriptionTarget::Terminal(surface.owner).to_string(),
            5000
        )
        .is_err());
}

#[tokio::test]
async fn test_snapshot作成中_別terminalのsnapshotと出力とexecutorを止めない() {
    // Given
    use crate::usecase::terminal_surface::io_usecase::io_usecase_tests::FakePtyGateway;
    let mut gateway = FakePtyGateway::new();
    let first = TerminalSurface::new(
        1,
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/first")).unwrap(),
        None,
    );
    let second = TerminalSurface::new(
        2,
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/second")).unwrap(),
        None,
    );
    gateway.additional_surfaces = vec![first.clone(), second.clone()];
    let gateway = Arc::new(gateway);
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            gateway.clone(),
            hub.clone(),
        ),
    );
    let subscriptions = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    )
    .with_terminal(terminal.clone());
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    let (started, waiting) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    *gateway.snapshot_gate.lock() = Some((started, blocked));
    // When
    let request = tokio::spawn({
        let subscriptions = subscriptions.clone();
        async move {
            subscriptions
                .start_terminal(
                    "client",
                    &SubscriptionTarget::Terminal(first.owner).to_string(),
                    None,
                    "first-input",
                )
                .await
        }
    });
    tokio::task::spawn_blocking(move || {
        waiting
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
    })
    .await
    .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        subscriptions.start_terminal(
            "client",
            &SubscriptionTarget::Terminal(second.owner.clone()).to_string(),
            None,
            "second-input",
        ),
    )
    .await;
    // Then
    assert!(!request.is_finished());
    assert!(result.unwrap().is_ok());
    terminal.resize(&second.owner, 40, 120).unwrap();
    terminal
        .write_attached(&second.owner, "second-input", 0, None, "input")
        .unwrap();
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: second.session_key,
        data: "live".into(),
        sequence: 1,
    });
    stream.next().await;
    stream.next().await;
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(
            _,
            Event::Change(_, Delivery::Delta, _)
        ))
    ));
    release.send(()).unwrap();
    request.await.unwrap().unwrap();
}

#[tokio::test]
async fn test_terminal差分_出力の重複を除き同じ出力番号で寸法と終了を配信する() {
    let (subscriptions, _, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    assert!(subscriptions
        .start_terminal("client", &target, None, "")
        .await
        .is_err());
    subscriptions
        .start_terminal("client", &target, None, "input")
        .await
        .unwrap();
    stream.next().await;
    stream.next().await;
    assert!(subscriptions
        .terminal_processed("client", &target, 1)
        .is_err());
    assert!(subscriptions
        .terminal_processed("client", &target, 5000)
        .is_ok());
    let output = TerminalSurfaceEvent::Output {
        session_key: surface.session_key.clone(),
        data: "x".into(),
        sequence: 1,
    };
    hub.publish(output.clone());
    hub.publish(output);
    hub.publish(TerminalSurfaceEvent::Resize {
        session_key: surface.session_key.clone(),
        cols: 120,
        rows: 30,
        sequence: 1,
    });
    hub.publish(TerminalSurfaceEvent::Exit {
        session_key: surface.session_key.clone(),
        runtime_generation: 1,
        exit_code: Some(7),
        sequence: 1,
    });
    for (sequence, expected) in [
        (
            1,
            TerminalSurfaceStreamItem::Output {
                session_key: surface.session_key.clone(),
                data: "x".into(),
                sequence: 1,
            },
        ),
        (
            1,
            TerminalSurfaceStreamItem::Resize {
                session_key: surface.session_key.clone(),
                cols: 120,
                rows: 30,
                sequence: 1,
            },
        ),
        (
            1,
            TerminalSurfaceStreamItem::Exit {
                session_key: surface.session_key.clone(),
                exit_code: Some(7),
                sequence: 1,
            },
        ),
    ] {
        let Some(StateSubscriptionEvent::Item(
            actual_target,
            Event::Change(version, Delivery::Delta, value),
        )) = stream.next().await
        else {
            panic!("terminal delta");
        };
        assert_eq!(actual_target, target);
        assert_eq!(version.sequence, sequence);
        let StateValue::Terminal(actual) = value.as_ref() else {
            panic!("terminal payload");
        };
        assert_eq!(actual, &expected);
    }
    subscriptions.stop("client", &target).unwrap();
    assert!(subscriptions
        .terminal_processed("client", &target, 5000)
        .is_err());
}

#[tokio::test]
async fn test_terminal購読_出力前の寸法変更と終了を版ゼロで届け再開する() {
    // Given
    let (subscriptions, _, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    subscriptions
        .start_terminal("client", &target, None, "input")
        .await
        .unwrap();
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) = stream.next().await
    else {
        panic!("snapshot");
    };
    assert_eq!(version.sequence, 0);
    stream.next().await;
    // When
    hub.publish(TerminalSurfaceEvent::Resize {
        session_key: surface.session_key.clone(),
        cols: 120,
        rows: 30,
        sequence: 0,
    });
    hub.publish(TerminalSurfaceEvent::Exit {
        session_key: surface.session_key.clone(),
        runtime_generation: 1,
        exit_code: Some(7),
        sequence: 0,
    });
    // Then
    for reconnect in [false, true] {
        if reconnect {
            subscriptions.stop("client", &target).unwrap();
            subscriptions
                .start_terminal("client", &target, Some(&version), "input")
                .await
                .unwrap();
        }
        let Some(StateSubscriptionEvent::Item(
            _,
            Event::Change(resize_version, Delivery::Delta, resize),
        )) = stream.next().await
        else {
            panic!("resize");
        };
        assert_eq!(resize_version, version);
        assert!(matches!(
            resize.as_ref(),
            StateValue::Terminal(TerminalSurfaceStreamItem::Resize {
                cols: 120,
                rows: 30,
                sequence: 0,
                ..
            })
        ));
        let Some(StateSubscriptionEvent::Item(
            _,
            Event::Change(exit_version, Delivery::Delta, exit),
        )) = stream.next().await
        else {
            panic!("exit");
        };
        assert_eq!(exit_version, version);
        assert!(matches!(
            exit.as_ref(),
            StateValue::Terminal(TerminalSurfaceStreamItem::Exit {
                exit_code: Some(7),
                sequence: 0,
                ..
            })
        ));
    }
}

#[tokio::test]
async fn test_terminal購読_出力と寸法の逆転は古い寸法を捨てずsnapshotで復元する() {
    // Given
    let (subscriptions, gateway, hub, mut surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    subscriptions
        .start_terminal("client", &target, None, "input")
        .await
        .unwrap();
    stream.next().await;
    stream.next().await;
    surface
        .record_output(surface.runtime_generation, std::time::Instant::now())
        .unwrap();
    surface.checkpoint.cols = 120;
    surface.checkpoint.rows = 30;
    gateway.insert_surface(surface.clone());
    // When
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: surface.session_key.clone(),
        sequence: 1,
        data: "x".into(),
    });
    hub.publish(TerminalSurfaceEvent::Resize {
        session_key: surface.session_key.clone(),
        sequence: 0,
        cols: 120,
        rows: 30,
    });
    // Then
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, snapshot))) =
        stream.next().await
    else {
        panic!("resynchronized snapshot");
    };
    assert_eq!(version.sequence, 1);
    assert!(
        matches!(snapshot.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(value)) if value.checkpoint.cols == 120 && value.checkpoint.rows == 30)
    );
}

#[tokio::test]
async fn test_terminal削除_経路と履歴を解放し購読と入力は明示停止まで保つ() {
    // Given
    let (subscriptions, gateway, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    // When
    for generation in 1..=10 {
        let surface = TerminalSurface::new(generation, surface.owner.clone(), None);
        gateway.insert_surface(surface.clone());
        subscriptions
            .start_terminal("client", &target, None, "input")
            .await
            .unwrap();
        stream.next().await;
        stream.next().await;
        hub.publish(TerminalSurfaceEvent::Output {
            session_key: surface.session_key.clone(),
            data: "retained".into(),
            sequence: 1,
        });
        hub.publish(TerminalSurfaceEvent::Exit {
            session_key: surface.session_key.clone(),
            runtime_generation: generation,
            exit_code: Some(0),
            sequence: 1,
        });
        gateway.remove_surface(generation).unwrap();
        // Then
        assert!(subscriptions.publisher.terminal_routes.lock().is_empty());
        assert_eq!(subscriptions.terminal_inputs.lock().len(), 1);
        let mut state = subscriptions.publisher.state.lock();
        assert!(state.current_version(&target).is_none());
        state.bookmark("client");
        assert!(state.snapshot_requests("client").is_empty());
        drop(state);
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Output { .. })))
        );
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Exit { exit_code: Some(0), .. })))
        );
        assert!(subscriptions
            .publisher
            .state
            .lock()
            .is_subscribed("client", &target));
        assert_eq!(subscriptions.terminal_inputs.lock().len(), 1);
        subscriptions.stop("client", &target).unwrap();
        assert!(subscriptions
            .publisher
            .state
            .lock()
            .active_targets()
            .is_empty());
        assert!(subscriptions.terminal_inputs.lock().is_empty());
    }
}

#[tokio::test]
async fn test_terminal購読開始_古いsummary取得後の再作成でepochを巻き戻さない() {
    // Given
    let (subscriptions, gateway, _, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    let old_version = subscriptions
        .publisher
        .state
        .lock()
        .current_version(&target)
        .unwrap();
    let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
    let replacement = recreated.clone();
    let gateway_for_replacement = gateway.clone();
    *gateway.before_output_order.lock() = Some(Box::new(move || {
        gateway_for_replacement.remove_surface(1).unwrap();
        gateway_for_replacement.insert_surface(replacement);
    }));
    // When
    subscriptions
        .start_terminal("client", &target, Some(&old_version), "new-input")
        .await
        .unwrap();
    // Then
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, value))) =
        stream.next().await
    else {
        panic!("new runtime snapshot");
    };
    assert_ne!(version.epoch, old_version.epoch);
    assert_eq!(
        version,
        subscriptions
            .publisher
            .state
            .lock()
            .current_version(&target)
            .unwrap()
    );
    assert!(
        matches!(value.as_ref(), StateValue::Terminal(TerminalSurfaceStreamItem::Snapshot(surface)) if surface.runtime_generation == recreated.runtime_generation)
    );
    subscriptions.publisher.remove(&surface.summary());
    assert_eq!(
        subscriptions
            .publisher
            .state
            .lock()
            .current_version(&target),
        Some(version)
    );
    assert_eq!(subscriptions.publisher.terminal_routes.lock().len(), 1);
}
