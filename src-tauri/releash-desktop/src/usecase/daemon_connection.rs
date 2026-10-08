use super::daemon_connection_query::DaemonConnectionQueryService;
use crate::domain::daemon_connection::{
    DaemonConnection, DaemonConnectionFailure, DaemonEndpoint, DaemonService, DaemonSubscription,
};
use releashd::desktop_api::DesktopSettingsDto;
use std::sync::Arc;

pub struct DaemonConnectionUsecase {
    port: Arc<dyn DaemonService>,
    query: Arc<dyn DaemonConnectionQueryService>,
    protocol: u32,
    release: String,
    state: parking_lot::Mutex<DaemonConnection>,
    connecting: tokio::sync::Mutex<()>,
}
impl DaemonConnectionUsecase {
    pub fn new(
        port: Arc<dyn DaemonService>,
        query: Arc<dyn DaemonConnectionQueryService>,
        protocol: u32,
        release: String,
    ) -> Self {
        Self {
            port,
            query,
            protocol,
            release,
            state: parking_lot::Mutex::new(DaemonConnection::default()),
            connecting: tokio::sync::Mutex::new(()),
        }
    }
    pub fn failure(&self) -> Option<DaemonConnectionFailure> {
        let state = self.state.lock();
        state.failure().cloned()
    }
    pub fn is_current_subscription(&self, subscription: DaemonSubscription) -> bool {
        self.state.lock().is_current_subscription(subscription)
    }
    pub fn initial_settings(&self) -> Option<DesktopSettingsDto> {
        self.query.initial_settings()
    }
    pub fn settings(&self) -> Option<DesktopSettingsDto> {
        if self.state.lock().is_connected() {
            self.query.settings()
        } else {
            None
        }
    }
    pub async fn connect(&self) -> Result<(DaemonEndpoint, bool), DaemonConnectionFailure> {
        let _guard = self.connecting.lock().await;
        self.establish(true).await
    }
    pub async fn endpoint(&self) -> Result<(DaemonEndpoint, bool), DaemonConnectionFailure> {
        let _guard = self.connecting.lock().await;
        self.establish(false).await
    }
    async fn establish(
        &self,
        start_if_missing: bool,
    ) -> Result<(DaemonEndpoint, bool), DaemonConnectionFailure> {
        let result = async {
            let mut server = self.port.discover().await?;
            if server.is_none() && start_if_missing {
                self.state.lock().begin_start();
                self.port.start().await?;
                server = self.port.discover().await?;
            }
            let Some(server) = server else {
                let mut state = self.state.lock();
                return Err(state.observe_not_running());
            };
            {
                let mut state = self.state.lock();
                if let Some(failure) = state.assess(&server, self.protocol, &self.release) {
                    return Err(failure);
                }
            }
            let changed = !self.state.lock().is_connected_to(&server.endpoint);
            if changed {
                let subscription = self.port.connect(&server.endpoint).await?;
                self.state
                    .lock()
                    .connected(server.endpoint.clone(), subscription);
            }
            Ok((server.endpoint, changed))
        }
        .await;
        if let Err(failure) = &result {
            self.state.lock().failed(failure.clone());
        }
        result
    }
    pub async fn stop(&self) -> Result<(), DaemonConnectionFailure> {
        let _guard = self.connecting.lock().await;
        self.port.stop().await?;
        self.state.lock().observe_not_running();
        Ok(())
    }
    pub async fn replace(&self) -> Result<(DaemonEndpoint, bool), DaemonConnectionFailure> {
        let _guard = self.connecting.lock().await;
        self.port.stop().await?;
        self.state.lock().observe_not_running();
        self.establish(true).await
    }
}
#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
