use std::sync::Arc;
use std::time::Duration;

use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceEventSource,
};

use super::TerminalSurfaceEventHub;

fn output_event(sequence: u64, data: &str) -> TerminalSurfaceEvent {
    TerminalSurfaceEvent::Output {
        session_key: "session".to_string(),
        data: data.into(),
        sequence,
    }
}

#[test]
fn test_global_broadcastへはexitだけが流れoutputやresizeは流れない() {
    let hub = TerminalSurfaceEventHub::with_flags(8, true);
    let mut global = hub.sender.subscribe();

    hub.publish(output_event(1, "x"));
    hub.publish(TerminalSurfaceEvent::Resize {
        session_key: "session".to_string(),
        cols: 80,
        rows: 24,
        sequence: 2,
    });
    assert!(matches!(
        global.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    hub.publish(TerminalSurfaceEvent::Exit {
        session_key: "session".to_string(),
        runtime_generation: 1,
        exit_code: Some(0),
        sequence: 3,
    });
    assert!(matches!(
        global.try_recv(),
        Ok(TerminalSurfaceEvent::Exit { sequence: 3, .. })
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
    hub.publish(TerminalSurfaceEvent::Resize {
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
    use crate::domain::terminal_surface::entities::{TerminalSurface, TerminalSurfaceSummary};
    use crate::domain::terminal_surface::gateway::TerminalSurfaceStateSink;
    use crate::domain::terminal_surface::TerminalSurfaceOwner;
    use crate::domain::workspace_tree::WorkspaceIdentity;

    struct StateSink(bool);
    impl TerminalSurfaceStateSink for StateSink {
        fn initialize(&self, _: &TerminalSurfaceSummary) {}
        fn remove(&self, _: &TerminalSurfaceSummary) -> bool {
            self.0
        }
        fn publish(&self, _: TerminalSurfaceEvent) {}
    }

    for subscribed in [false, true] {
        // Given
        let hub = Arc::new(TerminalSurfaceEventHub::with_flags(8, true));
        hub.set_state_sink(Arc::new(StateSink(subscribed)));
        let owner = TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap();
        let surface = TerminalSurface::new(1, owner, None).summary();
        hub.subscribe_output(&surface.session_key, "client", 0);
        hub.publish(TerminalSurfaceEvent::Output {
            session_key: surface.session_key.clone(),
            sequence: 1,
            data: "x".repeat(100_001).into(),
        });
        let pause = hub.output.lock()[&surface.session_key].1.clone();
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
        assert_eq!(hub.remove(&surface), subscribed);
        // Then
        let completed = receiver.recv_timeout(Duration::from_secs(1));
        pause.set(false);
        worker.join().unwrap();
        completed.unwrap();
        assert!(!hub.output.lock().contains_key(&surface.session_key));
    }
}
