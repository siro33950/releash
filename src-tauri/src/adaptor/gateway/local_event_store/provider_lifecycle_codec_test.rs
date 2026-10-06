use super::shared_test_helpers::*;
use super::*;
use crate::adaptor::gateway::local_event_store::canonical_cbor::{
    decode_canonical, encode_canonical, CborValue,
};
use crate::adaptor::gateway::local_event_store::envelope::LocalEventPayloadCodec;
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleUnavailableReason,
};

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
