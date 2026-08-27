use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

use super::*;
use crate::infrastructure::local_api::process_start_time;
use crate::test_support::{EnvVarGuard, TEST_ENV_LOCK};

fn write_discovery(data_dir: &Path, port: u16, token: &str) {
    let pid = std::process::id();
    write_discovery_value(
        data_dir,
        serde_json::json!({
            "port": port,
            "token": token,
            "instance_id": "test-instance",
            "pid": pid,
            "process_started_at": process_start_time(pid).unwrap(),
        }),
    );
}

fn write_discovery_value(data_dir: &Path, discovery: serde_json::Value) {
    std::fs::write(local_api_discovery_path(data_dir), discovery.to_string()).unwrap();
}

fn found_process(start_time: u64) -> ProcessStartTimeLookup {
    ProcessStartTimeLookup {
        process_list_available: true,
        start_time: Some(start_time),
    }
}

#[test]
fn test_local_api接続先確認_204応答をtoken送信前に受理する() {
    // Given
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    let temp = TempDir::new().unwrap();
    let token = "identity-secret";
    write_discovery(temp.path(), port, token);

    // When
    let client = LocalApiClientGateway::discover(temp.path()).unwrap();
    let request = server.join().unwrap();

    // Then
    assert!(client.is_some());
    assert!(request.starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    assert!(!request.contains(token));
}

#[test]
fn test_local_api接続_proxy環境変数を無視してloopbackへ直接接続する() {
    // Given
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

    // When
    let client = LocalApiClientGateway::discover(temp.path())
        .unwrap()
        .unwrap();
    let response: serde_json::Value = client
        .post_json(
            &["v1", "test"],
            &serde_json::json!({"marker": "proxy-secret-marker"}),
        )
        .unwrap();

    // Then
    assert_eq!(response, serde_json::json!({"ok": true}));
    server.join().unwrap();
    let direct_request = String::from_utf8_lossy(&received.lock().unwrap()).into_owned();
    assert!(direct_request
        .to_ascii_lowercase()
        .contains("authorization: bearer proxy-secret-token"));
    assert!(direct_request.contains("proxy-secret-marker"));
    assert!(matches!(
        proxy.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[test]
fn test_local_api接続先確認_process情報参照不能を専用失敗として返す() {
    // Given
    let temp = TempDir::new().unwrap();
    let token = "unavailable-process-secret";
    write_discovery_value(
        temp.path(),
        serde_json::json!({
            "port": 43123,
            "token": token,
            "instance_id": "test-instance",
            "pid": 42,
            "process_started_at": 123,
        }),
    );

    // When
    let error = LocalApiClientGateway::discover_with_process_lookup(temp.path(), |_| {
        ProcessStartTimeLookup {
            process_list_available: false,
            start_time: None,
        }
    })
    .unwrap_err();

    // Then
    assert!(matches!(
        error,
        LocalApiClientError::ProcessInformationUnavailable
    ));
    let message = error.to_string();
    assert_eq!(
        message,
        "プロセス情報を参照できないため、local API の接続先を確認できませんでした"
    );
    assert!(!message.contains("不正"));
    assert!(!message.contains("古い"));
    assert!(!message.contains(token));
    assert!(!message.contains(&local_api_discovery_path(temp.path()).display().to_string()));
}

#[test]
fn test_local_api接続先確認_process情報参照不能なら接続を試みない() {
    // Given
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let temp = TempDir::new().unwrap();
    write_discovery_value(
        temp.path(),
        serde_json::json!({
            "port": port,
            "token": "secret",
            "instance_id": "test-instance",
            "pid": 42,
            "process_started_at": 123,
        }),
    );

    // When
    let result = LocalApiClientGateway::discover_with_process_lookup(temp.path(), |_| {
        ProcessStartTimeLookup {
            process_list_available: false,
            start_time: None,
        }
    });

    // Then
    assert!(matches!(
        result,
        Err(LocalApiClientError::ProcessInformationUnavailable)
    ));
    assert!(matches!(
        listener.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[test]
fn test_local_api接続先確認_接続不能を確認不能として返す() {
    // Given
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let temp = TempDir::new().unwrap();
    write_discovery_value(
        temp.path(),
        serde_json::json!({
            "port": port,
            "token": "unreachable-secret",
            "instance_id": "test-instance",
            "pid": 42,
            "process_started_at": 123,
        }),
    );

    // When
    let error =
        LocalApiClientGateway::discover_with_process_lookup(temp.path(), |_| found_process(123))
            .unwrap_err();

    // Then
    let expected_message = match &error {
        LocalApiClientError::DiscoveryUnreachable {
            port: error_port,
            source,
        } => {
            assert_eq!(*error_port, port);
            format!(
				"local API の接続先 (127.0.0.1:{port}) へ接続できず、接続先を確認できませんでした。接続が拒否されたか、実行環境が loopback 接続を許可していません: {source}"
			)
        }
        error => panic!("expected DiscoveryUnreachable, got {error:?}"),
    };
    assert_eq!(error.to_string(), expected_message);
}

#[test]
fn test_local_api接続先確認_別instance応答をtoken送信せず不一致として返す() {
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
    let temp = TempDir::new().unwrap();
    let token = "mismatch-secret";
    write_discovery(temp.path(), port, token);
    let discovery_path = local_api_discovery_path(temp.path());

    // When
    let error = LocalApiClientGateway::discover(temp.path()).unwrap_err();
    let request = server.join().unwrap();

    // Then
    assert!(matches!(
        &error,
        LocalApiClientError::DiscoveryInstanceMismatch { path } if path == &discovery_path
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "local API discovery が別のインスタンスを指しているか、古くなっています ({})",
            discovery_path.display()
        )
    );
    assert!(request.starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    assert!(!request.contains(token));
}

#[test]
fn test_local_api接続先確認_redirectを追従せずtoken送信前に不一致として返す() {
    // Given
    let redirect_target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    redirect_target.set_nonblocking(true).unwrap();
    let redirect_target_port = redirect_target.local_addr().unwrap().port();
    let discovery_target = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let discovery_port = discovery_target.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = discovery_target.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        let response = format!(
			"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{redirect_target_port}/identity\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
		);
        stream.write_all(response.as_bytes()).unwrap();
        String::from_utf8_lossy(&request[..read]).into_owned()
    });
    let temp = TempDir::new().unwrap();
    let token = "redirect-secret";
    write_discovery(temp.path(), discovery_port, token);
    let discovery_path = local_api_discovery_path(temp.path());

    // When
    let error = LocalApiClientGateway::discover(temp.path()).unwrap_err();
    let request = server.join().unwrap();

    // Then
    assert!(matches!(
        &error,
        LocalApiClientError::DiscoveryInstanceMismatch { path } if path == &discovery_path
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "local API discovery が別のインスタンスを指しているか、古くなっています ({})",
            discovery_path.display()
        )
    );
    assert!(request.starts_with("GET /.well-known/releash-local-api/test-instance HTTP/1.1"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
    assert!(!request.contains(token));
    assert!(matches!(
        redirect_target.accept(),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
    ));
}

#[test]
fn test_local_api_discovery_終了済みprocessを従来の表示で拒否する() {
    // Given
    let temp = TempDir::new().unwrap();
    let path = local_api_discovery_path(temp.path());
    write_discovery_value(
        temp.path(),
        serde_json::json!({
            "port": 43123,
            "token": "stale-secret",
            "instance_id": "stale-instance",
            "pid": 42,
            "process_started_at": 123,
        }),
    );

    // When
    let error = LocalApiClientGateway::discover_with_process_lookup(temp.path(), |_| {
        ProcessStartTimeLookup {
            process_list_available: true,
            start_time: None,
        }
    })
    .unwrap_err();

    // Then
    assert!(matches!(
        &error,
        LocalApiClientError::InvalidDiscovery { path: error_path } if error_path == &path
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "local API discovery file が不正または古いです ({})",
            path.display()
        )
    );
}

#[test]
fn test_local_api_discovery_pid再利用で開始時刻が異なる接続先を拒否する() {
    // Given
    let temp = TempDir::new().unwrap();
    write_discovery_value(
        temp.path(),
        serde_json::json!({
            "port": 43123,
            "token": "stale-secret",
            "instance_id": "stale-instance",
            "pid": 42,
            "process_started_at": 123,
        }),
    );

    // When
    let result =
        LocalApiClientGateway::discover_with_process_lookup(temp.path(), |_| found_process(124));

    // Then
    assert!(matches!(
        result,
        Err(LocalApiClientError::InvalidDiscovery { .. })
    ));
}

#[test]
fn test_local_api_discovery_空または0の内容を従来の表示で拒否する() {
    // Given
    let temp = TempDir::new().unwrap();
    let path = local_api_discovery_path(temp.path());
    let cases = [
        ("port", serde_json::json!(0)),
        ("token", serde_json::json!("")),
        ("instance_id", serde_json::json!("")),
        ("pid", serde_json::json!(0)),
        ("process_started_at", serde_json::json!(0)),
    ];

    for (field, value) in cases {
        let mut discovery = serde_json::json!({
            "port": 43123,
            "token": "secret",
            "instance_id": "test-instance",
            "pid": 42,
            "process_started_at": 123,
        });
        discovery[field] = value;
        write_discovery_value(temp.path(), discovery);

        // When
        let error = LocalApiClientGateway::discover_with_process_lookup(temp.path(), |_| {
            found_process(123)
        })
        .unwrap_err();

        // Then
        assert!(matches!(
            &error,
            LocalApiClientError::InvalidDiscovery { path: error_path } if error_path == &path
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "local API discovery file が不正または古いです ({})",
                path.display()
            ),
            "field: {field}"
        );
    }
}
