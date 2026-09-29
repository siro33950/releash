use super::*;
use crate::usecase::agent_session::provider_session_title_ingestion_tests::{
    session, FixedTitleGateway, RecordingNotifier, RecordingRepository,
};
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

fn usecase(
    repository: Arc<RecordingRepository>,
    gateway: Arc<FixedTitleGateway>,
    notifier: Arc<RecordingNotifier>,
) -> Arc<ProviderSessionTitleIngestionUsecase> {
    Arc::new(ProviderSessionTitleIngestionUsecase::new(
        repository,
        gateway,
        notifier.subscriptions.clone(),
    ))
}

#[tokio::test(start_paused = true)]
async fn test_providerタイトル入口_一覧のやり直さない失敗で周期を終える() {
    // Given
    let repository = Arc::new(RecordingRepository::new(vec![session(
        "queued",
        "provider-queued",
        None,
    )]));
    *repository.list_failures.lock().unwrap() = usize::MAX;
    let gateway = Arc::new(FixedTitleGateway::new([]));
    let notifier = Arc::new(RecordingNotifier::default());
    let retrying = crate::usecase::retry::test_retrying();
    // When
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        run(
            retrying.clone(),
            usecase(repository.clone(), gateway, notifier),
            Box::pin(
                crate::infrastructure::timer::ticks(std::time::Duration::from_secs(20)).take(3),
            ),
        ),
    )
    .await
    .unwrap_err();
    // Then
    assert!(repository.list_calls.load(Ordering::SeqCst) > 1);
    let records = retrying.records("daemon");
    assert_eq!(records.len(), 1);
    assert!(!records[0].requires_attention);
}

#[tokio::test(start_paused = true)]
async fn test_providerタイトル入口_やり直さない失敗の対象は次の周期で取り直さない() {
    // Given
    let repository = Arc::new(RecordingRepository::new(vec![
        session("broken", "provider-broken", None),
        session("fine", "provider-fine", None),
    ]));
    let gateway = Arc::new(FixedTitleGateway::new([
        (
            "provider-broken",
            Err(crate::domain::agent_session::ProviderSessionTitleGatewayError::Corrupt),
        ),
        ("provider-fine", Ok(None)),
    ]));
    let notifier = Arc::new(RecordingNotifier::default());
    let retrying = crate::usecase::retry::test_retrying();
    // When
    run(
        retrying.clone(),
        usecase(repository.clone(), gateway.clone(), notifier),
        Box::pin(crate::infrastructure::timer::ticks(std::time::Duration::from_secs(20)).take(3)),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // Then
    assert_eq!(repository.list_calls.load(Ordering::SeqCst), 3);
    assert_eq!(gateway.read_count("provider-broken"), 1);
    assert_eq!(gateway.read_count("provider-fine"), 3);
    let records = retrying.records("broken");
    assert_eq!(records.len(), 1);
    assert!(records[0].requires_attention);
}
