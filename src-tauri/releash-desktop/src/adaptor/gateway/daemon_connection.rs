use super::desktop_client::{self, DesktopClient};
use crate::domain::daemon_connection::{ConnectionError, DaemonConnectionPort, DaemonEndpoint};
use crate::usecase::daemon_connection_query::{ConnectionFailure, DaemonConnectionQueryService};
use releash_sdk::{compatibility::Compatibility, daemon};
use releashd::desktop_api::{ClientConnectionDto, DesktopSettingsDto};
use std::{path::PathBuf, sync::Arc};

pub struct DaemonConnection {
    executable: PathBuf,
    data_dir: PathBuf,
    client: parking_lot::RwLock<Option<(DaemonEndpoint, Arc<DesktopClient>)>>,
    failure: parking_lot::RwLock<Option<ConnectionError>>,
}

impl DaemonConnection {
    pub fn new(executable: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            executable,
            data_dir,
            client: parking_lot::RwLock::new(None),
            failure: parking_lot::RwLock::new(None),
        }
    }
    pub fn client(&self) -> Result<Arc<DesktopClient>, String> {
        self.client
            .read()
            .as_ref()
            .map(|(_, client)| client.clone())
            .ok_or_else(|| "サーバに接続していません".into())
    }
}

#[async_trait::async_trait]
impl DaemonConnectionPort for DaemonConnection {
    async fn discover(&self) -> Result<Option<DaemonEndpoint>, ConnectionError> {
        let Some(discovery) =
            daemon::running(&self.data_dir).map_err(|e| ConnectionError::from(e.to_string()))?
        else {
            return Ok(None);
        };
        let info = daemon::server_info(&discovery)
            .await
            .map_err(|e| ConnectionError::from(e.to_string()))?;
        let compatibility =
            Compatibility::assess(releash_sdk::descriptor::protocol(), info.protocol);
        if compatibility != Compatibility::Compatible {
            let server_older = compatibility == Compatibility::ServerOlder;
            return Err(ConnectionError {
                server_older,
                message: format!(
                    "{}（サーバ {}, 画面 {}）",
                    if server_older {
                        "サーバが古い"
                    } else {
                        "画面が古い"
                    },
                    info.release,
                    env!("CARGO_PKG_VERSION")
                ),
            });
        }
        Ok(Some(DaemonEndpoint {
            url: format!("http://127.0.0.1:{}", discovery.port),
            token: discovery.token,
        }))
    }
    async fn start(&self) -> Result<(), ConnectionError> {
        let cwd = std::env::current_dir().map_err(|e| ConnectionError::from(e.to_string()))?;
        daemon::start(&self.executable, &self.data_dir, &cwd)
            .await
            .map_err(|e| ConnectionError::from(e.to_string()))?;
        Ok(())
    }
    async fn subscribe(&self, endpoint: &DaemonEndpoint) -> Result<(), ConnectionError> {
        *self.client.write() = None;
        let connection = ClientConnectionDto {
            url: endpoint.url.clone(),
            token: endpoint.token.clone(),
        };
        let client = Arc::new(DesktopClient::start(
            desktop_client::client(&connection).map_err(ConnectionError::from)?,
            desktop_client::stream_client(&connection).map_err(ConnectionError::from)?,
            Arc::new(crate::common::retry::RetryLimiter::new()),
        ));
        tokio::time::timeout(
            daemon::timeout("min_connect_timeout_ms"),
            client.first_settings(),
        )
        .await
        .map_err(|e| ConnectionError::from(e.to_string()))?
        .map_err(|e| ConnectionError::from(e.message))?;
        *self.client.write() = Some((endpoint.clone(), client));
        Ok(())
    }
    fn subscribed_to(&self, endpoint: &DaemonEndpoint) -> bool {
        self.client
            .read()
            .as_ref()
            .is_some_and(|(current, _)| current == endpoint)
    }
    async fn stop(&self) -> Result<(), ConnectionError> {
        if let Some(discovery) =
            daemon::running(&self.data_dir).map_err(|e| ConnectionError::from(e.to_string()))?
        {
            daemon::stop(&self.data_dir, &discovery)
                .await
                .map_err(|e| ConnectionError::from(e.to_string()))?;
        }
        *self.client.write() = None;
        Ok(())
    }
    fn record_failure(&self, failure: Option<ConnectionError>) {
        *self.failure.write() = failure;
    }
}
impl DaemonConnectionQueryService for DaemonConnection {
    fn failure(&self) -> Option<ConnectionFailure> {
        self.failure
            .read()
            .as_ref()
            .map(|failure| ConnectionFailure {
                message: failure.message.clone(),
                server_older: failure.server_older,
            })
    }
    fn settings(&self) -> Option<DesktopSettingsDto> {
        self.client().ok()?.current_settings()
    }
    fn settings_update(&self) -> Option<DesktopSettingsDto> {
        self.client().ok()?.settings_update()
    }
}
