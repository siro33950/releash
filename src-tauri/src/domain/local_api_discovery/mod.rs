#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveryContent {
    port: u16,
    token: String,
    instance_id: String,
    pid: u32,
    process_started_at: u64,
}

impl DiscoveryContent {
    pub(crate) fn new(
        port: u16,
        token: String,
        instance_id: String,
        pid: u32,
        process_started_at: u64,
    ) -> Self {
        Self {
            port,
            token,
            instance_id,
            pid,
            process_started_at,
        }
    }

    pub(crate) fn is_acceptable(&self) -> bool {
        self.port != 0
            && !self.token.trim().is_empty()
            && !self.instance_id.trim().is_empty()
            && self.pid != 0
            && self.process_started_at != 0
    }

    pub(crate) fn process_started_at(&self) -> u64 {
        self.process_started_at
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

impl ConnectionObservation {
    pub(crate) fn from_response_status(status: Option<u16>) -> Self {
        match status {
            Some(204) => Self::IdentityVerified,
            Some(_) => Self::UnexpectedResponse,
            None => Self::NoResponse,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveryRejection {
    InvalidOrStale,
    ProcessInformationUnavailable,
    InstanceMismatch,
    ConnectionUnreachable,
}

pub(crate) struct DiscoveryAdmissionService;

impl DiscoveryAdmissionService {
    pub(crate) fn assess_process(
        content: &DiscoveryContent,
        observation: ProcessObservation,
    ) -> Result<(), DiscoveryRejection> {
        if !content.is_acceptable() {
            return Err(DiscoveryRejection::InvalidOrStale);
        }
        match observation {
            ProcessObservation::Unavailable => {
                Err(DiscoveryRejection::ProcessInformationUnavailable)
            }
            ProcessObservation::ProcessNotFound => Err(DiscoveryRejection::InvalidOrStale),
            ProcessObservation::StartedAt(start_time)
                if start_time != content.process_started_at() =>
            {
                Err(DiscoveryRejection::InvalidOrStale)
            }
            ProcessObservation::StartedAt(_) => Ok(()),
        }
    }

    pub(crate) fn assess_connection(
        observation: ConnectionObservation,
    ) -> Result<(), DiscoveryRejection> {
        match observation {
            ConnectionObservation::IdentityVerified => Ok(()),
            ConnectionObservation::UnexpectedResponse => Err(DiscoveryRejection::InstanceMismatch),
            ConnectionObservation::NoResponse => Err(DiscoveryRejection::ConnectionUnreachable),
        }
    }
}

#[cfg(test)]
#[path = "local_api_discovery_test.rs"]
mod local_api_discovery_tests;
