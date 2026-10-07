use super::daemon_connection_query::DaemonConnectionQueryService;
use crate::domain::daemon_connection::{
    DaemonConnection, DaemonConnectionState, DaemonEndpoint, DaemonService,
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
    pub fn failure(&self) -> Option<DaemonConnectionState> {
        let state = self.state.lock();
        state.is_failure().then(|| state.state().clone())
    }
    pub fn settings(&self) -> Option<DesktopSettingsDto> {
        if self.state.lock().is_connected() {
            self.query.settings()
        } else {
            None
        }
    }
    pub async fn connect(&self) -> Result<(), DaemonConnectionState> {
        let _guard = self.connecting.lock().await;
        self.establish(true).await.map(|_| ())
    }
    pub async fn endpoint(&self) -> Result<(DaemonEndpoint, bool), DaemonConnectionState> {
        let _guard = self.connecting.lock().await;
        self.establish(false).await
    }
    async fn establish(
        &self,
        start_if_missing: bool,
    ) -> Result<(DaemonEndpoint, bool), DaemonConnectionState> {
        let result = async {
            let mut server = self.port.discover().await?;
            if server.is_none() && start_if_missing {
                self.state.lock().begin_start();
                self.port.start().await?;
                server = self.port.discover().await?;
            }
            let Some(server) = server else {
                let mut state = self.state.lock();
                state.observe_not_running();
                return Err(state.state().clone());
            };
            {
                let mut state = self.state.lock();
                if !state.assess(&server, self.protocol, &self.release) {
                    return Err(state.state().clone());
                }
            }
            let changed = !self.state.lock().is_connected_to(&server.endpoint);
            if changed {
                self.port.connect(&server.endpoint).await?;
            }
            self.state.lock().connected(server.endpoint.clone());
            Ok((server.endpoint, changed))
        }
        .await;
        if let Err(failure) = &result {
            self.state.lock().failed(failure.clone());
        }
        result
    }
    pub async fn stop(&self) -> Result<(), DaemonConnectionState> {
        let _guard = self.connecting.lock().await;
        self.port.stop().await?;
        self.state.lock().observe_not_running();
        Ok(())
    }
    pub async fn replace(&self) -> Result<(), DaemonConnectionState> {
        let _guard = self.connecting.lock().await;
        self.port.stop().await?;
        self.establish(true).await.map(|_| ())
    }
}
#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
