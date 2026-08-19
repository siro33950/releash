use std::error::Error;

use super::*;

#[test]
fn test_local_api_token_空でなく起動ごとに異なる値を生成する() {
    let first = generate_token();
    let second = generate_token();
    assert_eq!(first.len(), 64);
    assert_ne!(first, second);
}

#[test]
fn test_local_api_server_error_全variantが原因errorを保持する() {
    let address: std::net::SocketAddr = "192.0.2.1:43123".parse().unwrap();
    let errors = [
        LocalApiServerError::ListenerBind(io::Error::other("bind")),
        LocalApiServerError::AddressResolution(io::Error::other("address")),
        LocalApiServerError::NonLoopback {
            address,
            source: io::Error::other("non-loopback"),
        },
        LocalApiServerError::Nonblocking(io::Error::other("nonblocking")),
        LocalApiServerError::Discovery(io::Error::other("discovery")),
    ];

    for error in errors {
        assert!(error.source().is_some(), "missing source for {error}");
    }
}

#[test]
fn test_local_api_server起動_discovery作成失敗を専用errorで返す() {
    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("not-a-directory");
    std::fs::write(&data_path, "occupied").unwrap();

    let error = match LocalApiServerBinding::bind(data_path) {
        Ok(_) => panic!("discovery creation unexpectedly succeeded"),
        Err(error) => error,
    };

    assert!(matches!(error, LocalApiServerError::Discovery(_)));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn test_local_api_server終了_停止を通知して所有discoveryを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let binding = LocalApiServerBinding::bind(directory.path().to_path_buf()).unwrap();
    let discovery_path = binding.discovery.path().to_path_buf();
    let server = binding.start(Router::new(), &tokio::runtime::Handle::current());

    assert!(discovery_path.exists());
    server.shutdown_and_wait().await.unwrap();
    assert!(!discovery_path.exists());
}
