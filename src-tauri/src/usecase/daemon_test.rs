use super::*;
use crate::domain::daemon::{ServingStatus, StartupFailureKind};
#[tokio::test]
async fn test_daemon操作_同じ集約を読み取り遷移させる() {
    // Given
    let repository = serving();
    let usecase = DaemonUsecase(repository);
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
