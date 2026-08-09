use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurfaceInput {
    pub sequence: u64,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalSurfaceInputIngressError {
    StaleAttachment,
    PendingCapacityExceeded,
}

struct TerminalSurfaceInputIngress {
    attachment_id: String,
    next_sequence: u64,
    pending: BTreeMap<u64, TerminalSurfaceInput>,
    failure_active: bool,
}

pub struct TerminalSurfaceInputIngressRegistry {
    sessions: HashMap<String, TerminalSurfaceInputIngress>,
    pending_capacity: usize,
}

impl Default for TerminalSurfaceInputIngressRegistry {
    fn default() -> Self {
        Self::with_pending_capacity(1024)
    }
}

impl TerminalSurfaceInputIngressRegistry {
    pub fn with_pending_capacity(pending_capacity: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            pending_capacity,
        }
    }

    pub fn activate(&mut self, session_key: &str, attachment_id: &str) {
        self.sessions.insert(
            session_key.to_string(),
            TerminalSurfaceInputIngress {
                attachment_id: attachment_id.to_string(),
                next_sequence: 0,
                pending: BTreeMap::new(),
                failure_active: false,
            },
        );
    }

    pub fn deactivate(&mut self, session_key: &str, attachment_id: &str) {
        if self
            .sessions
            .get(session_key)
            .map(|ingress| ingress.attachment_id.as_str())
            == Some(attachment_id)
        {
            self.sessions.remove(session_key);
        }
    }

    fn active(
        &mut self,
        session_key: &str,
        attachment_id: &str,
    ) -> Result<&mut TerminalSurfaceInputIngress, TerminalSurfaceInputIngressError> {
        let ingress = self
            .sessions
            .get_mut(session_key)
            .ok_or(TerminalSurfaceInputIngressError::StaleAttachment)?;
        if ingress.attachment_id != attachment_id {
            return Err(TerminalSurfaceInputIngressError::StaleAttachment);
        }
        Ok(ingress)
    }

    fn drain_ready(ingress: &mut TerminalSurfaceInputIngress) -> Vec<TerminalSurfaceInput> {
        let mut ready = Vec::new();
        while let Some(input) = ingress.pending.remove(&ingress.next_sequence) {
            ready.push(input);
            ingress.next_sequence = ingress.next_sequence.saturating_add(1);
        }
        ready
    }

    pub fn admit(
        &mut self,
        session_key: &str,
        attachment_id: &str,
        sequence: u64,
        data: String,
    ) -> Result<Vec<TerminalSurfaceInput>, TerminalSurfaceInputIngressError> {
        let pending_capacity = self.pending_capacity;
        let ingress = self.active(session_key, attachment_id)?;
        let mut ready = Self::drain_ready(ingress);
        if sequence < ingress.next_sequence || ingress.pending.contains_key(&sequence) {
            return Ok(ready);
        }
        if sequence > ingress.next_sequence && ingress.pending.len() >= pending_capacity {
            return Err(TerminalSurfaceInputIngressError::PendingCapacityExceeded);
        }
        ingress
            .pending
            .insert(sequence, TerminalSurfaceInput { sequence, data });
        ready.extend(Self::drain_ready(ingress));
        Ok(ready)
    }

    pub fn restore_failed(
        &mut self,
        session_key: &str,
        attachment_id: &str,
        inputs: Vec<TerminalSurfaceInput>,
    ) -> Result<(), TerminalSurfaceInputIngressError> {
        let ingress = self.active(session_key, attachment_id)?;
        if let Some(first_sequence) = inputs.first().map(|input| input.sequence) {
            ingress.next_sequence = ingress.next_sequence.min(first_sequence);
        }
        for input in inputs {
            ingress.pending.entry(input.sequence).or_insert(input);
        }
        Ok(())
    }

    pub fn record_failure(&mut self, session_key: &str, attachment_id: &str) -> bool {
        let Ok(ingress) = self.active(session_key, attachment_id) else {
            return false;
        };
        if ingress.failure_active {
            return false;
        }
        ingress.failure_active = true;
        true
    }

    pub fn record_success(&mut self, session_key: &str, attachment_id: &str) {
        if let Ok(ingress) = self.active(session_key, attachment_id) {
            ingress.failure_active = false;
        }
    }
}

#[cfg(test)]
#[path = "terminal_surface_input_ingress_test.rs"]
mod terminal_surface_input_ingress_tests;
