use std::sync::Arc;

use tempfile::tempdir;

use super::LocalProviderHookHealthRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::{
    ProviderHookHealthOutcome, ProviderHookHealthRepository, ProviderKind,
    ProviderLifecycleUnavailableReason,
};

#[tokio::test]
async fn test_provider_hook_health_repository_warningと解除を再起動後も復元する() {
    let directory = tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = LocalProviderHookHealthRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let mut health = repository.load(ProviderKind::Codex).await.unwrap();
    assert_eq!(health.revision(), 0);
    assert_eq!(health.health().warning(), None);

    health.health_mut().observe_launch("launch-1");
    assert!(matches!(
        health.health_mut().observe_unavailable(
            "launch-1",
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
        ),
        ProviderHookHealthOutcome::Applied(_)
    ));
    repository.save(health, "warning-request-1").await.unwrap();

    drop(repository);
    drop(store);
    let restarted = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = LocalProviderHookHealthRepository::new(
        restarted.clone() as Arc<dyn LocalEventTransactionRepository>,
        restarted.installation_id().to_string(),
    );
    let mut restored = repository.load(ProviderKind::Codex).await.unwrap();
    assert_eq!(restored.revision(), 2);
    assert_eq!(
        restored.health().warning(),
        Some((
            "launch-1",
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed
        ))
    );

    restored.health_mut().observe_launch("launch-2");
    restored.health_mut().observe_session_started("launch-2");
    repository.save(restored, "clear-request-1").await.unwrap();
    let cleared = repository.load(ProviderKind::Codex).await.unwrap();
    assert_eq!(cleared.revision(), 4);
    assert_eq!(cleared.health().warning(), None);
}

#[tokio::test]
async fn test_provider_hook_health_repository_providerごとの状態を混同しない() {
    let directory = tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = LocalProviderHookHealthRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    );
    let mut claude = repository.load(ProviderKind::Claude).await.unwrap();
    claude.health_mut().observe_launch("launch-claude");
    claude.health_mut().observe_unavailable(
        "launch-claude",
        ProviderLifecycleUnavailableReason::LocalApiUnavailable,
    );
    repository.save(claude, "warning-claude").await.unwrap();

    assert_eq!(
        repository
            .load(ProviderKind::Claude)
            .await
            .unwrap()
            .health()
            .warning(),
        Some((
            "launch-claude",
            ProviderLifecycleUnavailableReason::LocalApiUnavailable
        ))
    );
    assert_eq!(
        repository
            .load(ProviderKind::Codex)
            .await
            .unwrap()
            .health()
            .warning(),
        None
    );
}

#[tokio::test]
async fn test_hook保存_同一キーの異なる内容と古いrevisionの分類を区別する() {
    use crate::adaptor::presenter::connect::ConnectFailure;
    use connectrpc::ErrorCode;
    // Given
    let directory = tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let repository =
        LocalProviderHookHealthRepository::new(store.clone(), store.installation_id().into());
    let mut first = repository.load(ProviderKind::Codex).await.unwrap();
    let mut stale = repository.load(ProviderKind::Codex).await.unwrap();
    first.health_mut().observe_launch("first");
    repository.save(first, "same-request").await.unwrap();
    let mut updated = repository.load(ProviderKind::Codex).await.unwrap();
    updated.health_mut().observe_launch("second");
    stale.health_mut().observe_launch("stale");
    // When
    let payload = repository.save(updated, "same-request").await.unwrap_err();
    let head = repository.save(stale, "new-request").await.unwrap_err();
    // Then
    assert_eq!(payload.connect_code(), ErrorCode::FailedPrecondition);
    assert_eq!(head.connect_code(), ErrorCode::Aborted);
}
