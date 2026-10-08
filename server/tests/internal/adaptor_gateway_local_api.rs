use releashd::test_support::integration::transport::{
    process_start_time, ClientConnectionFileQuery, ClientConnectionQueryService, LocalApiDiscovery,
    LocalApiDiscoveryFile, ProcessStartTimeLookup,
};
use std::net::TcpListener;
fn found_process(start_time: u64) -> ProcessStartTimeLookup {
    ProcessStartTimeLookup {
        process_list_available: true,
        start_time: Some(start_time),
    }
}
#[test]
pub fn test_クライアント接続情報_公開入口のqueryserviceとして読み取れる() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let query = releashd::test_support::integration::transport::ClientConnectionFileQuery(
        directory.path().to_owned(),
    );
    let service: &dyn releashd::test_support::integration::transport::ClientConnectionQueryService =
        &query;
    // When
    let result = service.read();
    // Then
    assert_eq!(
        result.unwrap_err().to_string(),
        "unavailable: client discovery is unavailable"
    );
}

#[test]
pub fn test_クライアント接続情報_再起動したinstanceとtokenを再読込する() {
    use releashd::test_support::integration::transport::LocalApiDiscovery;
    use releashd::test_support::integration::transport::LocalApiDiscoveryFile;
    let directory = tempfile::tempdir().unwrap();
    // Given
    let query = releashd::test_support::integration::transport::ClientConnectionFileQuery(
        directory.path().to_owned(),
    );
    // When / Then
    assert!(query.read().is_err());
    for (instance, port) in [("first", 12345), ("second", 23456)] {
        let discovery = LocalApiDiscovery {
            port,
            token: format!("client-{instance}"),
            daemon_id: instance.into(),
            pid: std::process::id(),
            process_started_at: process_start_time(std::process::id()).unwrap(),
            ..Default::default()
        };

        LocalApiDiscoveryFile::create_client(directory.path(), discovery).unwrap();
        let endpoint = query.read().unwrap();
        assert_eq!(endpoint.url, format!("http://127.0.0.1:{port}"));
        assert_eq!(endpoint.token, format!("client-{instance}"));
    }
    std::fs::write(directory.path().join("client-api.json"), b"invalid").unwrap();
    assert!(query.read().is_err());
}

#[test]
pub fn test_クライアント接続情報_停止済みとpid再利用と参照不能では接続先もtokenも返さない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let discovery = LocalApiDiscovery {
        port: listener.local_addr().unwrap().port().into(),
        token: "client-secret".into(),
        daemon_id: "instance".into(),
        pid: 42,
        process_started_at: 123,
        ..Default::default()
    };
    LocalApiDiscoveryFile::create_client(directory.path(), discovery).unwrap();
    let query = ClientConnectionFileQuery(directory.path().into());
    // When / Then
    for observation in [
        ProcessStartTimeLookup {
            process_list_available: true,
            start_time: None,
        },
        found_process(124),
        ProcessStartTimeLookup {
            process_list_available: false,
            start_time: None,
        },
    ] {
        let error = query
            .read_with_process_lookup(|pid| {
                assert_eq!(pid, 42);
                observation
            })
            .unwrap_err();
        assert!(!error.to_string().contains("secret"));
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
    assert!(query
        .read_with_process_lookup(|_| found_process(123))
        .is_ok());
}

#[test]
pub fn test_クライアント接続情報_clientだけで不正なendpointと古いprocessを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let client = LocalApiDiscovery {
        port: 12345,
        token: "client-secret".into(),
        daemon_id: "instance".into(),
        pid: 42,
        process_started_at: 123,
        ..Default::default()
    };
    let query = ClientConnectionFileQuery(directory.path().into());
    for invalid in [
        LocalApiDiscovery {
            port: 0,
            ..client.clone()
        },
        LocalApiDiscovery {
            token: " ".into(),
            ..client.clone()
        },
        LocalApiDiscovery {
            daemon_id: " ".into(),
            ..client.clone()
        },
        LocalApiDiscovery {
            process_started_at: 124,
            ..client.clone()
        },
    ] {
        LocalApiDiscoveryFile::create_client(directory.path(), invalid).unwrap();
        // When / Then
        assert!(query
            .read_with_process_lookup(|_| found_process(123))
            .is_err());
    }
    LocalApiDiscoveryFile::create_client(directory.path(), client).unwrap();
    assert_eq!(
        query
            .read_with_process_lookup(|_| found_process(123))
            .unwrap()
            .token,
        "client-secret"
    );
}
