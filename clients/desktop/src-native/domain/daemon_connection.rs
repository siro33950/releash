use releash::compatibility::Compatibility;
use std::{future::Future, pin::Pin};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub url: String,
    pub token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DaemonSubscription(pub u128);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredDaemon {
    pub endpoint: DaemonEndpoint,
    pub protocol: u32,
    pub release: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonConnectionState {
    NotObserved,
    Connected(DaemonEndpoint, DaemonSubscription),
    Failed(DaemonConnectionFailure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonConnectionFailure {
    NotRunning,
    Incompatible {
        server_older: bool,
        server_release: String,
        client_release: String,
    },
    StartupFailed {
        status: Option<String>,
        stderr: String,
    },
    InitialSettingsUnavailable {
        detail: Option<String>,
    },
    TechnicalFailure(String),
}

pub struct DaemonConnection {
    state: DaemonConnectionState,
}
impl Default for DaemonConnection {
    fn default() -> Self {
        Self {
            state: DaemonConnectionState::NotObserved,
        }
    }
}
impl DaemonConnection {
    pub fn state(&self) -> &DaemonConnectionState {
        &self.state
    }
    pub fn assess(
        &mut self,
        server: &DiscoveredDaemon,
        protocol: u32,
        release: &str,
    ) -> Option<DaemonConnectionFailure> {
        match Compatibility::assess(protocol, server.protocol) {
            Compatibility::Compatible => None,
            compatibility => {
                let failure = DaemonConnectionFailure::Incompatible {
                    server_older: compatibility == Compatibility::ServerOlder,
                    server_release: server.release.clone(),
                    client_release: release.into(),
                };
                self.failed(failure.clone());
                Some(failure)
            }
        }
    }
    pub fn observe_not_running(&mut self) -> DaemonConnectionFailure {
        let failure = match &self.state {
            DaemonConnectionState::Failed(
                failure @ DaemonConnectionFailure::StartupFailed { .. },
            ) => failure.clone(),
            _ => DaemonConnectionFailure::NotRunning,
        };
        self.failed(failure.clone());
        failure
    }

    pub fn begin_start(&mut self) {
        self.state = DaemonConnectionState::NotObserved;
    }
    pub fn failed(&mut self, failure: DaemonConnectionFailure) {
        self.state = DaemonConnectionState::Failed(failure);
    }
    pub fn connected(&mut self, endpoint: DaemonEndpoint, subscription: DaemonSubscription) {
        self.state = DaemonConnectionState::Connected(endpoint, subscription);
    }
    pub fn is_connected_to(&self, endpoint: &DaemonEndpoint) -> bool {
        matches!(&self.state, DaemonConnectionState::Connected(current, _) if current == endpoint)
    }
    pub fn is_current_subscription(&self, subscription: DaemonSubscription) -> bool {
        matches!(self.state, DaemonConnectionState::Connected(_, current) if current == subscription)
    }
    pub fn failure(&self) -> Option<&DaemonConnectionFailure> {
        match self.state() {
            DaemonConnectionState::Failed(failure) => Some(failure),
            _ => None,
        }
    }
    pub fn is_connected(&self) -> bool {
        matches!(self.state, DaemonConnectionState::Connected(_, _))
    }
}

pub type DaemonResult<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DaemonConnectionFailure>> + Send + 'a>>;
pub trait DaemonService: Send + Sync {
    fn discover(&self) -> DaemonResult<'_, Option<DiscoveredDaemon>>;
    fn connect<'a>(&'a self, endpoint: &'a DaemonEndpoint) -> DaemonResult<'a, DaemonSubscription>;
    fn start(&self) -> DaemonResult<'_, ()>;
    fn stop(&self) -> DaemonResult<'_, ()>;
}

#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
