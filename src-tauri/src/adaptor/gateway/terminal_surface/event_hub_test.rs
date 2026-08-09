use std::sync::Arc;
use std::time::{Duration, Instant};

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
    hub.publish(TerminalSurfaceEvent::InputUnavailable {
        session_key: "session".to_string(),
        message: "unavailable".to_string(),
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

#[tokio::test]
async fn test_owner購読があってもexitはownerとglobalの両方へ流れoutputはownerだけに流れる() {
    let hub = TerminalSurfaceEventHub::with_flags(8, true);
    let mut global = hub.sender.subscribe();
    let mut owner = hub.subscribe_owner("session", "attachment");

    hub.publish(output_event(1, "x"));
    hub.publish(TerminalSurfaceEvent::Exit {
        session_key: "session".to_string(),
        runtime_generation: 1,
        exit_code: Some(0),
        sequence: 2,
    });

    assert!(matches!(
        owner.subscription.recv().await,
        Ok(TerminalSurfaceEvent::Output { sequence: 1, .. })
    ));
    assert!(matches!(
        owner.subscription.recv().await,
        Ok(TerminalSurfaceEvent::Exit { sequence: 2, .. })
    ));
    assert!(matches!(
        global.try_recv(),
        Ok(TerminalSurfaceEvent::Exit { sequence: 2, .. })
    ));
    assert!(matches!(
        global.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[test]
fn test_flow_control無効時は未ack超過でもpublishがブロックしない() {
    let hub = TerminalSurfaceEventHub::with_flags(8, false);
    let _attachment = hub.subscribe_owner("session", "attachment");
    let hub = Arc::new(hub);

    let worker = {
        let hub = Arc::clone(&hub);
        std::thread::spawn(move || {
            let data = "x".repeat(300 * 1024);
            hub.publish(output_event(1, &data));
            hub.publish(output_event(2, &data));
        })
    };

    let started_at = Instant::now();
    while !worker.is_finished() {
        assert!(
            started_at.elapsed() < Duration::from_secs(5),
            "publish must not block on credit when flow control is disabled"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    worker.join().expect("publisher thread must finish");
}
