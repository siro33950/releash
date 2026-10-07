use super::*;

#[test]
fn test_接続後の窓_ログイン起動の設定と失敗窓からの復旧に従う() {
    // Given
    for (hidden, start_minimized, failure, expected) in [
        (true, true, false, false),
        (false, true, false, true),
        (true, false, false, true),
        (true, true, true, true),
    ] {
        // When
        let show = DaemonConnection::show_after_connection(hidden, start_minimized, failure);
        // Then
        assert_eq!(show, expected);
    }
}

#[test]
fn test_起動失敗_未起動の観測で保持し次の起動か接続成功で消す() {
    // Given
    let failed = DaemonConnectionState::StartupFailed {
        status: Some("exit status: 7".into()),
        stderr: "startup failure".into(),
    };
    let mut connection = DaemonConnection::default();
    // When / Then
    connection.failed(failed.clone());
    connection.observe_not_running();
    assert_eq!(connection.state(), &failed);
    connection.begin_start();
    assert_eq!(connection.state(), &DaemonConnectionState::NotObserved);
    connection.failed(failed);
    let endpoint = DaemonEndpoint {
        url: "localhost".into(),
        token: "test".into(),
    };
    connection.connected(endpoint.clone());
    assert_eq!(
        connection.state(),
        &DaemonConnectionState::Connected(endpoint)
    );
}
#[test]
fn test_互換判定_非互換なら双方の版と古い側を保持する() {
    // Given
    for (protocol, expected) in [(0, true), (2, false)] {
        let mut connection = DaemonConnection::default();
        let server = DiscoveredDaemon {
            endpoint: DaemonEndpoint {
                url: "localhost".into(),
                token: "test".into(),
            },
            protocol,
            release: "server".into(),
        };
        // When / Then
        assert!(!connection.assess(&server, 1, "client"));
        assert_eq!(
            connection.state(),
            &DaemonConnectionState::Incompatible {
                server_older: expected,
                server_release: "server".into(),
                client_release: "client".into()
            }
        );
    }
    let mut connection = DaemonConnection::default();
    let server = DiscoveredDaemon {
        endpoint: DaemonEndpoint {
            url: "localhost".into(),
            token: "test".into(),
        },
        protocol: 1,
        release: "different release".into(),
    };
    assert!(connection.assess(&server, 1, "client"));
}
