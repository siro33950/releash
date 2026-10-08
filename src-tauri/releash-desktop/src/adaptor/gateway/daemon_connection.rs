use super::desktop_client::{self, DesktopClient};
use crate::common::retry::RetryLimiter;
use crate::domain::daemon_connection::{
    DaemonConnectionFailure, DaemonEndpoint, DaemonResult, DaemonService, DaemonSubscription,
    DiscoveredDaemon,
};
use crate::usecase::daemon_connection_query::DaemonConnectionQueryService;
use releash_sdk::daemon;
use releashd::desktop_api::{ClientConnectionDto, DesktopSettingsDto};
use std::{path::PathBuf, sync::Arc};

pub struct DaemonServiceGateway {
    executable: PathBuf,
    data_dir: PathBuf,
    limiter: Arc<RetryLimiter>,
    deadline: crate::common::deadline::Deadline,
    client: tokio::sync::watch::Sender<Option<Arc<DesktopClient>>>,
}
impl DaemonServiceGateway {
    pub fn new(
        executable: PathBuf,
        data_dir: PathBuf,
        limiter: Arc<RetryLimiter>,
        deadline: crate::common::deadline::Deadline,
        client: tokio::sync::watch::Sender<Option<Arc<DesktopClient>>>,
    ) -> Self {
        Self {
            executable,
            data_dir,
            limiter,
            deadline,
            client,
        }
    }
    pub fn client(&self) -> Result<Arc<DesktopClient>, String> {
        self.client
            .borrow()
            .clone()
            .ok_or_else(|| "サーバに接続していません".into())
    }
}
fn daemon_failure(error: daemon::DaemonError) -> DaemonConnectionFailure {
    match error {
        daemon::DaemonError::Exited { status, stderr } => DaemonConnectionFailure::StartupFailed {
            status: Some(status.to_string()),
            stderr,
        },
        daemon::DaemonError::StartupTimeout { stderr } => DaemonConnectionFailure::StartupFailed {
            status: None,
            stderr,
        },
        error => DaemonConnectionFailure::TechnicalFailure(error.to_string()),
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
                .map_err(|e| DaemonConnectionFailure::TechnicalFailure(e.to_string()))?;
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
            self.client.send_replace(None);
            Ok(())
        })
    }
    fn connect<'a>(&'a self, endpoint: &'a DaemonEndpoint) -> DaemonResult<'a, DaemonSubscription> {
        Box::pin(async move {
            self.client.send_replace(None);
            let connection = ClientConnectionDto {
                url: endpoint.url.clone(),
                token: endpoint.token.clone(),
            };
            let subscription = DaemonSubscription(uuid::Uuid::new_v4().as_u128());
            let client = Arc::new(DesktopClient::start(
                subscription,
                desktop_client::client(&connection)
                    .map_err(DaemonConnectionFailure::TechnicalFailure)?,
                desktop_client::stream_client(&connection)
                    .map_err(DaemonConnectionFailure::TechnicalFailure)?,
                self.limiter.clone(),
            ));
            self.deadline
                .call(client.first_settings())
                .await
                .map_err(|_| DaemonConnectionFailure::InitialSettingsUnavailable { detail: None })?
                .map_err(|e| DaemonConnectionFailure::InitialSettingsUnavailable {
                    detail: Some(e.message),
                })?;
            self.client.send_replace(Some(client));
            Ok(subscription)
        })
    }
}
impl DaemonConnectionQueryService for DaemonServiceGateway {
    fn initial_settings(&self) -> Option<DesktopSettingsDto> {
        self.client().ok()?.initial_settings()
    }
    fn settings(&self) -> Option<DesktopSettingsDto> {
        self.client().ok()?.current_settings()
    }
}

#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
