use std::collections::HashMap;

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

#[cfg(test)]
#[path = "output_flow_control_test.rs"]
mod output_flow_control_tests;
