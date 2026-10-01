use super::LocalProviderHookHealthFailureQuery;
use crate::domain::provider_lifecycle::{ProviderKind, ProviderLifecycleUnavailableReason};
use crate::usecase::provider_lifecycle::ProviderHookHealthFailureQuery;

#[tokio::test]
async fn test_hook警告読取_正常な記録と破損を一緒に返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (agent, launch, provider) in [
        ("agent-a", "launch-a", "claude"),
        ("agent-b", "launch-b", "codex"),
    ] {
        let marker = directory
            .path()
            .join("provider-launches")
            .join(agent)
            .join(launch)
            .join("hook-health.json");
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            &marker,
            provider,
            launch,
        )
        .unwrap();
    }
    let invalid = directory
        .path()
        .join("provider-launches/agent-c/launch-c/hook-health.json");
    std::fs::create_dir_all(invalid.parent().unwrap()).unwrap();
    std::fs::write(&invalid, br#"{"provider":"claude","secret":"ignored"}"#).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().to_path_buf());

    // When
    let records = query.list(3).await.unwrap();
    // Then
    assert!(matches!(
        records[2],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt)
    ));
}

#[tokio::test]
async fn test_hook警告読取_件数上限までの正常な記録を変換する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for (agent, launch, provider) in [
        ("agent-a", "launch-a", "claude"),
        ("agent-b", "launch-b", "codex"),
    ] {
        let marker = directory
            .path()
            .join("provider-launches")
            .join(agent)
            .join(launch)
            .join("hook-health.json");
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            &marker,
            provider,
            launch,
        )
        .unwrap();
    }
    let invalid = directory
        .path()
        .join("provider-launches/agent-c/launch-c/hook-health.json");
    std::fs::create_dir_all(invalid.parent().unwrap()).unwrap();
    std::fs::write(&invalid, br#"{"provider":"claude","secret":"ignored"}"#).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().to_path_buf());

    // When
    let observations: Vec<_> = query
        .list(2)
        .await
        .unwrap()
        .into_iter()
        .map(Result::unwrap)
        .collect();

    // Then
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].provider, ProviderKind::Claude);
    assert_eq!(observations[0].launch_id, "launch-a");
    assert_eq!(
        observations[0].reason,
        ProviderLifecycleUnavailableReason::LocalApiUnavailable
    );
    assert_eq!(observations[1].provider, ProviderKind::Codex);
    assert_eq!(observations[1].launch_id, "launch-b");
}

#[tokio::test]
async fn test_hook警告読取_置き場所が無ければ警告なしを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let records = query.list(10).await.unwrap();
    // Then
    assert!(records.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_記録情報を読めない場合も他の警告を返す() {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let healthy = directory
        .path()
        .join("provider-launches/a/healthy/hook-health.json");
    let path = directory
        .path()
        .join("provider-launches/b/invalid/hook-health.json");
    for marker in [&healthy, &path] {
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            marker,
            "claude",
            "launch",
        )
        .unwrap();
    }
    std::fs::set_permissions(path.parent().unwrap(), std::fs::Permissions::from_mode(0)).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;
    std::fs::set_permissions(
        path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    // Then
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].is_ok());
    assert_eq!(
        records[1],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Unavailable)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_通常ファイルではない場合も他の警告を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let healthy = directory
        .path()
        .join("provider-launches/a/healthy/hook-health.json");
    let path = directory
        .path()
        .join("provider-launches/b/invalid/hook-health.json");
    for marker in [&healthy, &path] {
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            marker,
            "claude",
            "launch",
        )
        .unwrap();
    }
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;

    // Then
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].is_ok());
    assert_eq!(
        records[1],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_サイズが大きすぎる場合も他の警告を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let healthy = directory
        .path()
        .join("provider-launches/a/healthy/hook-health.json");
    let path = directory
        .path()
        .join("provider-launches/b/invalid/hook-health.json");
    for marker in [&healthy, &path] {
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            marker,
            "claude",
            "launch",
        )
        .unwrap();
    }
    std::fs::write(&path, vec![b'x'; 4097]).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;

    // Then
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].is_ok());
    assert_eq!(
        records[1],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_中身を読めない場合も他の警告を返す() {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let healthy = directory
        .path()
        .join("provider-launches/a/healthy/hook-health.json");
    let path = directory
        .path()
        .join("provider-launches/b/invalid/hook-health.json");
    for marker in [&healthy, &path] {
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            marker,
            "claude",
            "launch",
        )
        .unwrap();
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0)).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    // Then
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].is_ok());
    assert_eq!(
        records[1],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Unavailable)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_解析できない場合も他の警告を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let healthy = directory
        .path()
        .join("provider-launches/a/healthy/hook-health.json");
    let path = directory
        .path()
        .join("provider-launches/b/invalid/hook-health.json");
    for marker in [&healthy, &path] {
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            marker,
            "claude",
            "launch",
        )
        .unwrap();
    }
    std::fs::write(&path, "broken").unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;

    // Then
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].is_ok());
    assert_eq!(
        records[1],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Corrupt)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_sessionを読めない場合も他の警告を返す() {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let healthy = directory
        .path()
        .join("provider-launches/a/healthy/hook-health.json");
    let path = directory
        .path()
        .join("provider-launches/b/invalid/hook-health.json");
    for marker in [&healthy, &path] {
        crate::infrastructure::provider_lifecycle::write_provider_hook_local_api_failure(
            directory.path(),
            marker,
            "claude",
            "launch",
        )
        .unwrap();
    }
    std::fs::set_permissions(
        path.parent().unwrap().parent().unwrap(),
        std::fs::Permissions::from_mode(0),
    )
    .unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;
    std::fs::set_permissions(
        path.parent().unwrap().parent().unwrap(),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    // Then
    let records = result.unwrap();
    assert_eq!(records.len(), 2);
    assert!(records[0].is_ok());
    assert_eq!(
        records[1],
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Unavailable)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_hook警告読取_置き場所を読めない場合は一覧全体の失敗を返す() {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("provider-launches");
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0)).unwrap();
    let query = LocalProviderHookHealthFailureQuery::new(directory.path().into());
    // When
    let result = query.list(10).await;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Then
    assert_eq!(
        result,
        Err(crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError::Unavailable)
    );
}
