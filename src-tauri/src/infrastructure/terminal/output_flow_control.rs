use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use super::output_pause::OutputPause;

pub(crate) const OUTPUT_HIGH_WATERMARK: usize = 100_000;
pub(crate) const OUTPUT_LOW_WATERMARK: usize = 5_000;
pub(crate) const OUTPUT_REPORT_UNITS: usize = 5_000;
pub(crate) const OUTPUT_PENDING_LIMIT: usize = 2 * OUTPUT_HIGH_WATERMARK;

#[derive(Default)]
pub(crate) struct OutputFlowControl {
    pending: HashMap<String, usize>,
    paused: bool,
    sequence: u64,
}

impl OutputFlowControl {
    pub fn subscribe(&mut self, id: &str, units: usize) -> bool {
        self.pending.insert(id.into(), units);
        self.update()
    }

    pub fn reset(&mut self, sequence: u64) {
        self.sequence = sequence;
        for pending in self.pending.values_mut() {
            *pending = 0;
        }
        self.paused = false;
    }

    pub fn unsubscribe(&mut self, id: &str) -> bool {
        self.pending.remove(id);
        self.update()
    }

    pub fn output(&mut self, sequence: u64, units: usize) -> bool {
        if sequence <= self.sequence {
            return self.paused;
        }
        self.sequence = sequence;
        for pending in self.pending.values_mut() {
            *pending = pending.saturating_add(units);
        }
        self.update()
    }

    pub fn processed(&mut self, id: &str, units: usize) -> bool {
        if let Some(pending) = self.pending.get_mut(id) {
            *pending = pending.saturating_sub(units);
        }
        self.update()
    }

    fn update(&mut self) -> bool {
        let pending = self.pending.values().copied().max().unwrap_or(0);
        if pending > OUTPUT_HIGH_WATERMARK {
            self.paused = true;
        } else if pending < OUTPUT_LOW_WATERMARK {
            self.paused = false;
        }
        self.paused
    }
}

pub(crate) struct TerminalOutputFlow {
    enabled: bool,
    sessions: Mutex<HashMap<String, (OutputFlowControl, Arc<OutputPause>)>>,
}

impl TerminalOutputFlow {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn subscribe(&self, session: &str, client: &str, units: usize) {
        if !self.enabled {
            return;
        }
        let mut sessions = self.sessions.lock();
        let (flow, pause) = sessions.entry(session.into()).or_default();
        pause.set(flow.subscribe(client, units));
    }

    pub(crate) fn unsubscribe(&self, session: &str, client: &str) {
        if let Some((flow, pause)) = self.sessions.lock().get_mut(session) {
            pause.set(flow.unsubscribe(client));
        }
    }

    pub(crate) fn processed(&self, session: &str, client: &str, units: usize) {
        if let Some((flow, pause)) = self.sessions.lock().get_mut(session) {
            pause.set(flow.processed(client, units));
        }
    }

    pub(crate) fn reset(&self, session: &str, sequence: u64) {
        if let Some((flow, pause)) = self.sessions.lock().get_mut(session) {
            flow.reset(sequence);
            pause.set(false);
        }
    }

    pub(crate) fn release(&self, session: &str) {
        if let Some((_, pause)) = self.sessions.lock().remove(session) {
            pause.set(false);
        }
    }

    pub(crate) fn wait(&self, session: &str) {
        let pause = self
            .sessions
            .lock()
            .get(session)
            .map(|(_, pause)| pause.clone());
        if let Some(pause) = pause {
            pause.wait();
        }
    }

    pub(crate) fn output(&self, session: &str, sequence: u64, units: usize) {
        if !self.enabled {
            return;
        }
        if let Some((flow, pause)) = self.sessions.lock().get_mut(session) {
            pause.set(flow.output(sequence, units));
        }
    }

    #[cfg(test)]
    pub(crate) fn test_pause(&self, session: &str) -> Option<Arc<OutputPause>> {
        self.sessions
            .lock()
            .get(session)
            .map(|(_, pause)| pause.clone())
    }

    #[cfg(test)]
    pub(crate) fn test_subscribed(&self, session: &str, client: &str) -> bool {
        self.sessions
            .lock()
            .get(session)
            .is_some_and(|(flow, _)| flow.pending.contains_key(client))
    }
}

#[cfg(test)]
#[path = "output_flow_control_test.rs"]
mod output_flow_control_tests;
