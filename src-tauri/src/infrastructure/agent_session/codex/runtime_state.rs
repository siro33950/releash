use std::collections::HashMap;

use serde_json::{json, Value};

use crate::infrastructure::agent_session::codex::wire::{
    request, PendingClientRequests, METHOD_TURN_INTERRUPT,
};
use crate::infrastructure::agent_session::stdout_line_reader::StdoutDiagnostics;

#[derive(Debug)]
pub(crate) struct CodexRuntimeState {
    pub(crate) thread_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) active_turn_start_request_id: Option<u64>,
    pub(crate) interrupt_requested_for: Option<u64>,
    pub(crate) turn_start_handshake_active: bool,
    pub(crate) interrupt_requested_during_start_handshake: bool,
    pub(crate) startup_error: Option<String>,
    pub(crate) requested_resume_id: Option<String>,
    pub(crate) resume_rejected: bool,
    pub(crate) cwd: String,
    pub(crate) model: String,
    pub(crate) permission_profile_id: Option<String>,
    pub(crate) pending_methods: HashMap<u64, String>,
    pub(crate) pending_client_requests: PendingClientRequests,
    pub(crate) stdout_diagnostics: StdoutDiagnostics,
}

impl CodexRuntimeState {
    pub(crate) fn begin_turn_start_handshake(&mut self) {
        self.turn_start_handshake_active = true;
        self.interrupt_requested_during_start_handshake = false;
    }

    fn reserve_interrupt_for_start_handshake(&mut self) {
        if self.turn_start_handshake_active {
            self.interrupt_requested_during_start_handshake = true;
        }
    }

    pub(crate) fn register_turn_start_request(&mut self, request_id: u64) {
        self.active_turn_start_request_id = Some(request_id);
        self.interrupt_requested_for = self
            .interrupt_requested_during_start_handshake
            .then_some(request_id);
        self.turn_start_handshake_active = false;
        self.interrupt_requested_during_start_handshake = false;
    }

    pub(crate) fn clear_turn_start_handshake(&mut self) {
        self.turn_start_handshake_active = false;
        self.interrupt_requested_during_start_handshake = false;
    }

    pub(crate) fn clear_failed_turn_start_request(&mut self, request_id: u64) {
        self.pending_client_requests.remove(request_id);
        if self.active_turn_start_request_id == Some(request_id) {
            self.active_turn_start_request_id = None;
            self.interrupt_requested_for = None;
            self.clear_turn_start_handshake();
        }
    }
}

pub(crate) fn prepare_interrupt_request(
    state: &mut CodexRuntimeState,
    request_id: u64,
) -> Option<Value> {
    let Some(thread_id) = state.thread_id.clone() else {
        state.reserve_interrupt_for_start_handshake();
        return None;
    };
    let Some(turn_id) = state.turn_id.clone() else {
        if let Some(active_request_id) = state.active_turn_start_request_id {
            state.interrupt_requested_for = Some(active_request_id);
        } else {
            state.reserve_interrupt_for_start_handshake();
        }
        return None;
    };
    state.interrupt_requested_for = None;
    Some(turn_interrupt_request(request_id, thread_id, turn_id))
}

pub(crate) fn take_reserved_interrupt_request(
    state: &mut CodexRuntimeState,
    request_id: u64,
) -> Option<Value> {
    let requested_for = state.interrupt_requested_for?;
    if state.active_turn_start_request_id != Some(requested_for) {
        state.interrupt_requested_for = None;
        return None;
    }
    prepare_interrupt_request(state, request_id)
}

fn turn_interrupt_request(request_id: u64, thread_id: String, turn_id: String) -> Value {
    request(
        request_id,
        METHOD_TURN_INTERRUPT,
        json!({
            "threadId": thread_id,
            "turnId": turn_id,
        }),
    )
}

pub(crate) fn reset_completed_turn_state(state: &mut CodexRuntimeState) {
    state.turn_id = None;
    state.active_turn_start_request_id = None;
    state.interrupt_requested_for = None;
    state.turn_start_handshake_active = false;
    state.interrupt_requested_during_start_handshake = false;
    state.pending_methods.clear();
}
