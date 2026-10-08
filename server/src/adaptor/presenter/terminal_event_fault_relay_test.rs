use std::sync::{Arc, Mutex};

use super::*;

#[derive(Default)]
struct RecordingEventSink {
    events: Mutex<Vec<TerminalSurfaceOutputEvent>>,
    removed: Mutex<Vec<u64>>,
}

impl TerminalSurfaceEventSink for RecordingEventSink {
    fn remove(&self, runtime_generation: u64) -> bool {
        self.removed.lock().unwrap().push(runtime_generation);
        true
    }

    fn publish(&self, event: TerminalSurfaceOutputEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn output(sequence: u64) -> TerminalSurfaceOutputEvent {
    TerminalSurfaceOutputEvent::Output {
        session_key: "surface".to_string(),
        data: format!("chunk-{sequence}").into(),
        sequence,
    }
}

#[test]
fn test_ターミナル画面fault中継_次イベントの欠落重複逆転を指定どおり注入する() {
    let recorded = Arc::new(RecordingEventSink::default());
    let target: Arc<dyn TerminalSurfaceEventSink> = recorded.clone();
    let (sink, faults) = fault_injecting_event_sink(target);

    faults.arm(TerminalSurfaceEventFault::DropNext);
    sink.publish(output(1));
    faults.arm(TerminalSurfaceEventFault::DuplicateNext);
    sink.publish(output(2));
    faults.arm(TerminalSurfaceEventFault::ReverseNextTwo);
    sink.publish(output(3));
    sink.publish(output(4));

    let sequences = recorded
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|event| match event {
            TerminalSurfaceOutputEvent::Output { sequence, .. }
            | TerminalSurfaceOutputEvent::Resize { sequence, .. }
            | TerminalSurfaceOutputEvent::Exit { sequence, .. } => *sequence,
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![2, 2, 4, 3]);
}

#[test]
fn test_ターミナル画面fault中継_削除はfault指定によらず中継する() {
    // Given
    let recorded = Arc::new(RecordingEventSink::default());
    let (sink, faults) = fault_injecting_event_sink(recorded.clone());
    for fault in [
        TerminalSurfaceEventFault::DropNext,
        TerminalSurfaceEventFault::DuplicateNext,
        TerminalSurfaceEventFault::ReverseNextTwo,
    ] {
        // When
        faults.arm(fault);
        assert!(sink.remove(1));
    }

    // Then
    assert_eq!(*recorded.removed.lock().unwrap(), vec![1; 3]);
}
