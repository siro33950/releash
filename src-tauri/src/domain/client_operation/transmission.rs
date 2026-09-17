use super::{handoff::restored_unknown, policy, registry::OperationIdentity};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(crate) enum WriteProgress {
    NotStarted,
    Attempted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransmissionFailure {
    NotSent,
    Unknown,
}
impl WriteProgress {
    pub fn failure(self) -> TransmissionFailure {
        match self {
            Self::NotStarted => TransmissionFailure::NotSent,
            Self::Attempted => TransmissionFailure::Unknown,
        }
    }
}

#[derive(Default)]
pub(crate) struct ClientTransmission {
    pending: HashMap<String, OperationIdentity>,
    restored_queries: HashSet<String>,
    restored_state: HashSet<String>,
    restoration_complete: bool,
}

impl ClientTransmission {
    pub fn admit(
        &self,
        id: &str,
        identity: &OperationIdentity,
        deadline: u64,
        now: u64,
    ) -> Result<(), &'static str> {
        if !self.restoration_complete
            && !matches!(
                policy::recovery(&identity.command),
                policy::Recovery::Read | policy::Recovery::Connection
            )
            && identity.command != "request_application_quit"
        {
            return Err("Desktop state restoration is incomplete");
        }
        if let Some(previous) = self.pending.get(id) {
            if previous != identity {
                return Err("Operation identity changed before transmission");
            }
        } else if self.pending.len() >= policy::MAX_UNACKNOWLEDGED_OPERATIONS {
            return Err("Desktop operation limit reached");
        }
        if deadline != 0 && deadline <= now {
            return Err("Request expired before transmission");
        }
        Ok(())
    }
    pub fn sent(&mut self, id: String, identity: OperationIdentity) {
        self.pending.insert(id, identity);
    }
    pub fn queried(
        &mut self,
        id: &str,
        identity: OperationIdentity,
        sent: bool,
        previous: &str,
        current: &str,
    ) -> Result<bool, &'static str> {
        if !self.restoration_complete && sent && policy::persists_for_restart(&identity.command) {
            return Ok(false);
        }
        if sent && previous == current {
            self.admit(id, &identity, 0, 0)?;
            self.sent(id.into(), identity);
        } else if sent && restored_unknown(&identity.command, previous, current) {
            if !self.restored_queries.contains(id)
                && self.restored_queries.len() >= policy::MAX_UNACKNOWLEDGED_OPERATIONS
            {
                return Err("Desktop recovery operation limit reached");
            }
            self.restored_queries.insert(id.into());
        }
        Ok(true)
    }
    pub fn unknown_after_restart(&mut self, id: &str) -> bool {
        self.restored_queries.remove(id)
    }
    pub fn needs_handoff(&self, id: &str) -> bool {
        self.pending
            .get(id)
            .is_some_and(|operation| policy::persists_for_restart(&operation.command))
    }
    pub fn responded(&mut self, id: &str, succeeded: bool) {
        if succeeded {
            if let Some(operation) = self.pending.get(id) {
                if ["get_repo_paths", "get_performance_telemetry_enabled"]
                    .contains(&operation.command.as_str())
                {
                    self.restored_state.insert(operation.command.clone());
                }
            }
        }
        self.restored_queries.remove(id);
        if !self.needs_handoff(id) {
            self.pending.remove(id);
        }
    }
    pub fn finish_restoration(&mut self) -> Result<(), &'static str> {
        if self.restored_state.len() != 2 {
            return Err("Required desktop state has not been restored");
        }
        self.restoration_complete = true;
        Ok(())
    }
    pub fn acknowledged(&mut self, id: &str) {
        self.pending.remove(id);
        self.restored_queries.remove(id);
    }
}

#[cfg(test)]
#[path = "transmission_test.rs"]
mod transmission_tests;
