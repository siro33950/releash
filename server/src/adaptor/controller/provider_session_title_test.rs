use super::*;
use crate::usecase::agent_session::provider_session_title_ingestion::provider_session_title_ingestion_tests::{
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
    let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
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
    assert!(store.records("*").is_empty());
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
    let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
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
    assert!(store.records("*").is_empty());
}

use crate::domain::agent_session::repository::AgentSessionRepositoryError;
#[tokio::test(start_paused = true)]
async fn test_provider_title入口_開始の一時失敗と対象の分類別再試行を行う() {
    use std::sync::atomic::Ordering;
    for failure in [
        AgentSessionRepositoryError::Unavailable,
        AgentSessionRepositoryError::Conflict,
    ] {
        let repository = Arc::new(RecordingRepository::new(vec![session(
            "queued",
            "provider-queued",
            None,
        )]));
        *repository.list_failures.lock().unwrap() = 2;
        *repository.save_failures.lock().unwrap() = vec![failure.clone(), failure.clone()];
        let gateway = Arc::new(FixedTitleGateway::new([(
            "provider-queued",
            Ok(Some("title")),
        )]));
        let notifier = Arc::new(RecordingNotifier::default());
        let usecase = Arc::new(ProviderSessionTitleIngestionUsecase::new(
            repository.clone(),
            gateway.clone(),
            notifier.subscriptions.clone(),
        ));
        let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
        let _run = tokio::spawn(crate::adaptor::controller::provider_session_title::run(
            retrying.clone(),
            usecase,
            Box::pin(futures_util::stream::iter([()])),
        ));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while notifier.worktrees.lock().unwrap().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(repository.list_calls.load(Ordering::SeqCst), 3);
        let expected_reads = if failure == AgentSessionRepositoryError::Conflict {
            3
        } else {
            1
        };
        assert_eq!(repository.find_calls.load(Ordering::SeqCst), expected_reads);
        assert_eq!(gateway.read_count("provider-queued"), expected_reads);
        assert_eq!(repository.saved_titles.lock().unwrap().len(), 1);
        let records = store.records("queued");
        assert!(records.is_empty());
    }
}
