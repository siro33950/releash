use std::sync::Arc;

use crate::domain::terminal_surface::gateway::{TerminalSurfaceEvent, TerminalSurfaceEventSink};

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceEventFault {
    DropNext,
    DuplicateNext,
    ReverseNextTwo,
}

#[doc(hidden)]
#[derive(Clone)]
pub struct TerminalSurfaceEventFaultController {
    state: Arc<parking_lot::Mutex<TerminalSurfaceEventFaultState>>,
}

#[derive(Default)]
struct TerminalSurfaceEventFaultState {
    armed: Option<TerminalSurfaceEventFault>,
    held: Option<TerminalSurfaceEvent>,
}

impl TerminalSurfaceEventFaultController {
    pub fn arm(&self, fault: TerminalSurfaceEventFault) {
        let mut state = self.state.lock();
        state.armed = Some(fault);
        state.held = None;
    }
}

struct FaultInjectingTerminalSurfaceEventSink {
    target: Arc<dyn TerminalSurfaceEventSink>,
    state: Arc<parking_lot::Mutex<TerminalSurfaceEventFaultState>>,
}

impl TerminalSurfaceEventSink for FaultInjectingTerminalSurfaceEventSink {
    fn publish(&self, event: TerminalSurfaceEvent) {
        let events = {
            let mut state = self.state.lock();
            match state.armed {
                Some(TerminalSurfaceEventFault::DropNext) => {
                    state.armed = None;
                    Vec::new()
                }
                Some(TerminalSurfaceEventFault::DuplicateNext) => {
                    state.armed = None;
                    vec![event.clone(), event]
                }
                Some(TerminalSurfaceEventFault::ReverseNextTwo) => {
                    if let Some(held) = state.held.take() {
                        state.armed = None;
                        vec![event, held]
                    } else {
                        state.held = Some(event);
                        Vec::new()
                    }
                }
                None => vec![event],
            }
        };
        for event in events {
            self.target.publish(event);
        }
    }
}

pub(crate) fn fault_injecting_event_sink(
    target: Arc<dyn TerminalSurfaceEventSink>,
) -> (
    Arc<dyn TerminalSurfaceEventSink>,
    TerminalSurfaceEventFaultController,
) {
    let state = Arc::new(parking_lot::Mutex::new(
        TerminalSurfaceEventFaultState::default(),
    ));
    (
        Arc::new(FaultInjectingTerminalSurfaceEventSink {
            target,
            state: Arc::clone(&state),
        }),
        TerminalSurfaceEventFaultController { state },
    )
}

#[cfg(test)]
#[path = "event_fault_relay_test.rs"]
mod event_fault_relay_tests;
