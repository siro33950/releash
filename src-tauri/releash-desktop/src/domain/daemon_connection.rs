use releash_sdk::compatibility::Compatibility;
use std::{future::Future, pin::Pin};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonEndpoint {
    pub url: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredDaemon {
    pub endpoint: DaemonEndpoint,
    pub protocol: u32,
    pub release: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DaemonConnectionState {
    NotObserved,
    NotRunning,
    Connected(DaemonEndpoint),
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
    pub fn assess(&mut self, server: &DiscoveredDaemon, protocol: u32, release: &str) -> bool {
        match Compatibility::assess(protocol, server.protocol) {
            Compatibility::Compatible => true,
            compatibility => {
                self.state = DaemonConnectionState::Incompatible {
                    server_older: compatibility == Compatibility::ServerOlder,
                    server_release: server.release.clone(),
                    client_release: release.into(),
                };
                false
            }
        }
    }
    pub fn observe_not_running(&mut self) {
        if !matches!(self.state, DaemonConnectionState::StartupFailed { .. }) {
            self.state = DaemonConnectionState::NotRunning;
        }
    }
    pub fn begin_start(&mut self) {
        self.state = DaemonConnectionState::NotObserved;
    }
    pub fn failed(&mut self, failure: DaemonConnectionState) {
        self.state = failure;
    }
    pub fn connected(&mut self, endpoint: DaemonEndpoint) {
        self.state = DaemonConnectionState::Connected(endpoint);
    }
    pub fn is_connected_to(&self, endpoint: &DaemonEndpoint) -> bool {
        matches!(&self.state, DaemonConnectionState::Connected(current) if current == endpoint)
    }
    pub fn show_after_connection(
        hidden: bool,
        start_minimized: bool,
        failure_window: bool,
    ) -> bool {
        !hidden || !start_minimized || failure_window
    }
}

pub type DaemonResult<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DaemonConnectionState>> + Send + 'a>>;
pub trait DaemonService: Send + Sync {
    fn discover(&self) -> DaemonResult<'_, Option<DiscoveredDaemon>>;
    fn start(&self) -> DaemonResult<'_, ()>;
    fn stop(&self) -> DaemonResult<'_, ()>;
}

#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
