use releashd::test_support::integration::persistence::{LocalEventStore, LocalEventStoreConfig};
use releashd::test_support::integration::platform::{
    scope, CommitIdentity, CommitOperationKind, Deadline, ExpectedStreamHead, IdempotencyBinding,
    LoadStreamRequest, LoadedDomainEvent, LocalAtomicBatch, LocalDomainEvent, OperationContext,
    RetryLimiter, StreamId, StreamVersion, UncommittedDomainEvent,
};
use releashd::test_support::integration::providers::ProviderLifecycleEvent;
use releashd::test_support::integration::repository::LocalEventTransactionRepository;
use releashd::test_support::integration::transport::ConnectFailure;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test]
async fn test_event_store_b001_b002_期限超過後も読み取りが成功し新しい記録を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        Arc::new(RetryLimiter::new()),
    ))
    .unwrap();
    // 他の3 workerを待機させ、期限を超えた接続を後続の公開APIで再利用する。
    let mut releases = Vec::new();
    let mut waiting = Vec::new();
    for _ in 0..3 {
        let store = store.clone();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let (started, ready) = tokio::sync::oneshot::channel();
        releases.push(release);
        waiting.push(tokio::spawn(async move {
            store
                .submit_query(move |_| {
                    started.send(()).unwrap();
                    let _ = wait.recv_timeout(Duration::from_secs(10));
                    Ok(())
                })
                .await
        }));
        ready.await.unwrap();
    }
    let context = OperationContext::default()
        .with_deadline(Deadline::new(Instant::now() + Duration::from_millis(250)));
    let (started, ready) = tokio::sync::oneshot::channel();
    let expired = scope(
        context,
        store.submit_query(move |connection| {
            let transaction = connection.unchecked_transaction().unwrap();
            transaction
                .query_row("SELECT COUNT(*) FROM events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        }),
    );
    tokio::pin!(expired);
    tokio::select! {
        result = &mut expired => panic!("read ended before opening its snapshot: {result:?}"),
        result = ready => result.unwrap(),
    }
    let error = expired.await.unwrap_err();
    assert_eq!(
        error.connect_code(),
        connectrpc::ErrorCode::DeadlineExceeded
    );
    let stream_id = StreamId::provider_lifecycle("timeout-recovery").unwrap();
    let event = LocalDomainEvent::ProviderLifecycle(ProviderLifecycleEvent::StopObserved {
        binding_id: "after-timeout".into(),
    });
    let commit_id = CommitIdentity::parse("after-timeout").unwrap();
    store
        .commit_batch(LocalAtomicBatch {
            commit_id: commit_id.clone(),
            idempotency: IdempotencyBinding {
                installation_id: store.installation_id().into(),
                operation_kind: CommitOperationKind::UserMutation,
                idempotency_key: "after-timeout".into(),
                payload_hash: [0; 32],
            },
            expected_heads: vec![ExpectedStreamHead {
                stream_id: stream_id.clone(),
                expected: StreamVersion::zero(),
            }],
            events: vec![UncommittedDomainEvent {
                stream_id: stream_id.clone(),
                event: event.clone(),
                occurred_at_ms: 1,
            }],
            state_mutations: vec![],
        })
        .await
        .unwrap();

    // When
    let mut results = Vec::new();
    for _ in 0..8 {
        results.push(
            store
                .load_stream(LoadStreamRequest {
                    stream_id: stream_id.clone(),
                    after: None,
                    limit: 10,
                })
                .await,
        );
    }
    drop(releases);
    for task in waiting {
        if let Err(error) = task.await.unwrap() {
            assert_eq!(
                error.connect_code(),
                connectrpc::ErrorCode::DeadlineExceeded
            );
        }
    }

    // Then: B-001 / B-002
    for result in results {
        let page = result.expect("every read after the timeout must succeed");
        assert!(
            page.events.iter().any(|record| {
                record.commit_id == commit_id
                    && record.event == LoadedDomainEvent::Known(Box::new(event.clone()))
            }),
            "every read must return the record appended after the timeout"
        );
    }
}
