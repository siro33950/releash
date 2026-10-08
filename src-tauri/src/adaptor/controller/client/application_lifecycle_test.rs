use super::stop_daemon_shared;

#[tokio::test]
async fn test_サーバ停止_コード0を一度だけ通知し重複を受理する() {
    // Given
    let daemon =
        crate::usecase::daemon::DaemonUsecase::new(crate::adaptor::gateway::daemon::serving());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    // When
    stop_daemon_shared(&daemon, &sender).await.unwrap();
    // Then
    assert_eq!(receiver.try_recv().unwrap(), 0);
    receiver.close();
    stop_daemon_shared(&daemon, &sender).await.unwrap();
    assert_eq!(receiver.len(), 0);
}

#[tokio::test]
async fn test_サーバ停止_受信先がなければ失敗を返す() {
    // Given
    let daemon =
        crate::usecase::daemon::DaemonUsecase::new(crate::adaptor::gateway::daemon::serving());
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    drop(receiver);
    // When / Then
    assert!(stop_daemon_shared(&daemon, &sender).await.is_err());
}

#[tokio::test]
async fn test_サーバ停止_command同期中も停止を受理する() {
    // Given
    let repository = crate::adaptor::gateway::daemon::serving();
    let admission = repository.admission().await;
    let daemon = crate::usecase::daemon::DaemonUsecase::new(repository.clone());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    // When
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        stop_daemon_shared(&daemon, &sender),
    )
    .await
    .unwrap()
    .unwrap();
    // Then
    assert_eq!(receiver.try_recv().unwrap(), 0);
    assert!(!admission.admits(crate::domain::daemon::DaemonRequest::Operation));
}
