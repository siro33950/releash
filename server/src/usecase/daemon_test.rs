use super::*;
use crate::domain::daemon::{ServingStatus, StartupFailureKind};
#[tokio::test]
async fn test_daemon操作_同じ集約を読み取り遷移させる() {
    // Given
    let repository = serving();
    let usecase = DaemonUsecase::new(repository);
    // When / Then
    assert!(usecase.admits(DaemonRequest::Operation).await);
    assert_eq!(
        usecase.stop(StopRequest::Exit { code: 23 }).await,
        StopAcceptance::Started { code: 23 }
    );
    assert_eq!(
        usecase.stop(StopRequest::Exit { code: 99 }).await,
        StopAcceptance::AlreadyAccepted
    );
    assert!(!usecase.admits(DaemonRequest::Operation).await);
    assert_eq!(usecase.info().await.serving_status, ServingStatus::Stopping);
    usecase.stopped().await;
    assert_eq!(usecase.info().await.serving_status, ServingStatus::Stopped);
    usecase
        .fail(StartupFailure::new(
            StartupFailureKind::StoreInUse,
            "id".into(),
        ))
        .await;
    usecase.serve().await;
    assert_eq!(usecase.info().await.serving_status, ServingStatus::Stopped);
}

struct FakeDaemon(parking_lot::Mutex<crate::domain::daemon::Daemon>);
fn serving() -> std::sync::Arc<FakeDaemon> {
    use crate::domain::daemon::{Daemon, DaemonIdentity};
    let mut daemon = Daemon::new(
        DaemonIdentity {
            daemon_id: "test".into(),
            pid: 1,
            process_started_at: 1,
        },
        "test".into(),
        1,
        Ok(crate::domain::installation::CliInstallation::Allowed),
    );
    daemon.serve();
    std::sync::Arc::new(FakeDaemon(parking_lot::Mutex::new(daemon)))
}
#[async_trait::async_trait]
impl crate::domain::daemon::DaemonRepository for FakeDaemon {
    async fn info(&self) -> crate::domain::daemon::DaemonInfo {
        self.0.lock().info()
    }
    async fn admits(&self, request: DaemonRequest) -> bool {
        self.0.lock().admits(request)
    }
    async fn serve(&self) {
        self.0.lock().serve();
    }
    async fn fail(&self, failure: StartupFailure) {
        self.0.lock().fail(failure);
    }
    async fn stop(&self, request: StopRequest) -> StopAcceptance {
        self.0.lock().stop(request)
    }
    async fn stopped(&self) {
        self.0.lock().stopped();
    }
}

#[tokio::test]
async fn test_daemon状態配信_停止受理で通知し重複要求では通知しない() {
    // Given
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let usecase = DaemonUsecase::new(serving()).with_state_publisher(subscriptions);
    // When
    usecase.stop(StopRequest::Exit { code: 0 }).await;
    // Then
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Daemon
    );
    assert_eq!(usecase.info().await.serving_status, ServingStatus::Stopping);
    usecase.stop(StopRequest::Exit { code: 0 }).await;
    assert!(changes.try_recv().is_err());
    usecase.stopped().await;
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::usecase::state_subscription::StateChangeSource::Daemon
    );
}
