#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceSequenceDecision {
    Deliver,
    Ignore,
    Resynchronize,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceAttachment {
    attachment_id: String,
    last_sequence: u64,
    closed: bool,
    process_exited: bool,
}

impl TerminalSurfaceAttachment {
    pub fn new(attachment_id: String, snapshot_sequence: u64) -> Self {
        Self {
            attachment_id,
            last_sequence: snapshot_sequence,
            closed: false,
            process_exited: false,
        }
    }

    pub fn attachment_id(&self) -> &str {
        &self.attachment_id
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn observe(
        &mut self,
        sequence: u64,
        closes_surface: bool,
    ) -> TerminalSurfaceSequenceDecision {
        if self.closed {
            return TerminalSurfaceSequenceDecision::Closed;
        }
        if sequence <= self.last_sequence {
            return TerminalSurfaceSequenceDecision::Ignore;
        }
        if self.last_sequence.checked_add(1) != Some(sequence) {
            return TerminalSurfaceSequenceDecision::Resynchronize;
        }
        self.last_sequence = sequence;
        self.closed = closes_surface;
        self.process_exited = closes_surface;
        TerminalSurfaceSequenceDecision::Deliver
    }

    pub fn apply_snapshot(
        &mut self,
        snapshot_sequence: u64,
        minimum_covered_sequence: Option<u64>,
        process_exited: bool,
    ) -> bool {
        if self.closed || snapshot_sequence < self.last_sequence {
            return false;
        }
        if minimum_covered_sequence.is_some_and(|minimum| snapshot_sequence < minimum) {
            self.closed = true;
            return false;
        }
        self.last_sequence = snapshot_sequence;
        self.closed = process_exited;
        self.process_exited = process_exited;
        true
    }

    pub fn should_resynchronize(&self) -> bool {
        !self.process_exited
    }

    pub fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
#[path = "terminal_surface_attachment_test.rs"]
mod terminal_surface_attachment_tests;
