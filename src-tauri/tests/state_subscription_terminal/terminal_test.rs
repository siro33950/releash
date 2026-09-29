use super::*;
use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGatewayFor;
use crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::TerminalSurfaceGateway;
use crate::test_support::state_subscription::{
    terminal_item, Delivery, Event, StateSubscriptionEvent, Version,
};
use crate::usecase::state_subscription::SubscriptionTarget;
use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;
use crate::usecase::terminal_surface::output::TerminalSurfaceStateSink;

use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;

struct BlockingResetOutput {
    hub: Arc<TerminalSurfaceEventHub>,
    started: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl TerminalSurfaceOutputControl for BlockingResetOutput {
    fn set_state_sink(
        &self,
        sink: Arc<dyn TerminalSurfaceStateSink>,
    ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
        self.hub.set_state_sink(sink)
    }

    fn initialize(
        &self,
        registration: crate::usecase::terminal_surface::output::TerminalRegistration,
    ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
        self.hub.initialize(registration)
    }

    fn subscribe_output(&self, session_key: &str, client: &str, units: usize) {
        if client == "stopping" && units == 0 {
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.hub.subscribe_output(session_key, client, units);
    }

    fn unsubscribe_output(&self, session_key: &str, client: &str) {
        self.hub.unsubscribe_output(session_key, client);
    }

    fn processed_output(&self, session_key: &str, client: &str, units: usize) {
        self.hub.processed_output(session_key, client, units);
    }
}

fn fixture() -> (
    StateSubscriptionUsecase,
    Arc<TerminalSurfaceRuntimeGatewayFor>,
    Arc<TerminalSurfaceEventHub>,
    TerminalSurface,
) {
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let gateway = Arc::new(TerminalSurfaceRuntimeGatewayFor::new_with_event_sink(
        Arc::new(|_| {}),
        std::path::PathBuf::new(),
        hub.clone(),
        false,
    ));
    let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
    let surface = TerminalSurface::new(1, owner, None);
    gateway.insert_surface(surface.clone());
    hub.initialize(crate::test_support::state_subscription::registration(
        &surface.session_key,
        "/repo",
        None,
        1,
        0,
    ))
    .unwrap();
    let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub.clone(),
        ),
    );
    let subscriptions = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    subscriptions
        .test_presenter()
        .unwrap()
        .connect_terminal(&terminal)
        .unwrap();
    let subscriptions = subscriptions.with_terminal(terminal);
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
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &raw,
        None,
        "input",
    )
    .await
    .unwrap();
    crate::test_support::state_subscription::start_read(
        &subscriptions,
        "client",
        "repository-paths",
        None,
    )
    .await
    .unwrap();
    // Then
    let mut terminal_snapshot = false;
    let mut paths_snapshot = false;
    let mut terminal_bookmark = false;
    let mut paths_bookmark = false;
    for _ in 0..4 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
        {
            Some(StateSubscriptionEvent::Item(
                target,
                Event::Snapshot(Version { sequence: 0, .. }, value),
            )) if target == raw => {
                terminal_snapshot = matches!(
                    terminal_item(&value),
                    crate::adaptor::presenter::client::terminal_event::Item::Snapshot(_)
                );
            }
            Some(StateSubscriptionEvent::Item(target, Event::Snapshot(_, _)))
                if target == "repository-paths" =>
            {
                paths_snapshot = true
            }
            Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_))) if target == raw => {
                terminal_bookmark = true
            }
            Some(StateSubscriptionEvent::Item(target, Event::Bookmark(_)))
                if target == "repository-paths" =>
            {
                paths_bookmark = true
            }
            event => panic!("unexpected subscription event: {}", event.is_some()),
        }
    }
    assert!(terminal_snapshot && paths_snapshot && terminal_bookmark && paths_bookmark);
    assert_eq!(gateway.snapshot_materialization_count(), before + 1);
    for sequence in 1..=100 {
        surface
            .record_output(surface.runtime_generation, std::time::Instant::now())
            .unwrap();
        gateway.insert_surface(surface.clone());
        hub.initialize(crate::test_support::state_subscription::registration(
            &surface.session_key,
            "/repo",
            None,
            1,
            sequence,
        ))
        .unwrap();
        hub.publish(TerminalSurfaceOutputEvent::Output {
            session_key: surface.session_key.clone(),
            data: "🙂".into(),
            sequence,
        });
    }
    assert_eq!(gateway.snapshot_materialization_count(), before + 1);
    for sequence in 1..=100 {
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(version, Delivery::Delta, value))) if version.sequence == sequence && matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(_)))
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
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        None,
        "input",
    )
    .await
    .unwrap();
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) = stream.next().await
    else {
        panic!("snapshot");
    };
    stream.next().await;
    let before = gateway.snapshot_materialization_count();
    crate::test_support::state_subscription::stop(&subscriptions, "client", &target).unwrap();
    surface
        .record_output(surface.runtime_generation, std::time::Instant::now())
        .unwrap();
    gateway.insert_surface(surface.clone());
    hub.initialize(crate::test_support::state_subscription::registration(
        &surface.session_key,
        "/repo",
        None,
        1,
        1,
    ))
    .unwrap();
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: surface.session_key.clone(),
        data: "next".into(),
        sequence: 1,
    });
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        Some((&version.epoch, version.sequence)),
        "input-2",
    )
    .await
    .unwrap();
    assert_eq!(gateway.snapshot_materialization_count(), before);
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(v, Delivery::Delta, _))) if v.sequence == 1)
    );
    stream.next().await;
    crate::test_support::state_subscription::stop(&subscriptions, "client", &target).unwrap();
    let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
    gateway.remove_surface(surface.runtime_generation.value());
    gateway.insert_surface(recreated.clone());
    hub.initialize(crate::test_support::state_subscription::registration(
        &recreated.session_key,
        "/repo",
        None,
        2,
        0,
    ))
    .unwrap();
    subscriptions
        .test_presenter()
        .unwrap()
        .initialize(&crate::test_support::state_subscription::registration(
            &recreated.session_key,
            "/repo",
            None,
            recreated.runtime_generation.value(),
            recreated.latest_sequence(),
        ))
        .unwrap();
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        Some((&version.epoch, version.sequence)),
        "input-3",
    )
    .await
    .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(v, _))) if v.epoch != version.epoch)
    );
    assert_eq!(gateway.snapshot_materialization_count(), before + 1);
}

#[tokio::test]
async fn test_terminal再開不能_配信開始直後の出力を現在snapshotで回復する() {
    // Given
    let (subscriptions, gateway, hub, mut surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone());
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let cursor = Some(("old-epoch", 0));
    subscriptions
        .start_subscription("client", &target, Some("input"), cursor)
        .await
        .unwrap();
    assert!(!hub.test_subscribed(&surface.session_key, "client"));
    assert_eq!(
        hub.test_pending_amount(&surface.session_key, "client"),
        None
    );

    // When
    surface
        .record_output(surface.runtime_generation, std::time::Instant::now())
        .unwrap();
    gateway.insert_surface(surface.clone());
    hub.initialize(crate::test_support::state_subscription::registration(
        &surface.session_key,
        "/repo",
        None,
        1,
        1,
    ))
    .unwrap();
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: surface.session_key.clone(),
        data: "next".into(),
        sequence: 1,
    });
    assert_eq!(
        hub.test_pending_amount(&surface.session_key, "client"),
        None
    );
    // Then
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap(),
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _)))
            if version.sequence == 1
    ));
    assert!(hub.test_subscribed(&surface.session_key, "client"));
    assert_eq!(
        hub.test_pending_amount(&surface.session_key, "client"),
        Some(0)
    );
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: surface.session_key.clone(),
        data: "x".repeat(100_001).into(),
        sequence: 2,
    });
    assert_eq!(
        hub.test_pending_amount(&surface.session_key, "client"),
        Some(100_001)
    );
}

#[tokio::test]
async fn test_terminal購読_件数上限がなく停止と切断で流量を解放する() {
    let (subscriptions, gateway, hub, surface) = fixture();
    let stream = subscriptions.open("client".into()).unwrap();
    for index in 0..20 {
        let owner =
            TerminalSurfaceOwner::workspace(WorkspaceIdentity::new(format!("/repo-{index}")))
                .unwrap();
        let surface = TerminalSurface::new(index + 2, owner.clone(), None);
        gateway.insert_surface(surface.clone());
        hub.initialize(crate::test_support::state_subscription::registration(
            &surface.session_key,
            &format!("/repo-{index}"),
            None,
            surface.runtime_generation.value(),
            surface.latest_sequence(),
        ))
        .unwrap();
        crate::test_support::state_subscription::start_terminal(
            &subscriptions,
            "client",
            &SubscriptionTarget::Terminal(owner).to_string(),
            None,
            "input",
        )
        .await
        .unwrap();
    }
    assert_eq!(
        subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.active_targets().len()),
        20
    );
    drop(stream);
    assert!(subscriptions
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.active_targets().is_empty()));
    assert!(crate::test_support::state_subscription::terminal_processed(
        &subscriptions,
        "client",
        &SubscriptionTarget::Terminal(surface.owner).to_string(),
        5000
    )
    .is_err());
}

#[tokio::test]
async fn test_terminal復元_同じ対象の全clientの流量停止を解放する() {
    let (subscriptions, _, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
    let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
    first.next().await;
    second.next().await;
    for client in ["first", "second"] {
        crate::test_support::state_subscription::start_terminal(
            &subscriptions,
            client,
            &target,
            None,
            client,
        )
        .await
        .unwrap();
    }
    first.next().await;
    first.next().await;
    second.next().await;
    second.next().await;
    for client in ["first", "second"] {
        hub.subscribe_output(
            &surface.session_key,
            client,
            crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1,
        );
    }
    subscriptions
        .test_presenter()
        .unwrap()
        .test_runtime()
        .update(|state| state.require_delta_snapshot(&target))
        .unwrap();

    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), first.next())
            .await
            .unwrap(),
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
    let (sender, receiver) = std::sync::mpsc::channel();
    let hub_for_wait = hub.clone();
    let session_key = surface.session_key.clone();
    let waiter = std::thread::spawn(move || {
        hub_for_wait.wait_output(&session_key);
        sender.send(()).unwrap();
    });
    let resumed = receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok();
    hub.unsubscribe_output(&surface.session_key, "first");
    hub.unsubscribe_output(&surface.session_key, "second");
    waiter.join().unwrap();
    assert!(resumed);
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
    hub.initialize(crate::test_support::state_subscription::registration(
        &first.session_key,
        "/first",
        None,
        1,
        0,
    ))
    .unwrap();
    hub.initialize(crate::test_support::state_subscription::registration(
        &second.session_key,
        "/second",
        None,
        2,
        0,
    ))
    .unwrap();
    let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub.clone(),
        ),
    );
    let subscriptions = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    subscriptions
        .test_presenter()
        .unwrap()
        .connect_terminal(&terminal)
        .unwrap();
    let subscriptions = subscriptions.with_terminal(terminal.clone());
    let mut first_stream = Box::pin(subscriptions.open("first-client".into()).unwrap());
    let mut second_stream = Box::pin(subscriptions.open("second-client".into()).unwrap());
    first_stream.next().await;
    second_stream.next().await;
    let (started, waiting) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    *gateway.snapshot_gate.lock() = Some((started, blocked));
    // When
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "first-client",
        &SubscriptionTarget::Terminal(first.owner).to_string(),
        None,
        "first-input",
    )
    .await
    .unwrap();
    let request = tokio::spawn(async move { first_stream.next().await });
    tokio::task::spawn_blocking(move || {
        waiting
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
    })
    .await
    .unwrap();
    assert!(gateway.snapshot_gate.try_lock().is_some());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::test_support::state_subscription::start_terminal(
            &subscriptions,
            "second-client",
            &SubscriptionTarget::Terminal(second.owner.clone()).to_string(),
            None,
            "second-input",
        ),
    )
    .await;
    // Then
    assert!(!request.is_finished());
    assert!(result.unwrap().is_ok());
    assert_eq!(
        subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.snapshot_requests("second-client")),
        vec![SubscriptionTarget::Terminal(second.owner.clone()).to_string()]
    );
    subscriptions.schedule_terminal_refresh(
        vec!["second-client".into()],
        SubscriptionTarget::Terminal(second.owner.clone()),
    );
    assert_eq!(subscriptions.test_worker_count(), 2);
    tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
        .await
        .expect("second snapshot");
    tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
        .await
        .expect("second bookmark");
    terminal.resize(&second.owner, 40, 120).unwrap();
    terminal
        .write_attached(&second.owner, "second-input", 0, None, "input")
        .unwrap();
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: second.session_key,
        data: "live".into(),
        sequence: 1,
    });
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), second_stream.next())
            .await
            .expect("second output"),
        Some(StateSubscriptionEvent::Item(
            _,
            Event::Change(_, Delivery::Delta, _)
        ))
    ));
    release.send(()).unwrap();
    assert!(matches!(
        request.await.unwrap(),
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
}

#[tokio::test]
async fn test_snapshot作成中_同じterminalへ追加されたclientにもsnapshotを届ける() {
    // Given
    use crate::usecase::terminal_surface::io_usecase::io_usecase_tests::FakePtyGateway;
    let mut gateway = FakePtyGateway::new();
    let surface = TerminalSurface::new(
        1,
        TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
        None,
    );
    gateway.additional_surfaces = vec![surface.clone()];
    let gateway = Arc::new(gateway);
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    hub.initialize(crate::test_support::state_subscription::registration(
        &surface.session_key,
        "/repo",
        None,
        1,
        0,
    ))
    .unwrap();
    let terminal = Arc::new(
        crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            gateway.clone(),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            hub,
        ),
    );
    let subscriptions = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    subscriptions
        .test_presenter()
        .unwrap()
        .connect_terminal(&terminal)
        .unwrap();
    let subscriptions = subscriptions.with_terminal(terminal);
    let target = SubscriptionTarget::Terminal(surface.owner);
    let mut first = Box::pin(subscriptions.open("first".into()).unwrap());
    let mut second = Box::pin(subscriptions.open("second".into()).unwrap());
    first.next().await;
    second.next().await;
    let (started, waiting) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    *gateway.snapshot_gate.lock() = Some((started, blocked));

    // When
    subscriptions
        .start_terminal("first", &target, "first-input", None)
        .await
        .unwrap();
    let first_event = tokio::spawn(async move { first.next().await });
    tokio::task::spawn_blocking(move || {
        waiting
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
    })
    .await
    .unwrap();
    subscriptions
        .start_terminal("second", &target, "second-input", None)
        .await
        .unwrap();
    subscriptions.schedule_terminal_refresh(vec!["second".into()], target.clone());
    assert_eq!(subscriptions.test_worker_count(), 1);
    release.send(()).unwrap();

    // Then
    assert!(matches!(
        first_event.await.unwrap(),
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), second.next())
            .await
            .unwrap(),
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
}

#[tokio::test]
async fn test_terminal復元_停止と切断の競合でも停止済みclientを再登録しない() {
    use crate::usecase::terminal_surface::io_usecase::io_usecase_tests::FakePtyGateway;

    for close in [false, true] {
        // Given
        let surface = TerminalSurface::new(
            1,
            TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap(),
            None,
        );
        let mut gateway = FakePtyGateway::new();
        gateway.additional_surfaces.push(surface.clone());
        let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
        hub.initialize(crate::test_support::state_subscription::registration(
            &surface.session_key,
            "/repo",
            None,
            1,
            0,
        ))
        .unwrap();
        let (started, waiting) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let output = Arc::new(BlockingResetOutput {
            hub: hub.clone(),
            started,
            release: std::sync::Mutex::new(blocked),
        });
        let terminal = Arc::new(crate::usecase::terminal_surface::application::TerminalSurfaceApplication::new(
            Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
            Arc::new(gateway),
            Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
            output,
        ));
        let subscriptions = StateSubscriptionUsecase::new(
            vec![],
            Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
        );
        subscriptions
            .test_presenter()
            .unwrap()
            .connect_terminal(&terminal)
            .unwrap();
        let subscriptions = subscriptions.with_terminal(terminal);
        let target = SubscriptionTarget::Terminal(surface.owner.clone());
        let _stopping = subscriptions.open("stopping".into()).unwrap();
        let _active = subscriptions.open("active".into()).unwrap();
        subscriptions
            .start_terminal("stopping", &target, "stopping-input", None)
            .await
            .unwrap();
        subscriptions
            .start_terminal("active", &target, "active-input", None)
            .await
            .unwrap();
        hub.subscribe_output(
            &surface.session_key,
            "active",
            crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1,
        );

        // When
        subscriptions
            .schedule_terminal_refresh(vec!["stopping".into(), "active".into()], target.clone());
        tokio::task::spawn_blocking(move || {
            waiting
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        })
        .await
        .unwrap();
        let stopping = subscriptions.clone();
        let stopped_target = target.clone();
        let (stop_started, started_stop) = tokio::sync::oneshot::channel();
        let mut stop = tokio::task::spawn_blocking(move || {
            let _ = stop_started.send(());
            if close {
                stopping.close_client("stopping");
            } else {
                stopping.stop("stopping", &stopped_target).unwrap();
            }
        });
        started_stop.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut stop)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        stop.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while subscriptions.test_worker_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        // Then
        assert!(!hub.test_subscribed(&surface.session_key, "stopping"));
        assert!(hub.test_subscribed(&surface.session_key, "active"));
        let units = crate::infrastructure::terminal::output_flow_control::OUTPUT_HIGH_WATERMARK + 1;
        hub.publish(TerminalSurfaceOutputEvent::Output {
            session_key: surface.session_key.clone(),
            data: "x".repeat(units).into(),
            sequence: 1,
        });
        hub.processed_output(&surface.session_key, "active", units);
        let (resumed, wait) = std::sync::mpsc::channel();
        let session_key = surface.session_key.clone();
        let waiting_hub = hub.clone();
        let waiter = std::thread::spawn(move || {
            waiting_hub.wait_output(&session_key);
            resumed.send(()).unwrap();
        });
        let active_resumed = wait.recv_timeout(std::time::Duration::from_secs(2)).is_ok();
        hub.unsubscribe_output(&surface.session_key, "active");
        hub.unsubscribe_output(&surface.session_key, "stopping");
        waiter.join().unwrap();
        assert!(active_resumed);
    }
}

#[tokio::test]
async fn test_terminal差分_出力の重複を除き同じ出力番号で寸法と終了を配信する() {
    let (subscriptions, _, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    assert!(crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        None,
        ""
    )
    .await
    .is_err());
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        None,
        "input",
    )
    .await
    .unwrap();
    stream.next().await;
    stream.next().await;
    assert!(crate::test_support::state_subscription::terminal_processed(
        &subscriptions,
        "client",
        &target,
        1
    )
    .is_err());
    assert!(crate::test_support::state_subscription::terminal_processed(
        &subscriptions,
        "client",
        &target,
        5000
    )
    .is_ok());
    let output = TerminalSurfaceOutputEvent::Output {
        session_key: surface.session_key.clone(),
        data: "x".into(),
        sequence: 1,
    };
    hub.publish(output.clone());
    hub.publish(output);
    hub.publish(TerminalSurfaceOutputEvent::Resize {
        session_key: surface.session_key.clone(),
        cols: 120,
        rows: 30,
        sequence: 1,
    });
    hub.publish(TerminalSurfaceOutputEvent::Exit {
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
        assert_eq!(
            *value,
            crate::adaptor::presenter::state_subscription_wire::payload(&StateValue::Terminal(
                expected.into()
            ))
            .unwrap()
        );
    }
    crate::test_support::state_subscription::stop(&subscriptions, "client", &target).unwrap();
    assert!(crate::test_support::state_subscription::terminal_processed(
        &subscriptions,
        "client",
        &target,
        5000
    )
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
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        None,
        "input",
    )
    .await
    .unwrap();
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(version, _))) = stream.next().await
    else {
        panic!("snapshot");
    };
    assert_eq!(version.sequence, 0);
    stream.next().await;
    // When
    hub.publish(TerminalSurfaceOutputEvent::Resize {
        session_key: surface.session_key.clone(),
        cols: 120,
        rows: 30,
        sequence: 0,
    });
    hub.publish(TerminalSurfaceOutputEvent::Exit {
        session_key: surface.session_key.clone(),
        runtime_generation: 1,
        exit_code: Some(7),
        sequence: 0,
    });
    // Then
    for reconnect in [false, true] {
        if reconnect {
            crate::test_support::state_subscription::stop(&subscriptions, "client", &target)
                .unwrap();
            crate::test_support::state_subscription::start_terminal(
                &subscriptions,
                "client",
                &target,
                Some((&version.epoch, version.sequence)),
                "input",
            )
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
            terminal_item(&resize),
            crate::adaptor::presenter::client::terminal_event::Item::Resize(
                crate::adaptor::presenter::client::TerminalResize {
                    cols: 120,
                    rows: 30,
                    sequence: 0,
                    ..
                }
            )
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
            terminal_item(&exit),
            crate::adaptor::presenter::client::terminal_event::Item::Exit(
                crate::adaptor::presenter::client::TerminalExit {
                    exit_code: Some(7),
                    sequence: 0,
                    ..
                }
            )
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
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        None,
        "input",
    )
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
    hub.initialize(crate::test_support::state_subscription::registration(
        &surface.session_key,
        "/repo",
        None,
        1,
        1,
    ))
    .unwrap();
    // When
    hub.publish(TerminalSurfaceOutputEvent::Output {
        session_key: surface.session_key.clone(),
        sequence: 1,
        data: "x".into(),
    });
    hub.publish(TerminalSurfaceOutputEvent::Resize {
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
        matches!(terminal_item(&snapshot), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(value) if value.cols == 120 && value.rows == 30)
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
        hub.initialize(crate::test_support::state_subscription::registration(
            &surface.session_key,
            "/repo",
            None,
            generation,
            0,
        ))
        .unwrap();
        crate::test_support::state_subscription::start_terminal(
            &subscriptions,
            "client",
            &target,
            None,
            "input",
        )
        .await
        .unwrap();
        stream.next().await;
        stream.next().await;
        hub.publish(TerminalSurfaceOutputEvent::Output {
            session_key: surface.session_key.clone(),
            data: "retained".into(),
            sequence: 1,
        });
        hub.publish(TerminalSurfaceOutputEvent::Exit {
            session_key: surface.session_key.clone(),
            runtime_generation: generation,
            exit_code: Some(0),
            sequence: 1,
        });
        gateway.remove_surface(generation).unwrap();
        // Then
        assert!(
            subscriptions
                .test_presenter()
                .unwrap()
                .test_runtime()
                .test_terminal_route_count()
                == 0
        );
        let runtime = &subscriptions.test_presenter().unwrap().test_runtime();
        assert_eq!(runtime.inspect(|state| state.active_targets().len()), 1);
        assert!(runtime.inspect(|state| state.current_version(&target).is_none()));
        runtime.mutate(|state| (state.bookmark("client"), true));
        assert!(runtime.inspect(|state| state.snapshot_requests("client").is_empty()));
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Output(_)))
        );
        assert!(
            matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Exit(event) if event.exit_code == Some(0)))
        );
        assert!(subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.is_subscribed("client", &target)));
        assert_eq!(
            subscriptions
                .test_presenter()
                .as_ref()
                .unwrap()
                .test_runtime()
                .inspect(|state| state.active_targets().len()),
            1
        );
        crate::test_support::state_subscription::stop(&subscriptions, "client", &target).unwrap();
        assert!(subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.active_targets().is_empty()));
        assert!(subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.active_targets().is_empty()));
    }
}

#[tokio::test]
async fn test_terminal購読開始_古いsummary取得後の再作成でepochを巻き戻さない() {
    // Given
    let (subscriptions, gateway, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone()).to_string();
    let stream = subscriptions.open("client".into()).unwrap();
    tokio::pin!(stream);
    stream.next().await;
    let old_version = subscriptions
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.current_version(&target))
        .unwrap();
    let recreated = TerminalSurface::new(2, surface.owner.clone(), None);
    let replacement = recreated.clone();
    let replacement_key = recreated.session_key.clone();
    let gateway_for_replacement = gateway.clone();
    let hub_for_replacement = hub.clone();
    *gateway.before_output_order.lock() = Some(Box::new(move || {
        gateway_for_replacement.remove_surface(1).unwrap();
        gateway_for_replacement.insert_surface(replacement);
        hub_for_replacement
            .initialize(crate::test_support::state_subscription::registration(
                &replacement_key,
                "/repo",
                None,
                2,
                0,
            ))
            .unwrap();
    }));
    // When
    crate::test_support::state_subscription::start_terminal(
        &subscriptions,
        "client",
        &target,
        Some((&old_version.epoch, old_version.sequence)),
        "new-input",
    )
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
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.current_version(&target))
            .unwrap()
    );
    assert!(
        matches!(terminal_item(&value), crate::adaptor::presenter::client::terminal_event::Item::Snapshot(surface) if surface.session_key == recreated.session_key)
    );
    subscriptions
        .test_presenter()
        .unwrap()
        .remove(&surface.session_key, surface.runtime_generation.value());
    assert_eq!(
        subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .inspect(|state| state.current_version(&target)),
        Some(version)
    );
    assert_eq!(
        subscriptions
            .test_presenter()
            .unwrap()
            .test_runtime()
            .test_terminal_route_count(),
        1
    );
}

#[tokio::test]
async fn test_terminal購読開始_出力順序区間内でsummary取得に失敗したら開始しない() {
    // Given
    let (subscriptions, gateway, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone());
    let session_key = surface.session_key.clone();
    let _stream = subscriptions.open("client".into()).unwrap();
    let removed = gateway.clone();
    *gateway.during_output_order.lock() = Some(Box::new(move || {
        removed
            .remove_surface(surface.runtime_generation.value())
            .unwrap();
    }));

    // When
    let result = subscriptions
        .start_subscription("client", &target, Some("input"), None)
        .await;

    // Then
    assert!(result.is_err());
    assert!(subscriptions.active_targets().is_empty());
    assert!(!hub.test_subscribed(&session_key, "client"));
    assert!(!subscriptions
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.is_subscribed("client", &target.to_string())));
}

#[tokio::test]
async fn test_terminal購読開始_途中でclientが切断したら出力登録を巻き戻す() {
    // Given
    let (subscriptions, gateway, hub, surface) = fixture();
    let target = SubscriptionTarget::Terminal(surface.owner.clone());
    let _stream = subscriptions.open("client".into()).unwrap();
    let disconnected = subscriptions.clone();
    *gateway.before_output_order.lock() =
        Some(Box::new(move || disconnected.close_client("client")));
    // When
    let result = subscriptions
        .start_subscription("client", &target, Some("input"), None)
        .await;
    // Then
    assert!(
        matches!(result, Err(crate::usecase::state_subscription::StateReadError {
        source: crate::usecase::state_subscription::StateReadFailure::Subscription(error), ..
    }) if *error == crate::usecase::state_subscription::SubscriptionError::StreamEnded)
    );
    assert!(!hub.test_subscribed(&surface.session_key, "client"));
    assert!(subscriptions.active_targets().is_empty());
    assert!(!subscriptions
        .test_presenter()
        .unwrap()
        .test_runtime()
        .inspect(|state| state.is_subscribed("client", &target.to_string())));
}
