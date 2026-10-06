use releash_lib::test_support::integration::fixtures::fixtures_adaptor_gateway_local_event_store_provider_lifecycle_codec_scope as scope;
use std::sync::Arc;

use releash_lib::test_support::integration::persistence::LocalEventStore;
use releash_lib::test_support::integration::persistence::LocalEventStoreConfig;
use releash_lib::test_support::integration::platform::CommitBatchError;
use releash_lib::test_support::integration::platform::CommitBatchResult;
use releash_lib::test_support::integration::platform::CommitIdentity;
use releash_lib::test_support::integration::platform::CommitOperationKind;
use releash_lib::test_support::integration::platform::ExpectedStreamHead;
use releash_lib::test_support::integration::platform::IdempotencyBinding;
use releash_lib::test_support::integration::platform::LoadStreamRequest;
use releash_lib::test_support::integration::platform::LoadedDomainEvent;
use releash_lib::test_support::integration::platform::LocalAtomicBatch;
use releash_lib::test_support::integration::platform::LocalDomainEvent;
use releash_lib::test_support::integration::platform::StreamId;
use releash_lib::test_support::integration::platform::StreamVersion;
use releash_lib::test_support::integration::platform::UncommittedDomainEvent;
use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::providers::ProviderLifecycleBinding;
use releash_lib::test_support::integration::providers::ProviderLifecycleEvent;
use releash_lib::test_support::integration::repository::LocalEventTransactionRepository;
use tempfile::TempDir;

#[tokio::test]
pub async fn test_providerライフサイクルcodec_eventをcommit再生しstale_stream_headを拒否する() {
    let directory = TempDir::new().unwrap();
    let clock = releash_lib::test_support::integration::persistence::FakeStoreClock::at(1_000);
    let fault = Arc::new(releash_lib::test_support::integration::persistence::FaultInjector::new());
    let installation_id = "11111111-1111-4111-8111-111111111596";
    fault.set_initial_installation_id(installation_id);
    let store = LocalEventStore::open(LocalEventStoreConfig {
        retry_limiter: std::sync::Arc::new(
            releash_lib::test_support::integration::platform::RetryLimiter::new(),
        ),
        app_data_root: directory.path().to_path_buf(),
        clock: Arc::new(clock),
        registry: Arc::new(
            releash_lib::test_support::integration::persistence::EventCodecRegistry::new(),
        ),
        fault,
        path_observer: Arc::new(
            releash_lib::test_support::integration::platform::NoopAppDataPathObserver,
        ),
    })
    .unwrap();
    let stream_id = StreamId::provider_lifecycle("agent-session-1").unwrap();
    let domain_events = vec![
        ProviderLifecycleEvent::BindingArmed {
            slot_id: "slot-1".to_string(),
            binding_id: "binding-1".to_string(),
            provider: ProviderKind::Codex,
            scope: scope(),
        },
        ProviderLifecycleEvent::SessionAssociated {
            binding_id: "binding-1".to_string(),
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: Some("provider://transcript/1".to_string()),
        },
        ProviderLifecycleEvent::StopObserved {
            binding_id: "binding-1".to_string(),
        },
    ];
    let events = domain_events
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, event)| UncommittedDomainEvent {
            stream_id: stream_id.clone(),
            event: LocalDomainEvent::ProviderLifecycle(event),
            occurred_at_ms: 1_000 + index as i64,
        })
        .collect::<Vec<_>>();
    let batch = LocalAtomicBatch {
        commit_id: CommitIdentity::parse("provider-lifecycle-commit-1").unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: installation_id.to_string(),
            operation_kind: CommitOperationKind::Projection,
            idempotency_key: "provider-lifecycle-binding-1-start-stop".to_string(),
            payload_hash: [15; 32],
        },
        expected_heads: vec![ExpectedStreamHead {
            stream_id: stream_id.clone(),
            expected: StreamVersion::zero(),
        }],
        events,
        state_mutations: Vec::new(),
    };

    assert!(matches!(
        store.commit_batch(batch.clone()).await.unwrap(),
        CommitBatchResult::Committed(_)
    ));
    assert!(matches!(
        store.commit_batch(batch).await.unwrap(),
        CommitBatchResult::Replayed(_)
    ));

    let page = store
        .load_stream(LoadStreamRequest {
            stream_id: stream_id.clone(),
            after: None,
            limit: 10,
        })
        .await
        .unwrap();
    let loaded = page
        .events
        .into_iter()
        .map(|event| match event.event {
            LoadedDomainEvent::Known(event) => match *event {
                LocalDomainEvent::ProviderLifecycle(event) => event,
                other => panic!("unexpected local event: {other:?}"),
            },
            other => panic!("provider lifecycle event must decode: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(loaded, domain_events);
    let restored = ProviderLifecycleBinding::rehydrate(loaded).unwrap();
    assert_eq!(restored.provider_session_id(), Some("provider-session-1"));

    let stale = LocalAtomicBatch {
        commit_id: CommitIdentity::parse("provider-lifecycle-commit-2").unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: installation_id.to_string(),
            operation_kind: CommitOperationKind::Projection,
            idempotency_key: "provider-lifecycle-stale".to_string(),
            payload_hash: [16; 32],
        },
        expected_heads: vec![ExpectedStreamHead {
            stream_id,
            expected: StreamVersion::zero(),
        }],
        events: vec![UncommittedDomainEvent {
            stream_id: StreamId::provider_lifecycle("agent-session-1").unwrap(),
            event: LocalDomainEvent::ProviderLifecycle(ProviderLifecycleEvent::BindingExpired {
                binding_id: "binding-1".to_string(),
            }),
            occurred_at_ms: 2_000,
        }],
        state_mutations: Vec::new(),
    };
    assert_eq!(
        store.commit_batch(stale).await.unwrap_err(),
        CommitBatchError::StreamHeadConflict {
            current: StreamVersion::new(3).unwrap(),
        }
    );
}
