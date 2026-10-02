use std::sync::Arc;
use std::time::Duration;

use crate::usecase::terminal_surface::output::TerminalSurfaceEventSink;
use crate::usecase::terminal_surface::output::TerminalSurfaceOutputControl;
use crate::usecase::terminal_surface::output::TerminalSurfaceOutputEvent;
use crate::usecase::terminal_surface::output::TerminalSurfaceStateSink;

use super::TerminalSurfaceEventHub;

fn output_event(sequence: u64, data: &str) -> TerminalSurfaceOutputEvent {
    TerminalSurfaceOutputEvent::Output {
        session_key: "session".to_string(),
        data: data.into(),
        sequence,
    }
}

#[test]
fn test_ターミナル状態接続_接続前の登録だけを再生し世代による削除を届ける() {
    struct RecordingStateSink {
        initialized: std::sync::Mutex<Vec<(String, String, Option<String>, u64, u64)>>,
        removed: std::sync::Mutex<Vec<(String, u64)>>,
    }
    impl TerminalSurfaceStateSink for RecordingStateSink {
        fn initialize(
            &self,
            registration: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
            self.initialized.lock().unwrap().push((
                registration.session_key.clone(),
                registration.workspace_path.clone(),
                registration.session_id.clone(),
                registration.runtime_generation,
                registration.latest_sequence,
            ));
            Ok(())
        }
        fn remove(
            &self,
            registration: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> bool {
            self.removed.lock().unwrap().push((
                registration.session_key.clone(),
                registration.runtime_generation,
            ));
            true
        }
        fn publish(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
            _: TerminalSurfaceOutputEvent,
        ) {
        }
    }

    // Given
    let hub = TerminalSurfaceEventHub::with_flags(8, true);
    hub.initialize(crate::test_support::state_subscription::registration(
        "old", "/repo", None, 1, 0,
    ))
    .unwrap();
    assert!(!hub.remove(1));
    hub.initialize(crate::test_support::state_subscription::registration(
        "session",
        "/repo",
        Some("agent"),
        2,
        7,
    ))
    .unwrap();
    let sink = Arc::new(RecordingStateSink {
        initialized: std::sync::Mutex::new(Vec::new()),
        removed: std::sync::Mutex::new(Vec::new()),
    });

    // When
    hub.set_state_sink(sink.clone()).unwrap();
    assert!(hub.remove(2));

    // Then
    assert_eq!(
        *sink.initialized.lock().unwrap(),
        vec![("session".into(), "/repo".into(), Some("agent".into()), 2, 7)]
    );
    assert_eq!(*sink.removed.lock().unwrap(), vec![("session".into(), 2)]);
}

#[test]
fn test_ターミナル登録_配信対象の登録失敗を返しhubに残さない() {
    struct RejectingStateSink;
    impl TerminalSurfaceStateSink for RejectingStateSink {
        fn initialize(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
            Err(
                crate::usecase::terminal_surface::error::UsecaseError::InvalidOperation(
                    "invalid target".into(),
                ),
            )
        }
        fn remove(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> bool {
            false
        }
        fn publish(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
            _: TerminalSurfaceOutputEvent,
        ) {
        }
    }

    // Given
    let hub = TerminalSurfaceEventHub::with_flags(8, true);
    hub.set_state_sink(Arc::new(RejectingStateSink)).unwrap();

    // When
    let result = hub.initialize(crate::test_support::state_subscription::registration(
        "session", "/repo", None, 1, 0,
    ));

    // Then
    assert!(result.is_err());
    assert!(!hub.remove(1));

    let replay = TerminalSurfaceEventHub::with_flags(8, true);
    replay
        .initialize(crate::test_support::state_subscription::registration(
            "session", "/repo", None, 1, 0,
        ))
        .unwrap();
    assert!(replay.set_state_sink(Arc::new(RejectingStateSink)).is_err());
}

#[test]
fn test_global_broadcastへはexitだけが流れoutputやresizeは流れない() {
    let hub = TerminalSurfaceEventHub::with_flags(8, true);
    let mut global = hub.sender.subscribe();

    hub.publish(output_event(1, "x"));
    hub.publish(TerminalSurfaceOutputEvent::Resize {
        session_key: "session".to_string(),
        cols: 80,
        rows: 24,
        sequence: 2,
    });
    assert!(matches!(
        global.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    hub.publish(TerminalSurfaceOutputEvent::Exit {
        session_key: "session".to_string(),
        runtime_generation: 1,
        exit_code: Some(0),
        sequence: 3,
    });
    assert!(matches!(
        global.try_recv(),
        Ok(
            crate::domain::terminal_surface::gateway::TerminalSurfaceEvent::Exit {
                sequence: 3,
                ..
            }
        )
    ));
}

#[test]
fn test_流量停止_配信を塞がず最も遅い購読が処理すると出力元を再開する() {
    // Given
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
    hub.subscribe_output("session", "fast", 0);
    hub.subscribe_output("session", "slow", 0);
    hub.publish(output_event(1, &"x".repeat(100_001)));
    let (done, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn({
        let hub = hub.clone();
        move || {
            hub.wait_output("session");
            done.send(()).unwrap();
        }
    });
    // When
    hub.processed_output("session", "fast", 100_000);
    hub.wait_output("another-session");
    hub.publish(TerminalSurfaceOutputEvent::Resize {
        session_key: "session".into(),
        cols: 100,
        rows: 30,
        sequence: 2,
    });
    // Then
    assert!(receiver.recv_timeout(Duration::from_millis(30)).is_err());
    hub.processed_output("session", "slow", 100_000);
    receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
}

#[test]
fn test_購読解除_停止中の出力元を解放する() {
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
    hub.subscribe_output("session", "client", 0);
    hub.publish(output_event(1, &"x".repeat(100_001)));
    let (done, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn({
        let hub = hub.clone();
        move || {
            hub.wait_output("session");
            done.send(()).unwrap();
        }
    });
    hub.unsubscribe_output("session", "client");
    receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
}

#[test]
fn test_流量制御無効_高水位を超える履歴再開と後続出力でも停止しない() {
    // Given
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, false));
    // When
    hub.subscribe_output("session", "client", 150_000);
    hub.publish(output_event(1, &"x".repeat(100_001)));
    hub.processed_output("session", "client", 5_000);
    let (done, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn({
        let hub = hub.clone();
        move || {
            hub.wait_output("session");
            done.send(()).unwrap();
        }
    });
    // Then
    let completed = receiver.recv_timeout(Duration::from_secs(1));
    hub.release_output("session");
    worker.join().unwrap();
    completed.unwrap();
}

#[test]
fn test_ターミナル削除_購読の有無によらず停止中の出力元を解放する() {
    use crate::domain::terminal_surface::entities::TerminalSurface;
    use crate::domain::terminal_surface::TerminalSurfaceOwner;
    use crate::domain::workspace_tree::WorkspaceIdentity;
    use crate::usecase::terminal_surface::output::TerminalSurfaceStateSink;

    struct StateSink(bool);
    impl TerminalSurfaceStateSink for StateSink {
        fn initialize(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
            Ok(())
        }
        fn remove(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> bool {
            self.0
        }
        fn publish(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
            _: TerminalSurfaceOutputEvent,
        ) {
        }
    }

    for subscribed in [false, true] {
        // Given
        let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
        hub.set_state_sink(Arc::new(StateSink(subscribed))).unwrap();
        let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
        let surface = TerminalSurface::new(1, owner, None).summary();
        hub.initialize(crate::test_support::state_subscription::registration(
            &surface.session_key,
            "/repo",
            None,
            1,
            0,
        ))
        .unwrap();
        hub.subscribe_output(&surface.session_key, "client", 0);
        hub.publish(TerminalSurfaceOutputEvent::Output {
            session_key: surface.session_key.clone(),
            sequence: 1,
            data: "x".repeat(100_001).into(),
        });
        let pause = hub.output.test_pause(&surface.session_key).unwrap();
        let (done, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn({
            let pause = pause.clone();
            move || {
                pause.wait();
                done.send(()).unwrap();
            }
        });
        assert!(receiver.recv_timeout(Duration::from_millis(30)).is_err());
        // When
        assert_eq!(hub.remove(surface.runtime_generation.value()), subscribed);
        // Then
        let completed = receiver.recv_timeout(Duration::from_secs(1));
        pause.set(false);
        worker.join().unwrap();
        completed.unwrap();
        assert!(hub.output.test_pause(&surface.session_key).is_none());
    }
}

#[test]
fn test_terminal再接続_持ち主ごとの最新世代だけを登録する() {
    struct Sink(std::sync::Mutex<Vec<u64>>);
    impl TerminalSurfaceStateSink for Sink {
        fn initialize(
            &self,
            registration: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
            self.0.lock().unwrap().push(registration.runtime_generation);
            Ok(())
        }
        fn remove(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> bool {
            false
        }
        fn publish(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
            _: TerminalSurfaceOutputEvent,
        ) {
        }
    }
    // Given
    let hub = TerminalSurfaceEventHub::new();
    for generation in [3, 1, 2] {
        hub.initialize(crate::test_support::state_subscription::registration(
            "session", "/repo", None, generation, 0,
        ))
        .unwrap();
    }
    hub.initialize(crate::test_support::state_subscription::registration(
        "other", "/other", None, 4, 0,
    ))
    .unwrap();
    let sink = Arc::new(Sink(std::sync::Mutex::new(vec![])));
    // When
    hub.set_state_sink(sink.clone()).unwrap();
    // Then
    let mut initialized = sink.0.lock().unwrap().clone();
    initialized.sort();
    assert_eq!(initialized, vec![3, 4]);
}

#[test]
fn test_terminal経路_現在と同じ番号の旧世代終了を届け最新削除後は復活しない() {
    use crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter;
    use crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter;
    use crate::usecase::state_subscription::SubscriptionTarget;
    // Given
    let hub = TerminalSurfaceEventHub::new();
    let presenter = Arc::new(TerminalSubscriptionPresenter::new(
        &StateSubscriptionPresenter::new(),
    ));
    hub.set_state_sink(presenter.clone()).unwrap();
    for generation in [1, 2, 3] {
        hub.initialize(crate::test_support::state_subscription::registration(
            "session", "/repo", None, generation, 0,
        ))
        .unwrap();
    }
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"])
        .unwrap()
        .to_string();
    let version = presenter
        .test_runtime()
        .inspect(|state| state.current_version(&target))
        .unwrap();
    assert!(version.epoch.ends_with(":3"));
    assert_eq!(version.sequence, 0);
    // When / Then
    assert!(!hub.remove(2));
    assert_eq!(
        presenter
            .test_runtime()
            .inspect(|state| state.current_version(&target)),
        Some(version.clone())
    );
    presenter
        .test_runtime()
        .update(|state| {
            state.open("client".into())?;
            state.start("client", &target, &target, Some(&version))?;
            assert!(matches!(
                state.next("client"),
                Some((
                    _,
                    crate::infrastructure::state_subscription::Event::Bookmark(_)
                ))
            ));
            Ok(true)
        })
        .unwrap();
    hub.publish(TerminalSurfaceOutputEvent::Exit {
        session_key: "session".into(),
        runtime_generation: 1,
        sequence: 0,
        exit_code: Some(9),
    });
    let event = presenter
        .test_runtime()
        .mutate(|state| (state.next("client"), false))
        .unwrap();
    assert!(matches!(event,
        (_, crate::infrastructure::state_subscription::Event::Change(delivered, crate::infrastructure::state_subscription::Delivery::Delta, value))
        if delivered == version && delivered.sequence == 0
        && matches!(crate::test_support::state_subscription::terminal_item(&value),
            crate::adaptor::presenter::client::terminal_event::Item::Exit(exit) if exit.exit_code == Some(9))));
    presenter
        .test_runtime()
        .mutate(|state| (state.close("client"), false));
    assert!(!hub.remove(3));
    hub.publish(output_event(1, "old"));
    assert!(presenter
        .test_runtime()
        .inspect(|state| state.current_version(&target).is_none()));
}

#[test]
fn test_terminal配信_登録表と接続先のlockを放して最新世代へ届ける() {
    struct Sink {
        hub: std::sync::Weak<TerminalSurfaceEventHub>,
        published: std::sync::Mutex<Vec<u64>>,
    }
    impl TerminalSurfaceStateSink for Sink {
        fn initialize(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> Result<(), crate::usecase::terminal_surface::error::UsecaseError> {
            Ok(())
        }
        fn remove(
            &self,
            _: &crate::usecase::terminal_surface::output::TerminalRegistration,
        ) -> bool {
            false
        }
        fn publish(
            &self,
            registration: &crate::usecase::terminal_surface::output::TerminalRegistration,
            _: TerminalSurfaceOutputEvent,
        ) {
            let hub = self.hub.upgrade().unwrap();
            assert!(hub.registrations.try_lock().is_some());
            assert!(hub.state_sink.try_lock().is_some());
            self.published
                .lock()
                .unwrap()
                .push(registration.runtime_generation);
        }
    }
    // Given
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let sink = Arc::new(Sink {
        hub: Arc::downgrade(&hub),
        published: Default::default(),
    });
    hub.set_state_sink(sink.clone()).unwrap();
    for (session, generation) in [("session", 1), ("other", 2), ("session", 3)] {
        hub.initialize(crate::test_support::state_subscription::registration(
            session, "/repo", None, generation, 0,
        ))
        .unwrap();
    }
    // When
    hub.publish(output_event(1, "output"));
    // Then
    assert_eq!(*sink.published.lock().unwrap(), vec![3]);
}
