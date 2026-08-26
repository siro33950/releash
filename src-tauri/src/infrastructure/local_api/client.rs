use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{local_api_discovery_path, process_start_time, LocalApiDiscovery};

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

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalApiHttpClient {
    base_url: Url,
    token: String,
    client: Client,
}

impl LocalApiHttpClient {
    pub(crate) fn discover(data_dir: &Path) -> Result<Option<Self>, LocalApiClientError> {
        let path = local_api_discovery_path(data_dir);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(LocalApiClientError::DiscoveryRead { path, source });
            }
        };
        let discovery = serde_json::from_str::<LocalApiDiscovery>(&contents).map_err(|source| {
            LocalApiClientError::DiscoveryDecode {
                path: path.clone(),
                source,
            }
        })?;
        if discovery.port == 0
            || discovery.token.trim().is_empty()
            || discovery.instance_id.trim().is_empty()
            || discovery.process_started_at == 0
            || process_start_time(discovery.pid) != Some(discovery.process_started_at)
        {
            return Err(LocalApiClientError::InvalidDiscovery { path });
        }
        let instance_id = discovery.instance_id.clone();
        let client = Self::new(discovery)?;
        if !client.matches_server_instance(&instance_id) {
            return Err(LocalApiClientError::InvalidDiscovery { path });
        }
        Ok(Some(client))
    }

    fn new(discovery: LocalApiDiscovery) -> Result<Self, LocalApiClientError> {
        let base_url = Url::parse(&format!("http://127.0.0.1:{}/", discovery.port))
            .map_err(LocalApiClientError::InvalidUrl)?;
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(LocalApiClientError::ClientInitialization)?;
        Ok(Self {
            base_url,
            token: discovery.token,
            client,
        })
    }

    pub(crate) fn get_json<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, LocalApiClientError> {
        let mut url = self.endpoint(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        self.send(self.client.get(url))
    }

    pub(crate) fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
    ) -> Result<T, LocalApiClientError> {
        let url = self.endpoint(segments)?;
        self.send(self.client.post(url).json(body))
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, LocalApiClientError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| LocalApiClientError::InvalidEndpoint)?
            .extend(segments);
        Ok(url)
    }

    fn matches_server_instance(&self, instance_id: &str) -> bool {
        let Ok(url) = self.endpoint(&[".well-known", "releash-local-api", instance_id]) else {
            return false;
        };
        self.client
            .get(url)
            .send()
            .is_ok_and(|response| response.status() == reqwest::StatusCode::NO_CONTENT)
    }

    fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, LocalApiClientError> {
        let response = self
            .authenticated(request)
            .send()
            .map_err(classify_transport_error)?;
        let status = response.status();
        let bytes = response.bytes().map_err(classify_transport_error)?;
        if !status.is_success() {
            let message = serde_json::from_slice::<ErrorResponse>(&bytes)
                .ok()
                .map(|error| error.message)
                .filter(|message| !message.trim().is_empty());
            return Err(LocalApiClientError::HttpStatus {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_slice(&bytes).map_err(LocalApiClientError::Decode)
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(&self.token)
    }
}

fn classify_transport_error(error: reqwest::Error) -> LocalApiClientError {
    if error.is_connect() {
        LocalApiClientError::Unavailable(error)
    } else {
        LocalApiClientError::Request(error)
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;
