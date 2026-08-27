use std::io::{Read, Write};
use std::net::TcpListener;

use tempfile::TempDir;

use super::*;

fn discovery(port: u16, token: &str) -> LocalApiDiscovery {
    LocalApiDiscovery {
        port,
        token: token.to_string(),
        instance_id: "test-instance".to_string(),
        pid: 42,
        process_started_at: 123,
    }
}

#[test]
fn test_local_api_discovery読取_file不在をnoneとして返す() {
    // Given
    let temp = TempDir::new().unwrap();

    // When
    let result = read_local_api_discovery(temp.path()).unwrap();

    // Then
    assert_eq!(result, None);
}

#[test]
fn test_local_api_discovery読取_jsonの生値を返す() {
    // Given
    let temp = TempDir::new().unwrap();
    let expected = discovery(43123, "secret");
    std::fs::write(
        local_api_discovery_path(temp.path()),
        serde_json::to_vec(&expected).unwrap(),
    )
    .unwrap();

    // When
    let result = read_local_api_discovery(temp.path()).unwrap();

    // Then
    assert_eq!(result, Some(expected));
}

#[test]
fn test_local_api_discovery読取_不正jsonをdecode_errorとして返す() {
    // Given
    let temp = TempDir::new().unwrap();
    let path = local_api_discovery_path(temp.path());
    std::fs::write(&path, "not-json").unwrap();

    // When
    let error = read_local_api_discovery(temp.path()).unwrap_err();

    // Then
    assert!(matches!(
        error,
        LocalApiDiscoveryReadError::Decode { path: error_path, .. } if error_path == path
    ));
}

#[test]
fn test_local_api認証_discoveryのbearer_tokenをrequestへ設定する() {
    // Given
    let client = LocalApiHttpClient::new(discovery(43123, "secret")).unwrap();

    // When
    let request = client
        .authenticated(client.client.get(client.base_url.clone()))
        .build()
        .unwrap();

    // Then
    assert_eq!(
        request
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .unwrap(),
        "Bearer secret"
    );
}

#[test]
fn test_local_api接続先観測_http_statusの生値を返す() {
    // Given
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    let client = LocalApiHttpClient::new(discovery(port, "raw-secret")).unwrap();

    // When
    let status = client.identity_status("test-instance").unwrap();
    let request = server.join().unwrap();

    // Then
    assert_eq!(status, 404);
    assert!(request.starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    assert!(!request.contains("raw-secret"));
}

#[test]
fn test_local_api応答_body_timeoutをrequest_errorとして返す() {
    // Given
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

    // When
    let error = client
        .get_json::<serde_json::Value>(&["v1", "test"], &[])
        .unwrap_err();

    // Then
    assert!(matches!(error, LocalApiTransportError::Request(_)));
    server.join().unwrap();
}
