use std::sync::{Arc, Mutex};

use super::*;

#[derive(Default)]
struct RecordingEventSink {
    events: Mutex<Vec<TerminalSurfaceEvent>>,
}

impl TerminalSurfaceEventSink for RecordingEventSink {
    fn publish(&self, event: TerminalSurfaceEvent) {
        self.events.lock().unwrap().push(event);
    }
}

fn output(sequence: u64) -> TerminalSurfaceEvent {
    TerminalSurfaceEvent::Output {
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
            TerminalSurfaceEvent::Output { sequence, .. }
            | TerminalSurfaceEvent::Resize { sequence, .. }
            | TerminalSurfaceEvent::Exit { sequence, .. } => *sequence,
            TerminalSurfaceEvent::InputUnavailable { .. } => {
                panic!("fault relay sequence fixture emitted an input failure")
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![2, 2, 4, 3]);
}
