use super::*;
#[tokio::test]
async fn test_daemon保存_進行中commandを待たず停止を受理し後始末で待つ() {
    // Given
    let repository = serving();
    let admission = repository.admission().await;
    let usecase = crate::usecase::daemon::DaemonUsecase::new(repository.clone());
    // When
    let accepted = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        usecase.stop(StopRequest::Exit { code: 23 }),
    )
    .await
    .unwrap();
    let mut drain = Box::pin(repository.drain_commands());
    // Then
    assert_eq!(accepted, StopAcceptance::Started { code: 23 });
    assert!(!admission.admits(DaemonRequest::Operation));
    assert!(futures_util::poll!(drain.as_mut()).is_pending());
    assert_eq!(
        usecase.stop(StopRequest::Exit { code: 99 }).await,
        StopAcceptance::AlreadyAccepted
    );
    drop(admission);
    drain.await;
}

#[tokio::test]
async fn test_daemon停止完了_停止後の受付ガードが残っても期限後の遷移を妨げない() {
    // Given
    let repository = serving();
    repository.stop(StopRequest::Exit { code: 23 }).await;
    let admission = repository.admission().await;
    // When
    let repeated = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        repository.stop(StopRequest::Exit { code: 99 }),
    )
    .await
    .unwrap();
    assert_eq!(repeated, StopAcceptance::AlreadyAccepted);
    tokio::time::timeout(std::time::Duration::from_secs(1), repository.stopped())
        .await
        .unwrap();
    // Then
    assert_eq!(
        repository.info().await.serving_status,
        crate::domain::daemon::ServingStatus::Stopped
    );
    assert!(!admission.admits(DaemonRequest::Operation));
}
