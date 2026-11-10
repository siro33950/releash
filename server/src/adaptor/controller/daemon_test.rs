use super::*;
use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
use domain::daemon::StartupFailureKind as K;

#[test]
fn b071_store_open_failures_map_to_the_closed_safe_startup_vocabulary() {
    for (error, expected) in [
        (E::WriterLockHeld, K::StoreInUse),
        (
            E::StorageUnavailable(domain::failure::TechnicalFailure {
                nature: domain::failure::TechnicalFailureNature::Other,
                message: "disk full".into(),
            }),
            K::StorageUnavailable(domain::failure::TechnicalFailureNature::Other),
        ),
        (
            E::StorageUnavailable(domain::failure::TechnicalFailure {
                nature: domain::failure::TechnicalFailureNature::Transient,
                message: "source".into(),
            }),
            K::StorageUnavailable(domain::failure::TechnicalFailureNature::Transient),
        ),
        (
            E::StorageUnavailable(domain::failure::TechnicalFailure {
                nature: domain::failure::TechnicalFailureNature::TimedOut,
                message: "source".into(),
            }),
            K::StorageUnavailable(domain::failure::TechnicalFailureNature::TimedOut),
        ),
        (E::UnsupportedRuntime, K::UnsupportedRuntime),
        (E::UnsupportedStoreVersion, K::UnsupportedStoreVersion),
        (E::InitializationStateInvalid, K::InitializationStateInvalid),
        (E::StoreValidationFailed, K::StoreValidationFailed),
        (E::SchemaEvolutionFailed, K::SchemaEvolutionFailed),
    ] {
        assert_eq!(K::from(error), expected);
    }
}

#[tokio::test]
async fn test_daemon起動_配置観測に失敗しても起動し未判定の理由を公開する() {
    use crate::domain::daemon::{DaemonIdentity, DaemonRepository, DaemonRequest};
    use crate::usecase::installation::{installation_tests::FakeInstallation, InstallationUsecase};
    // Given
    for stage in ["executable", "location"] {
        let installation = InstallationUsecase(std::sync::Arc::new(FakeInstallation {
            failure_stage: Some(stage),
            ..Default::default()
        }));
        let identity = DaemonIdentity {
            daemon_id: "test".into(),
            pid: 1,
            process_started_at: 1,
        };
        // When
        let repository = compose_daemon_repository(identity, 1, &installation);
        let daemon = crate::usecase::daemon::DaemonUsecase::new(repository.clone());
        daemon.serve().await;
        let response = crate::adaptor::presenter::daemon::server_info(daemon.info().await);
        // Then
        assert!(repository.admits(DaemonRequest::Operation).await);
        assert_eq!(
            response.serving_status,
            crate::adaptor::presenter::client::ServingStatus::Serving as i32
        );
        let result = response.cli_installation.unwrap();
        assert_eq!(
            result.status,
            crate::adaptor::presenter::client::CliInstallationStatus::Undetermined as i32
        );
        assert_eq!(result.reason, "denied");
        assert!(matches!(
            installation.install_cli(),
            Err(crate::domain::installation::InstallationError::Technical(_))
        ));
    }
}

#[tokio::test]
async fn test_起動失敗_failedに遷移して安全な説明と相関idだけを返す() {
    use crate::domain::daemon::{DaemonIdentity, ServingStatus};
    use crate::usecase::installation::{installation_tests::FakeInstallation, InstallationUsecase};
    // Given
    let installation = InstallationUsecase(std::sync::Arc::new(FakeInstallation::default()));
    let repository = compose_daemon_repository(
        DaemonIdentity {
            daemon_id: "test".into(),
            pid: 1,
            process_started_at: 1,
        },
        1,
        &installation,
    );
    let daemon = crate::usecase::daemon::DaemonUsecase::new(repository);
    // When
    let error = fail_startup(&daemon, K::StoreInUse).await;
    // Then
    let ServingStatus::Failed(failure) = daemon.info().await.serving_status else {
        panic!("起動失敗が記録されていない")
    };
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert!(uuid::Uuid::parse_str(&failure.correlation_id).is_ok());
    assert_eq!(
        error.to_string(),
        format!("{} ({})", failure.safe_description, failure.correlation_id)
    );
    assert_eq!(
        failure.safe_description,
        "Local data is currently in use by another Releash process."
    );
}
