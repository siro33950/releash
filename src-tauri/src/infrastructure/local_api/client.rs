use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use super::{local_api_discovery_path, LocalApiDiscovery};

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
    #[error("local API discovery file が不正です ({}): port と token が必要です", path.display())]
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
    #[allow(dead_code)]
    code: String,
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
        if discovery.port == 0 || discovery.token.trim().is_empty() {
            return Err(LocalApiClientError::InvalidDiscovery { path });
        }
        Self::new(discovery).map(Some)
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
        url.query_pairs_mut().extend_pairs(query.iter().copied());
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

    pub(crate) fn post_json_with_timeout<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
        timeout: Duration,
    ) -> Result<T, LocalApiClientError> {
        let url = self.endpoint(segments)?;
        self.send(self.client.post(url).timeout(timeout).json(body))
    }

    pub(crate) fn post_empty<T: DeserializeOwned>(
        &self,
        segments: &[&str],
    ) -> Result<T, LocalApiClientError> {
        let url = self.endpoint(segments)?;
        self.send(self.client.post(url))
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, LocalApiClientError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| LocalApiClientError::InvalidEndpoint)?
            .extend(segments);
        Ok(url)
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
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};

    fn write_discovery(data_dir: &Path, port: u16, token: &str) {
        std::fs::write(
            local_api_discovery_path(data_dir),
            serde_json::json!({"port": port, "token": token, "pid": 42}).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn every_request_receives_the_discovered_bearer_token() {
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), 43123, "secret");
        let client = LocalApiHttpClient::discover(temp.path()).unwrap().unwrap();
        let request = client
            .authenticated(client.client.get(client.base_url.clone()))
            .build()
            .unwrap();

        assert_eq!(
            request
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .unwrap(),
            "Bearer secret"
        );
    }

    #[test]
    fn client_ignores_environment_proxy_and_connects_to_loopback() {
        let _lock = TEST_ENV_LOCK.lock();
        let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let proxy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let _http_proxy = EnvVarGuard::set_value("HTTP_PROXY", &proxy_url);
        let _http_proxy_lower = EnvVarGuard::set_value("http_proxy", &proxy_url);
        let _all_proxy = EnvVarGuard::set_value("ALL_PROXY", &proxy_url);
        let _all_proxy_lower = EnvVarGuard::set_value("all_proxy", &proxy_url);
        let _no_proxy = EnvVarGuard::set_value("NO_PROXY", "");
        let _no_proxy_lower = EnvVarGuard::set_value("no_proxy", "");
        let target_port = target.local_addr().unwrap().port();
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received_for_server = received.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        request.extend_from_slice(&buffer[..read]);
                        if request
                            .windows(b"proxy-secret-marker".len())
                            .any(|window| window == b"proxy-secret-marker")
                        {
                            break;
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(error) => panic!("failed to read direct request: {error}"),
                }
            }
            *received_for_server.lock().unwrap() = request;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
                )
                .unwrap();
        });
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), target_port, "proxy-secret-token");
        let client = LocalApiHttpClient::discover(temp.path()).unwrap().unwrap();

        let response: serde_json::Value = client
            .post_json(
                &["v1", "test"],
                &serde_json::json!({"marker": "proxy-secret-marker"}),
            )
            .unwrap();

        assert_eq!(response, serde_json::json!({"ok": true}));
        server.join().unwrap();
        let direct_request = String::from_utf8_lossy(&received.lock().unwrap()).into_owned();
        assert!(direct_request
            .to_ascii_lowercase()
            .contains("authorization: bearer proxy-secret-token"));
        assert!(direct_request.contains("proxy-secret-marker"));
        assert!(
            matches!(proxy.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn response_body_timeout_is_a_request_error() {
        let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let target_port = target.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n[",
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let client = LocalApiHttpClient {
            base_url: Url::parse(&format!("http://127.0.0.1:{target_port}/")).unwrap(),
            token: "secret".to_string(),
            client: Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_millis(100))
                .timeout(Duration::from_millis(100))
                .build()
                .unwrap(),
        };

        let error = client
            .get_json::<serde_json::Value>(&["v1", "test"], &[])
            .unwrap_err();

        assert!(matches!(error, LocalApiClientError::Request(_)));
        server.join().unwrap();
    }

    #[test]
    fn post_json_with_timeout_returns_a_committed_id_after_more_than_five_seconds() {
        let target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let target_port = target.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = target.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            std::thread::sleep(Duration::from_millis(5_100));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 54\r\nConnection: close\r\n\r\n{\"executionId\":\"00000000-0000-4000-8000-000000000777\"}",
                )
                .unwrap();
        });
        let temp = TempDir::new().unwrap();
        write_discovery(temp.path(), target_port, "secret");
        let client = LocalApiHttpClient::discover(temp.path()).unwrap().unwrap();

        let response: serde_json::Value = client
            .post_json_with_timeout(
                &["v1", "workflow", "executions"],
                &serde_json::json!({"workflowName": "slow-start"}),
                Duration::from_secs(305),
            )
            .unwrap();

        assert_eq!(
            response["executionId"],
            "00000000-0000-4000-8000-000000000777"
        );
        server.join().unwrap();
    }
}
