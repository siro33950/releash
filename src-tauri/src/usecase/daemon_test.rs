use super::*;
use crate::domain::daemon::{ServingStatus, StartupFailureKind};
#[tokio::test]
async fn test_daemon操作_同じ集約を読み取り遷移させる() {
    // Given
    let repository = crate::adaptor::gateway::daemon::serving();
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
