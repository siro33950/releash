use super::*;
use std::error::Error;

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
