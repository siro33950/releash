use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{local_api_discovery_path, LocalApiDiscovery};

#[derive(Debug)]
pub(crate) enum LocalApiDiscoveryReadError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Decode {
        path: PathBuf,
        source: serde_json::Error,
    },
}

#[derive(Debug)]
pub(crate) enum LocalApiIdentityRequestError {
    InvalidEndpoint,
    Request(reqwest::Error),
}

#[derive(Debug)]
pub(crate) enum LocalApiTransportError {
    InvalidUrl(url::ParseError),
    ClientInitialization(reqwest::Error),
    InvalidEndpoint,
    Unavailable(reqwest::Error),
    Request(reqwest::Error),
    HttpStatus {
        status: u16,
        message: Option<String>,
    },
    Decode(serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    message: String,
}

pub(crate) fn read_local_api_discovery(
    data_dir: &Path,
) -> Result<Option<LocalApiDiscovery>, LocalApiDiscoveryReadError> {
    let path = local_api_discovery_path(data_dir);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(LocalApiDiscoveryReadError::Read { path, source }),
    };
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|source| LocalApiDiscoveryReadError::Decode { path, source })
}

#[derive(Debug, Clone)]
pub(crate) struct LocalApiHttpClient {
    base_url: Url,
    token: String,
    client: Client,
}

impl LocalApiHttpClient {
    pub(crate) fn new(discovery: LocalApiDiscovery) -> Result<Self, LocalApiTransportError> {
        let base_url = Url::parse(&format!("http://127.0.0.1:{}/", discovery.port))
            .map_err(LocalApiTransportError::InvalidUrl)?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(LocalApiTransportError::ClientInitialization)?;
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
    ) -> Result<T, LocalApiTransportError> {
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
    ) -> Result<T, LocalApiTransportError> {
        let url = self.endpoint(segments)?;
        self.send(self.client.post(url).json(body))
    }

    pub(crate) fn identity_status(
        &self,
        instance_id: &str,
    ) -> Result<u16, LocalApiIdentityRequestError> {
        let url = self
            .endpoint(&[".well-known", "releash-local-api", instance_id])
            .map_err(|error| match error {
                LocalApiTransportError::InvalidEndpoint => {
                    LocalApiIdentityRequestError::InvalidEndpoint
                }
                _ => unreachable!("endpoint only returns InvalidEndpoint"),
            })?;
        self.client
            .get(url)
            .send()
            .map(|response| response.status().as_u16())
            .map_err(LocalApiIdentityRequestError::Request)
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, LocalApiTransportError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| LocalApiTransportError::InvalidEndpoint)?
            .extend(segments);
        Ok(url)
    }

    fn send<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, LocalApiTransportError> {
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
            return Err(LocalApiTransportError::HttpStatus {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_slice(&bytes).map_err(LocalApiTransportError::Decode)
    }

    fn authenticated(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(&self.token)
    }
}

fn classify_transport_error(error: reqwest::Error) -> LocalApiTransportError {
    if error.is_connect() {
        LocalApiTransportError::Unavailable(error)
    } else {
        LocalApiTransportError::Request(error)
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_tests;
