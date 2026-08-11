use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};

fn write_discovery(data_dir: &Path, port: u16, token: &str) {
    let pid = std::process::id();
    std::fs::write(
        local_api_discovery_path(data_dir),
        serde_json::json!({
            "port": port,
            "token": token,
            "instance_id": "test-instance",
            "pid": pid,
            "process_started_at": process_start_time(pid).unwrap(),
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn test_local_api認証_discoveryのbearer_tokenをrequestへ設定する() {
    let client = LocalApiHttpClient {
        base_url: Url::parse("http://127.0.0.1:43123/").unwrap(),
        token: "secret".to_string(),
        client: Client::builder().no_proxy().build().unwrap(),
    };
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
fn test_local_api接続_proxy環境変数を無視してloopbackへ直接接続する() {
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
        let (mut identity_stream, _) = target.accept().unwrap();
        let mut identity_request = [0_u8; 4096];
        let identity_read = identity_stream.read(&mut identity_request).unwrap();
        let identity_request = String::from_utf8_lossy(&identity_request[..identity_read]);
        assert!(identity_request
            .starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
        assert!(!identity_request
            .to_ascii_lowercase()
            .contains("authorization:"));
        identity_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();

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
    assert!(matches!(proxy.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock));
}

#[test]
fn test_local_api応答_body_timeoutをrequest_errorとして返す() {
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
fn test_local_api_discovery_終了済みprocessの接続先を拒否する() {
    let temp = TempDir::new().unwrap();
    std::fs::write(
        local_api_discovery_path(temp.path()),
        serde_json::json!({
            "port": 43123,
            "token": "stale-secret",
            "instance_id": "stale-instance",
            "pid": i32::MAX as u32,
            "process_started_at": 1,
        })
        .to_string(),
    )
    .unwrap();

    let result = LocalApiHttpClient::discover(temp.path());

    assert!(matches!(
        result,
        Err(LocalApiClientError::InvalidDiscovery { .. })
    ));
}

#[test]
fn test_local_api_discovery_pid再利用で開始時刻が異なる接続先を拒否する() {
    let temp = TempDir::new().unwrap();
    let pid = std::process::id();
    let current_started_at = process_start_time(pid).unwrap();
    std::fs::write(
        local_api_discovery_path(temp.path()),
        serde_json::json!({
            "port": 43123,
            "token": "stale-secret",
            "instance_id": "stale-instance",
            "pid": pid,
            "process_started_at": current_started_at.saturating_add(1),
        })
        .to_string(),
    )
    .unwrap();

    let result = LocalApiHttpClient::discover(temp.path());

    assert!(matches!(
        result,
        Err(LocalApiClientError::InvalidDiscovery { .. })
    ));
}
