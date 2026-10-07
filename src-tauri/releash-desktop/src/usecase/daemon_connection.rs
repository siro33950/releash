use super::daemon_connection_query::{ConnectionFailure, DaemonConnectionQueryService};
use crate::domain::daemon_connection::{ConnectionError, DaemonConnectionPort, DaemonEndpoint};
use releashd::desktop_api::DesktopSettingsDto;
use std::sync::Arc;

pub struct DaemonConnectionUsecase {
    port: Arc<dyn DaemonConnectionPort>,
    query: Arc<dyn DaemonConnectionQueryService>,
    connecting: tokio::sync::Mutex<()>,
}

impl DaemonConnectionUsecase {
    pub fn new(
        port: Arc<dyn DaemonConnectionPort>,
        query: Arc<dyn DaemonConnectionQueryService>,
    ) -> Self {
        Self {
            port,
            query,
            connecting: tokio::sync::Mutex::new(()),
        }
    }
    pub fn failure(&self) -> Option<ConnectionFailure> {
        self.query.failure()
    }
    pub fn settings(&self) -> Option<DesktopSettingsDto> {
        self.query.settings()
    }
    pub fn settings_update(&self) -> Option<DesktopSettingsDto> {
        self.query.settings_update()
    }
    pub async fn connect(&self) -> Result<(), ConnectionError> {
        let _guard = self.connecting.lock().await;
        self.establish(true).await.map(|_| ())
    }
    pub async fn endpoint(&self) -> Result<(DaemonEndpoint, bool), ConnectionError> {
        let _guard = self.connecting.lock().await;
        self.establish(false).await
    }
    async fn establish(
        &self,
        start_if_missing: bool,
    ) -> Result<(DaemonEndpoint, bool), ConnectionError> {
        let result = async {
            let mut endpoint = self.port.discover().await?;
            if endpoint.is_none() && start_if_missing {
                self.port.start().await?;
                endpoint = self.port.discover().await?;
            }
            let endpoint = endpoint
                .ok_or_else(|| ConnectionError::from("サーバは動いていません".to_string()))?;
            let changed = !self.port.subscribed_to(&endpoint);
            if changed {
                self.port.subscribe(&endpoint).await?;
            }
            Ok((endpoint, changed))
        }
        .await;
        self.port.record_failure(result.as_ref().err().cloned());
        result
    }
    pub async fn stop(&self) -> Result<(), ConnectionError> {
        let _guard = self.connecting.lock().await;
        self.port.stop().await
    }
    pub async fn replace(&self) -> Result<(), ConnectionError> {
        let _guard = self.connecting.lock().await;
        self.port.stop().await?;
        self.establish(true).await.map(|_| ())
    }
}

#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
