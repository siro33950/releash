use super::*;
fn daemon() -> Daemon {
    Daemon::new(
        DaemonIdentity {
            daemon_id: "daemon".into(),
            pid: 42,
            process_started_at: 123,
        },
        "release".into(),
        1,
    )
}
#[test]
fn test_daemon受付_全状態で情報と停止だけが常に許可される() {
    // Given
    let mut daemon = daemon();
    // When / Then
    for status in [
        ServingStatus::Starting,
        ServingStatus::Serving,
        ServingStatus::Stopping,
        ServingStatus::Stopped,
        ServingStatus::Failed(StartupFailure::new(
            StartupFailureKind::StoreInUse,
            "correlation".into(),
        )),
    ] {
        daemon.info.serving_status = status.clone();
        assert!(daemon.admits(DaemonRequest::Status));
        assert!(daemon.admits(DaemonRequest::Stop));
        assert_eq!(
            daemon.admits(DaemonRequest::Operation),
            status == ServingStatus::Serving
        );
    }
}
#[test]
fn test_daemon遷移_最初の停止コードを保持し重複を受理する() {
    // Given
    let mut daemon = daemon();
    assert_eq!(daemon.info().serving_status, ServingStatus::Starting);
    // When / Then
    daemon.serve();
    let info = daemon.info();
    assert_eq!(info.serving_status, ServingStatus::Serving);
    assert_eq!(info.identity.daemon_id, "daemon");
    assert_eq!(info.release, "release");
    assert_eq!(info.protocol, 1);
    assert!(info.capabilities.is_empty());
    assert_eq!(
        daemon.stop(StopRequest::Exit { code: -7 }),
        StopAcceptance::Started { code: -7 }
    );
    for code in 0..100 {
        assert_eq!(
            daemon.stop(StopRequest::Exit { code }),
            StopAcceptance::AlreadyAccepted
        );
    }
    assert_eq!(daemon.exit_code, Some(-7));
    assert_eq!(daemon.info().serving_status, ServingStatus::Stopping);
    daemon.serve();
    assert_eq!(daemon.info().serving_status, ServingStatus::Stopping);
    daemon.stopped();
    assert_eq!(daemon.info().serving_status, ServingStatus::Stopped);
}
#[test]
fn test_daemon起動失敗_理由と相関idを状態として保持する() {
    // Given
    let mut daemon = daemon();
    let failure = StartupFailure::new(
        StartupFailureKind::StoreValidationFailed,
        "correlation".into(),
    );
    // When
    daemon.fail(failure.clone());
    // Then
    assert_eq!(daemon.info().serving_status, ServingStatus::Failed(failure));
    assert!(!daemon.admits(DaemonRequest::Operation));
}
#[test]
fn test_daemon起動失敗_すべての分類の説明に内部の詳細を含まない() {
    // Given
    let kinds = [
        StartupFailureKind::StoreInUse,
        StartupFailureKind::StorageUnavailable(
            crate::domain::failure::TechnicalFailureNature::Transient,
        ),
        StartupFailureKind::StorageUnavailable(
            crate::domain::failure::TechnicalFailureNature::TimedOut,
        ),
        StartupFailureKind::StorageUnavailable(
            crate::domain::failure::TechnicalFailureNature::Cancelled,
        ),
        StartupFailureKind::StorageUnavailable(
            crate::domain::failure::TechnicalFailureNature::Other,
        ),
        StartupFailureKind::UnsupportedRuntime,
        StartupFailureKind::UnsupportedStoreVersion,
        StartupFailureKind::InitializationStateInvalid,
        StartupFailureKind::StoreValidationFailed,
        StartupFailureKind::SchemaEvolutionFailed,
    ];
    for kind in kinds {
        // When
        let description = kind.safe_description().to_ascii_lowercase();
        // Then
        for forbidden in [
            "select ",
            "pragma ",
            "sqlite_",
            ".db",
            "/users/",
            "\\users\\",
            "session",
            "workflow",
        ] {
            assert!(
                !description.contains(forbidden),
                "{kind:?} leaked forbidden detail {forbidden:?}"
            );
        }
    }
}
#[test]
fn test_daemon同一性_不正な記録とプロセス観測と接続先を区別する() {
    // Given
    let identity = daemon().info().identity;
    // When / Then
    assert_eq!(
        identity.assess_process(true, ProcessObservation::StartedAt(123)),
        Ok(())
    );
    for observation in [
        ProcessObservation::ProcessNotFound,
        ProcessObservation::StartedAt(124),
    ] {
        assert_eq!(
            identity.assess_process(true, observation),
            Err(DiscoveryRejection::InvalidOrStale)
        );
    }
    assert_eq!(
        identity.assess_process(false, ProcessObservation::StartedAt(123)),
        Err(DiscoveryRejection::InvalidOrStale)
    );
    assert_eq!(
        identity.assess_process(true, ProcessObservation::Unavailable),
        Err(DiscoveryRejection::ProcessInformationUnavailable)
    );
    for invalid in [
        DaemonIdentity {
            daemon_id: " ".into(),
            ..identity.clone()
        },
        DaemonIdentity {
            pid: 0,
            ..identity.clone()
        },
        DaemonIdentity {
            process_started_at: 0,
            ..identity.clone()
        },
    ] {
        assert!(!invalid.is_valid());
    }
    assert_eq!(
        ProcessObservation::from_raw(false, None),
        ProcessObservation::Unavailable
    );
    assert_eq!(
        ProcessObservation::from_raw(true, None),
        ProcessObservation::ProcessNotFound
    );
    assert_eq!(
        ProcessObservation::from_raw(true, Some(0)),
        ProcessObservation::ProcessNotFound
    );
    assert_eq!(
        ProcessObservation::from_raw(true, Some(123)),
        ProcessObservation::StartedAt(123)
    );
    assert!(identity.matches_client(&identity, true));
    assert!(!identity.matches_client(&identity, false));
    assert!(!identity.matches_client(
        &DaemonIdentity {
            pid: 43,
            ..identity.clone()
        },
        true
    ));
    for (observation, expected) in [
        (ConnectionObservation::IdentityVerified, Ok(())),
        (
            ConnectionObservation::UnexpectedResponse,
            Err(DiscoveryRejection::InstanceMismatch),
        ),
        (
            ConnectionObservation::NoResponse,
            Err(DiscoveryRejection::ConnectionUnreachable),
        ),
    ] {
        assert_eq!(identity.assess_connection(observation), expected);
    }
}
