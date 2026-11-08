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

#[cfg(unix)]
#[test]
fn test_起動ロック_親の別名と保存先作成前後で同じ排他になる() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&parent, &alias).unwrap();
    let first = super::acquire_startup_lock(&alias.join("data")).unwrap();
    // When
    std::fs::create_dir(parent.join("data")).unwrap();
    let second = super::acquire_startup_lock(&parent.join("data"));
    // Then
    assert!(matches!(
        second,
        Err(crate::domain::daemon::StartupFailureKind::StoreInUse)
    ));
    drop(first);
    assert!(super::acquire_startup_lock(&parent.join("data")).is_ok());
}

#[test]
fn test_起動ロック_相対パスの現在と親を正規化して排他する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    for relative in [".", "child/.."] {
        let path = root.path().join(relative);
        let first = super::acquire_startup_lock(&path).unwrap();
        // When
        let second = super::acquire_startup_lock(&path.canonicalize().unwrap());
        // Then
        assert!(matches!(
            second,
            Err(crate::domain::daemon::StartupFailureKind::StoreInUse)
        ));
        drop(first);
    }
}
