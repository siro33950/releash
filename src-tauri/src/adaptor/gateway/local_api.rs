use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::domain::daemon::{
    ConnectionObservation, DaemonIdentity, DiscoveryRejection, ProcessObservation,
};
use crate::infrastructure::local_api::{
    local_api_discovery_path, lookup_process_start_time, read_local_api_discovery,
    LocalApiDiscoveryReadError, LocalApiHttpClient, LocalApiIdentityRequestError,
    LocalApiTransportError, ProcessStartTimeLookup,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum LocalApiClientError {
    #[error("local API discovery file の読み込みに失敗しました ({}): {source}", path.display())]
    DiscoveryRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("local API discovery file が不正です ({}): {source}", path.display())]
    DiscoveryDecode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("local API discovery file が不正または古いです ({})", path.display())]
    InvalidDiscovery { path: PathBuf },
    #[error("プロセス情報を参照できないため、local API の接続先を確認できませんでした")]
    ProcessInformationUnavailable,
    #[error(
		"local API の接続先 (127.0.0.1:{port}) へ接続できず、接続先を確認できませんでした。接続が拒否されたか、実行環境が loopback 接続を許可していません: {source}"
	)]
    DiscoveryUnreachable {
        port: u16,
        #[source]
        source: reqwest::Error,
    },
    #[error("local API discovery が別のインスタンスを指しているか、古くなっています ({})", path.display())]
    DiscoveryInstanceMismatch { path: PathBuf },
    #[error("local API URL が不正です: {0}")]
    InvalidUrl(#[source] url::ParseError),
    #[error("local API client の初期化に失敗しました: {0}")]
    ClientInitialization(#[source] reqwest::Error),
    #[error("local API URL を構築できません")]
    InvalidEndpoint,
    #[error("local API is unavailable: {0}")]
    Unavailable(#[source] reqwest::Error),
    #[error("local API request に失敗しました: {0}")]
    Request(#[source] reqwest::Error),
    #[error("local API error ({status})")]
    HttpStatus {
        status: u16,
        message: Option<String>,
    },
    #[error("local API response が不正です: {0}")]
    Decode(#[source] serde_json::Error),
}

#[derive(Debug, Clone)]
pub(crate) struct LocalApiClientGateway {
    client: LocalApiHttpClient,
}

impl LocalApiClientGateway {
    pub(crate) fn discover(data_dir: &Path) -> Result<Option<Self>, LocalApiClientError> {
        Self::discover_with_process_lookup(data_dir, lookup_process_start_time)
    }

    fn discover_with_process_lookup(
        data_dir: &Path,
        lookup_process: impl FnOnce(u32) -> ProcessStartTimeLookup,
    ) -> Result<Option<Self>, LocalApiClientError> {
        let Some(discovery) =
            read_local_api_discovery(data_dir).map_err(map_discovery_read_error)?
        else {
            return Ok(None);
        };
        let path = local_api_discovery_path(data_dir);
        assess_discovery(&discovery, lookup_process)
            .map_err(|rejection| map_process_rejection(rejection, path.clone()))?;

        let port = discovery.port;
        let instance_id = discovery.instance_id.clone();
        let discovery_identity = discovery.clone();
        let client = LocalApiHttpClient::new(discovery).map_err(map_transport_error)?;
        let (connection_observation, request_error) = match client.identity_status(&instance_id) {
            Ok(status) => (identity_response(Some(status)), None),
            Err(LocalApiIdentityRequestError::InvalidEndpoint) => {
                return Err(LocalApiClientError::InvalidEndpoint);
            }
            Err(LocalApiIdentityRequestError::Request(source)) => {
                (identity_response(None), Some(source))
            }
        };
        daemon_identity(&discovery_identity)
            .assess_connection(connection_observation)
            .map_err(|rejection| map_connection_rejection(rejection, path, port, request_error))?;
        Ok(Some(Self { client }))
    }

    pub(crate) fn get_json<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, LocalApiClientError> {
        self.client
            .get_json(segments, query)
            .map_err(map_transport_error)
    }

    pub(crate) fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
    ) -> Result<T, LocalApiClientError> {
        self.client
            .post_json(segments, body)
            .map_err(map_transport_error)
    }
}

fn map_discovery_read_error(error: LocalApiDiscoveryReadError) -> LocalApiClientError {
    match error {
        LocalApiDiscoveryReadError::Read { path, source } => {
            LocalApiClientError::DiscoveryRead { path, source }
        }
        LocalApiDiscoveryReadError::Decode { path, source } => {
            LocalApiClientError::DiscoveryDecode { path, source }
        }
    }
}

fn map_process_rejection(rejection: DiscoveryRejection, path: PathBuf) -> LocalApiClientError {
    match rejection {
        DiscoveryRejection::InvalidOrStale => LocalApiClientError::InvalidDiscovery { path },
        DiscoveryRejection::ProcessInformationUnavailable => {
            LocalApiClientError::ProcessInformationUnavailable
        }
        DiscoveryRejection::InstanceMismatch | DiscoveryRejection::ConnectionUnreachable => {
            unreachable!("process assessment cannot return a connection rejection")
        }
    }
}

fn map_connection_rejection(
    rejection: DiscoveryRejection,
    path: PathBuf,
    port: u16,
    request_error: Option<reqwest::Error>,
) -> LocalApiClientError {
    match rejection {
        DiscoveryRejection::InstanceMismatch => {
            LocalApiClientError::DiscoveryInstanceMismatch { path }
        }
        DiscoveryRejection::ConnectionUnreachable => LocalApiClientError::DiscoveryUnreachable {
            port,
            source: request_error
                .expect("a connection without a response must retain its transport error"),
        },
        DiscoveryRejection::InvalidOrStale | DiscoveryRejection::ProcessInformationUnavailable => {
            unreachable!("connection assessment cannot return a process rejection")
        }
    }
}

fn map_transport_error(error: LocalApiTransportError) -> LocalApiClientError {
    match error {
        LocalApiTransportError::InvalidUrl(source) => LocalApiClientError::InvalidUrl(source),
        LocalApiTransportError::ClientInitialization(source) => {
            LocalApiClientError::ClientInitialization(source)
        }
        LocalApiTransportError::InvalidEndpoint => LocalApiClientError::InvalidEndpoint,
        LocalApiTransportError::Unavailable(source) => LocalApiClientError::Unavailable(source),
        LocalApiTransportError::Request(source) => LocalApiClientError::Request(source),
        LocalApiTransportError::HttpStatus { status, message } => {
            LocalApiClientError::HttpStatus { status, message }
        }
        LocalApiTransportError::Decode(source) => LocalApiClientError::Decode(source),
    }
}

#[cfg(test)]
#[path = "local_api_test.rs"]
mod local_api_tests;

#[cfg(any(test, feature = "desktop"))]
pub(crate) struct ClientConnectionFileQuery(pub(crate) PathBuf);

#[cfg(any(test, feature = "desktop"))]
impl crate::usecase::client_connection::ClientConnectionQueryService for ClientConnectionFileQuery {
    fn read(
        &self,
    ) -> Result<
        crate::usecase::client_connection::ClientConnectionDto,
        crate::usecase::client_connection::ClientConnectionError,
    > {
        self.read_with_process_lookup(lookup_process_start_time)
    }
}

#[cfg(any(test, feature = "desktop"))]
impl ClientConnectionFileQuery {
    fn read_with_process_lookup(
        &self,
        lookup_process: impl FnOnce(u32) -> ProcessStartTimeLookup,
    ) -> Result<
        crate::usecase::client_connection::ClientConnectionDto,
        crate::usecase::client_connection::ClientConnectionError,
    > {
        use crate::usecase::client_connection::{ClientConnectionDto, ClientConnectionError};
        let discovery = read_local_api_discovery(&self.0)
            .map_err(|_| ClientConnectionError("daemon discovery is unreadable".into()))?
            .ok_or_else(|| ClientConnectionError("daemon discovery is unavailable".into()))?;
        let client: crate::infrastructure::local_api::LocalApiDiscovery = serde_json::from_slice(
            &std::fs::read(self.0.join("client-api.json"))
                .map_err(|_| ClientConnectionError("client discovery is unreadable".into()))?,
        )
        .map_err(|_| ClientConnectionError("client discovery is invalid".into()))?;
        let endpoints_match = discovery.port != 0
            && discovery.port == client.port
            && !discovery.token.trim().is_empty()
            && !client.token.trim().is_empty()
            && discovery.token != client.token;
        if !daemon_identity(&discovery).matches_client(&daemon_identity(&client), endpoints_match) {
            return Err(ClientConnectionError(
                "client discovery does not match daemon identity".into(),
            ));
        }
        assess_discovery(&discovery, lookup_process).map_err(|rejection| {
            ClientConnectionError(
                map_process_rejection(rejection, local_api_discovery_path(&self.0)).to_string(),
            )
        })?;
        Ok(ClientConnectionDto {
            url: format!("http://127.0.0.1:{}", client.port),
            token: client.token,
        })
    }
}

fn identity_response(status: Option<u16>) -> ConnectionObservation {
    match status {
        Some(204) => ConnectionObservation::IdentityVerified,
        Some(_) => ConnectionObservation::UnexpectedResponse,
        None => ConnectionObservation::NoResponse,
    }
}

fn daemon_identity(
    discovery: &crate::infrastructure::local_api::LocalApiDiscovery,
) -> DaemonIdentity {
    DaemonIdentity {
        daemon_id: discovery.instance_id.clone(),
        pid: discovery.pid,
        process_started_at: discovery.process_started_at,
    }
}
fn assess_discovery(
    discovery: &crate::infrastructure::local_api::LocalApiDiscovery,
    lookup: impl FnOnce(u32) -> ProcessStartTimeLookup,
) -> Result<(), DiscoveryRejection> {
    let process = lookup(discovery.pid);
    daemon_identity(discovery).assess_process(
        discovery.port != 0 && !discovery.token.trim().is_empty(),
        ProcessObservation::from_raw(process.process_list_available, process.start_time),
    )
}
