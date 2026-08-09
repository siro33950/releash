use std::collections::VecDeque;

use parking_lot::{Condvar, Mutex};

pub(crate) const TERMINAL_OUTPUT_CREDIT_CODE_UNITS: usize = 256 * 1024;

pub(crate) struct TerminalOutputFlowControl {
    state: Mutex<TerminalOutputFlowState>,
    changed: Condvar,
    enabled: bool,
}

#[derive(Default)]
struct TerminalOutputFlowState {
    active_attachment_id: Option<String>,
    pending: VecDeque<(u64, usize)>,
    pending_code_units: usize,
}

impl TerminalOutputFlowControl {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            state: Mutex::new(TerminalOutputFlowState::default()),
            changed: Condvar::new(),
            enabled,
        }
    }

    pub(crate) fn activate(&self, attachment_id: &str) {
        let mut state = self.state.lock();
        state.active_attachment_id = Some(attachment_id.to_string());
        state.pending.clear();
        state.pending_code_units = 0;
        self.changed.notify_all();
    }

    pub(crate) fn deactivate(&self, attachment_id: &str) {
        let mut state = self.state.lock();
        if state.active_attachment_id.as_deref() != Some(attachment_id) {
            return;
        }
        state.active_attachment_id = None;
        state.pending.clear();
        state.pending_code_units = 0;
        self.changed.notify_all();
    }

    pub(crate) fn reserve(&self, sequence: u64, code_units: usize) {
        if !self.enabled {
            return;
        }
        let mut state = self.state.lock();
        while state.active_attachment_id.is_some()
            && !state.pending.is_empty()
            && state.pending_code_units.saturating_add(code_units)
                > TERMINAL_OUTPUT_CREDIT_CODE_UNITS
        {
            self.changed.wait(&mut state);
        }
        if state.active_attachment_id.is_none() {
            return;
        }
        state.pending.push_back((sequence, code_units));
        state.pending_code_units = state.pending_code_units.saturating_add(code_units);
    }

    pub(crate) fn acknowledge(&self, attachment_id: &str, sequence: u64) {
        let mut state = self.state.lock();
        if state.active_attachment_id.as_deref() != Some(attachment_id) {
            return;
        }
        while let Some((pending_sequence, code_units)) = state.pending.front().copied() {
            if pending_sequence > sequence {
                break;
            }
            state.pending.pop_front();
            state.pending_code_units = state.pending_code_units.saturating_sub(code_units);
        }
        self.changed.notify_all();
    }
}
