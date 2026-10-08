use super::*;

#[test]
fn test_起動失敗_未起動の観測で保持し次の起動か接続成功で消す() {
    // Given
    let failed = DaemonConnectionFailure::StartupFailed {
        status: Some("exit status: 7".into()),
        stderr: "startup failure".into(),
    };
    let mut connection = DaemonConnection::default();
    // When / Then
    connection.failed(failed.clone());
    assert_eq!(connection.observe_not_running(), failed);
    assert_eq!(
        connection.state(),
        &DaemonConnectionState::Failed(failed.clone())
    );
    connection.begin_start();
    assert_eq!(connection.state(), &DaemonConnectionState::NotObserved);
    connection.failed(failed);
    let endpoint = DaemonEndpoint {
        url: "localhost".into(),
        token: "test".into(),
    };
    connection.connected(endpoint.clone(), DaemonSubscription(1));
    assert_eq!(
        connection.state(),
        &DaemonConnectionState::Connected(endpoint, DaemonSubscription(1))
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
        assert_eq!(
            connection.assess(&server, 1, "client"),
            connection.failure().cloned()
        );
        assert_eq!(
            connection.state(),
            &DaemonConnectionState::Failed(DaemonConnectionFailure::Incompatible {
                server_older: expected,
                server_release: "server".into(),
                client_release: "client".into()
            })
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
    assert_eq!(connection.assess(&server, 1, "client"), None);
}

#[test]
fn test_接続状態_未観測と接続済み以外を失敗として答える() {
    // Given
    let mut connection = DaemonConnection::default();
    // When / Then
    assert!(connection.failure().is_none());
    assert!(!connection.is_connected());
    for state in [
        DaemonConnectionFailure::NotRunning,
        DaemonConnectionFailure::Incompatible {
            server_older: true,
            server_release: "server".into(),
            client_release: "client".into(),
        },
        DaemonConnectionFailure::StartupFailed {
            status: None,
            stderr: "failure".into(),
        },
        DaemonConnectionFailure::InitialSettingsUnavailable { detail: None },
        DaemonConnectionFailure::TechnicalFailure("failure".into()),
    ] {
        connection.failed(state);
        assert!(connection.failure().is_some());
        assert!(!connection.is_connected());
    }
    connection.connected(
        DaemonEndpoint {
            url: "localhost".into(),
            token: "test".into(),
        },
        DaemonSubscription(1),
    );
    assert!(connection.failure().is_none());
    assert!(connection.is_connected());
}

#[test]
fn test_未起動の観測_接続済みと非互換と技術的失敗から動いていない状態へ遷移する() {
    // Given
    let endpoint = DaemonEndpoint {
        url: "localhost".into(),
        token: "test".into(),
    };
    let mut connection = DaemonConnection::default();
    connection.connected(endpoint, DaemonSubscription(1));
    // When / Then
    assert_eq!(
        connection.observe_not_running(),
        DaemonConnectionFailure::NotRunning
    );
    assert_eq!(
        connection.state(),
        &DaemonConnectionState::Failed(DaemonConnectionFailure::NotRunning)
    );
    for failure in [
        DaemonConnectionFailure::Incompatible {
            server_older: true,
            server_release: "server".into(),
            client_release: "client".into(),
        },
        DaemonConnectionFailure::TechnicalFailure("disconnected".into()),
    ] {
        // Given
        connection.failed(failure);
        // When / Then
        assert_eq!(
            connection.observe_not_running(),
            DaemonConnectionFailure::NotRunning
        );
        assert_eq!(
            connection.state(),
            &DaemonConnectionState::Failed(DaemonConnectionFailure::NotRunning)
        );
    }
}

#[test]
fn test_通知受理_同じ接続先でも現在の購読だけ受理する() {
    // Given
    let endpoint = DaemonEndpoint {
        url: "localhost".into(),
        token: "test".into(),
    };
    let mut connection = DaemonConnection::default();
    let old = DaemonSubscription(1);
    let current = DaemonSubscription(2);
    assert!(!connection.is_current_subscription(old));
    connection.connected(endpoint.clone(), old);
    assert!(connection.is_connected_to(&endpoint));
    assert!(connection.is_current_subscription(old));
    // When
    connection.connected(endpoint.clone(), current);
    // Then
    assert!(connection.is_connected_to(&endpoint));
    assert!(!connection.is_current_subscription(old));
    assert!(connection.is_current_subscription(current));
    connection.failed(DaemonConnectionFailure::TechnicalFailure(
        "discovery failed".into(),
    ));
    assert!(!connection.is_current_subscription(current));
}
