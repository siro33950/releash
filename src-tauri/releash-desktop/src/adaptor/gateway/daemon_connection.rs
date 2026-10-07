use super::desktop_client::{self, DesktopClient};
use crate::common::retry::RetryLimiter;
use crate::domain::daemon_connection::{
    DaemonConnectionState, DaemonEndpoint, DaemonResult, DaemonService, DiscoveredDaemon,
};
use crate::usecase::daemon_connection_query::{
    DaemonConnectionQueryService, DesktopSettingsSubscription,
};
use releash_sdk::daemon;
use releashd::desktop_api::{ClientConnectionDto, DesktopSettingsDto};
use std::{path::PathBuf, sync::Arc};

pub struct DaemonServiceGateway {
    executable: PathBuf,
    data_dir: PathBuf,
    limiter: Arc<RetryLimiter>,
    settings_sources: tokio::sync::watch::Sender<
        Option<tokio::sync::watch::Receiver<Option<DesktopSettingsDto>>>,
    >,
    client: parking_lot::RwLock<Option<Arc<DesktopClient>>>,
}
impl DaemonServiceGateway {
    pub fn new(
        executable: PathBuf,
        data_dir: PathBuf,
        limiter: Arc<RetryLimiter>,
        settings_sources: tokio::sync::watch::Sender<
            Option<tokio::sync::watch::Receiver<Option<DesktopSettingsDto>>>,
        >,
    ) -> Self {
        Self {
            executable,
            data_dir,
            limiter,
            settings_sources,
            client: parking_lot::RwLock::new(None),
        }
    }
    pub fn client(&self) -> Result<Arc<DesktopClient>, String> {
        self.client
            .read()
            .clone()
            .ok_or_else(|| "サーバに接続していません".into())
    }
}
fn daemon_failure(error: daemon::DaemonError) -> DaemonConnectionState {
    match error {
        daemon::DaemonError::Exited { status, stderr } => DaemonConnectionState::StartupFailed {
            status: Some(status.to_string()),
            stderr,
        },
        daemon::DaemonError::StartupTimeout { stderr } => DaemonConnectionState::StartupFailed {
            status: None,
            stderr,
        },
        error => DaemonConnectionState::TechnicalFailure(error.to_string()),
    }
}
impl DaemonService for DaemonServiceGateway {
    fn discover(&self) -> DaemonResult<'_, Option<DiscoveredDaemon>> {
        Box::pin(async move {
            let Some(discovery) = daemon::running(&self.data_dir).map_err(daemon_failure)? else {
                return Ok(None);
            };
            let info = daemon::server_info(&discovery)
                .await
                .map_err(daemon_failure)?;
            Ok(Some(DiscoveredDaemon {
                endpoint: DaemonEndpoint {
                    url: format!("http://127.0.0.1:{}", discovery.port),
                    token: discovery.token,
                },
                protocol: info.protocol,
                release: info.release,
            }))
        })
    }
    fn start(&self) -> DaemonResult<'_, ()> {
        Box::pin(async move {
            let cwd = std::env::current_dir()
                .map_err(|e| DaemonConnectionState::TechnicalFailure(e.to_string()))?;
            daemon::start(&self.executable, &self.data_dir, &cwd)
                .await
                .map_err(daemon_failure)?;
            Ok(())
        })
    }
    fn stop(&self) -> DaemonResult<'_, ()> {
        Box::pin(async move {
            if let Some(discovery) = daemon::running(&self.data_dir).map_err(daemon_failure)? {
                daemon::stop(&self.data_dir, &discovery)
                    .await
                    .map_err(daemon_failure)?;
            }
            *self.client.write() = None;
            self.settings_sources.send_replace(None);
            Ok(())
        })
    }
}
impl DesktopSettingsSubscription for DaemonServiceGateway {
    fn connect<'a>(&'a self, endpoint: &'a DaemonEndpoint) -> DaemonResult<'a, ()> {
        Box::pin(async move {
            *self.client.write() = None;
            let connection = ClientConnectionDto {
                url: endpoint.url.clone(),
                token: endpoint.token.clone(),
            };
            let client = Arc::new(DesktopClient::start(
                desktop_client::client(&connection)
                    .map_err(DaemonConnectionState::TechnicalFailure)?,
                desktop_client::stream_client(&connection)
                    .map_err(DaemonConnectionState::TechnicalFailure)?,
                self.limiter.clone(),
            ));
            tokio::time::timeout(
                daemon::timeout("min_connect_timeout_ms"),
                client.first_settings(),
            )
            .await
            .map_err(|_| DaemonConnectionState::InitialSettingsUnavailable { detail: None })?
            .map_err(|e| DaemonConnectionState::InitialSettingsUnavailable {
                detail: Some(e.message),
            })?;
            self.settings_sources
                .send_replace(Some(client.settings_receiver()));
            *self.client.write() = Some(client);
            Ok(())
        })
    }
}
impl DaemonConnectionQueryService for DaemonServiceGateway {
    fn settings(&self) -> Option<DesktopSettingsDto> {
        self.client().ok()?.current_settings()
    }
}

#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
