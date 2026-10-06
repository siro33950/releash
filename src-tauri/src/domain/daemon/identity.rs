#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaemonIdentity {
    pub(crate) daemon_id: String,
    pub(crate) pid: u32,
    pub(crate) process_started_at: u64,
}

impl DaemonIdentity {
    pub(crate) fn is_valid(&self) -> bool {
        !self.daemon_id.trim().is_empty() && self.pid != 0 && self.process_started_at != 0
    }
    pub(crate) fn assess_process(
        &self,
        endpoint_valid: bool,
        observation: ProcessObservation,
    ) -> Result<(), DiscoveryRejection> {
        if !endpoint_valid || !self.is_valid() {
            return Err(DiscoveryRejection::InvalidOrStale);
        }
        match observation {
            ProcessObservation::Unavailable => {
                Err(DiscoveryRejection::ProcessInformationUnavailable)
            }
            ProcessObservation::ProcessNotFound => Err(DiscoveryRejection::InvalidOrStale),
            ProcessObservation::StartedAt(time) if time != self.process_started_at => {
                Err(DiscoveryRejection::InvalidOrStale)
            }
            ProcessObservation::StartedAt(_) => Ok(()),
        }
    }

    pub(crate) fn matches_client(&self, client: &Self, endpoints_match: bool) -> bool {
        self.is_valid() && client.is_valid() && self == client && endpoints_match
    }
    pub(crate) fn assess_connection(
        &self,
        observation: ConnectionObservation,
    ) -> Result<(), DiscoveryRejection> {
        match observation {
            ConnectionObservation::IdentityVerified => Ok(()),
            ConnectionObservation::UnexpectedResponse => Err(DiscoveryRejection::InstanceMismatch),
            ConnectionObservation::NoResponse => Err(DiscoveryRejection::ConnectionUnreachable),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessObservation {
    Unavailable,
    ProcessNotFound,
    StartedAt(u64),
}

impl ProcessObservation {
    pub(crate) fn from_raw(process_list_available: bool, start_time: Option<u64>) -> Self {
        if !process_list_available {
            return Self::Unavailable;
        }
        match start_time {
            Some(start_time) if start_time != 0 => Self::StartedAt(start_time),
            Some(_) | None => Self::ProcessNotFound,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionObservation {
    IdentityVerified,
    UnexpectedResponse,
    NoResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveryRejection {
    InvalidOrStale,
    ProcessInformationUnavailable,
    InstanceMismatch,
    ConnectionUnreachable,
}
