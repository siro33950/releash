use std::sync::Arc;

use super::ProviderLifecycleEventCodec;
use crate::adaptor::gateway::local_event_store::canonical_cbor::{
    decode_canonical, encode_canonical, CborValue,
};
use crate::adaptor::gateway::local_event_store::envelope::LocalEventPayloadCodec;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::local_event::{
    CommitBatchError, CommitBatchResult, CommitIdentity, CommitOperationKind, ExpectedStreamHead,
    IdempotencyBinding, LoadStreamRequest, LoadedDomainEvent, LocalAtomicBatch, LocalDomainEvent,
    LocalEventTransactionRepository, StreamId, StreamVersion, UncommittedDomainEvent,
};
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleBinding, ProviderLifecycleEvent, ProviderLifecycleScope,
    ProviderLifecycleUnavailableReason,
};
use tempfile::TempDir;

fn scope() -> ProviderLifecycleScope {
    ProviderLifecycleScope::new(
        "agent-session-1",
        "workflow-execution-1",
        "node-execution-1",
        1,
    )
    .unwrap()
}

#[test]
fn test_providerライフサイクルcodec_version付きcanonical_payloadを往復できる() {
    let events = [
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
        ProviderLifecycleEvent::LifecycleUnavailable {
            binding_id: "binding-1".to_string(),
            provider: ProviderKind::Codex,
            scope: scope(),
            reason: ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
        },
    ];
    let codec = ProviderLifecycleEventCodec;

    for event in events {
        let domain = LocalDomainEvent::ProviderLifecycle(event);
        let value = codec.encode(&domain).unwrap();
        let bytes = encode_canonical(&value).unwrap();
        let decoded_value = decode_canonical(&bytes).unwrap();
        let decoded = codec.decode(1, &decoded_value).unwrap();

        assert_eq!(decoded, Some(domain));
        assert_eq!(codec.decode(2, &decoded_value).unwrap(), None);
    }
}

#[test]
fn test_providerライフサイクルcodec_payloadにtranscript_bodyとcapability_secretを含めない() {
    let event = LocalDomainEvent::ProviderLifecycle(ProviderLifecycleEvent::SessionAssociated {
        binding_id: "binding-1".to_string(),
        provider_session_id: "provider-session-1".to_string(),
        transcript_ref: Some("provider://transcript/1".to_string()),
    });
    let CborValue::Text(raw) = ProviderLifecycleEventCodec.encode(&event).unwrap() else {
        panic!("provider lifecycle payload must be a canonical text document");
    };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), 4);
    for key in [
        "binding_id",
        "event",
        "provider_session_id",
        "transcript_ref",
    ] {
        assert!(object.contains_key(key));
    }
}

#[test]
fn test_providerライフサイクルcodec_空のdomain識別値をmalformedとして拒否する() {
    let codec = ProviderLifecycleEventCodec;
    let invalid_payloads = [
        serde_json::json!({
            "event": "binding_armed",
            "binding_id": "",
            "provider": "codex",
            "agent_session_id": "agent-session-1",
            "workflow_execution_id": "workflow-execution-1",
            "node_execution_id": "node-execution-1",
            "attempt": 1,
        }),
        serde_json::json!({
            "event": "session_associated",
            "binding_id": "binding-1",
            "provider_session_id": " ",
            "transcript_ref": null,
        }),
        serde_json::json!({
            "event": "transcript_associated",
            "binding_id": "binding-1",
            "transcript_ref": "",
        }),
        serde_json::json!({
            "event": "stop_failed",
            "binding_id": "binding-1",
            "reason": " ",
        }),
        serde_json::json!({
            "event": "binding_expired",
            "binding_id": "",
        }),
    ];

    for payload in invalid_payloads {
        let value = CborValue::Text(payload.to_string());
        assert!(
            codec.decode(1, &value).is_err(),
            "invalid stored event decoded as a Domain fact: {payload}"
        );
    }
}

#[tokio::test]
async fn test_providerライフサイクルcodec_eventをcommit再生しstale_stream_headを拒否する() {
    let directory = TempDir::new().unwrap();
    let clock = crate::adaptor::gateway::local_event_store::clock::FakeStoreClock::at(1_000);
    let fault = Arc::new(crate::adaptor::gateway::local_event_store::fault::FaultInjector::new());
    let installation_id = "11111111-1111-4111-8111-111111111596";
    fault.set_initial_installation_id(installation_id);
    let store = LocalEventStore::open(LocalEventStoreConfig {
        app_data_root: directory.path().to_path_buf(),
        clock: Arc::new(clock),
        registry: Arc::new(
            crate::adaptor::gateway::local_event_store::envelope::EventCodecRegistry::new(),
        ),
        fault,
        path_observer: Arc::new(
            crate::adaptor::gateway::local_event_store::layout::NoopStorePathObserver,
        ),
    })
    .unwrap();
    let stream_id = StreamId::agent_session("agent-session-1").unwrap();
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
            operation_kind: CommitOperationKind::SessionLifecycle,
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
    assert!(restored.is_stopped());
    assert_eq!(restored.provider_session_id(), Some("provider-session-1"));

    let stale = LocalAtomicBatch {
        commit_id: CommitIdentity::parse("provider-lifecycle-commit-2").unwrap(),
        idempotency: IdempotencyBinding {
            installation_id: installation_id.to_string(),
            operation_kind: CommitOperationKind::SessionLifecycle,
            idempotency_key: "provider-lifecycle-stale".to_string(),
            payload_hash: [16; 32],
        },
        expected_heads: vec![ExpectedStreamHead {
            stream_id,
            expected: StreamVersion::zero(),
        }],
        events: vec![UncommittedDomainEvent {
            stream_id: StreamId::agent_session("agent-session-1").unwrap(),
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
