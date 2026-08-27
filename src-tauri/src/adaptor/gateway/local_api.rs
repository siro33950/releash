use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::domain::local_api_discovery::{
    ConnectionObservation, DiscoveryAdmissionService, DiscoveryContent, DiscoveryRejection,
    ProcessObservation,
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
        let content = DiscoveryContent::new(
            discovery.port,
            discovery.token.clone(),
            discovery.instance_id.clone(),
            discovery.pid,
            discovery.process_started_at,
        );
        let process_lookup = lookup_process(discovery.pid);
        let process_observation = ProcessObservation::from_raw(
            process_lookup.process_list_available,
            process_lookup.start_time,
        );
        DiscoveryAdmissionService::assess_process(&content, process_observation)
            .map_err(|rejection| map_process_rejection(rejection, path.clone()))?;

        let port = discovery.port;
        let instance_id = discovery.instance_id.clone();
        let client = LocalApiHttpClient::new(discovery).map_err(map_transport_error)?;
        let (connection_observation, request_error) = match client.identity_status(&instance_id) {
            Ok(status) => (
                ConnectionObservation::from_response_status(Some(status)),
                None,
            ),
            Err(LocalApiIdentityRequestError::InvalidEndpoint) => {
                return Err(LocalApiClientError::InvalidEndpoint);
            }
            Err(LocalApiIdentityRequestError::Request(source)) => (
                ConnectionObservation::from_response_status(None),
                Some(source),
            ),
        };
        DiscoveryAdmissionService::assess_connection(connection_observation)
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
