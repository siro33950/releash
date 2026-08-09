use super::ProviderSessionOwnershipEventCodec;
use crate::adaptor::gateway::local_event_store::canonical_cbor::{
    decode_canonical, encode_canonical,
};
use crate::adaptor::gateway::local_event_store::envelope::LocalEventPayloadCodec;
use crate::domain::agent_session::services::ProviderSessionOwnershipEvent;
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::ProviderKind;

#[test]
fn test_provider_session_ownership_codec_claimとreleaseを往復できる() {
    let events = [
        ProviderSessionOwnershipEvent::Claimed {
            provider: ProviderKind::Claude,
            provider_session_id: "provider-session-1".to_string(),
            agent_session_id: "agent-session-1".to_string(),
        },
        ProviderSessionOwnershipEvent::Released {
            provider: ProviderKind::Claude,
            provider_session_id: "provider-session-1".to_string(),
            agent_session_id: "agent-session-1".to_string(),
        },
    ];
    let codec = ProviderSessionOwnershipEventCodec;

    for event in events {
        let domain = LocalDomainEvent::ProviderSessionOwnership(event);
        let value = codec.encode(&domain).unwrap();
        let bytes = encode_canonical(&value).unwrap();
        let decoded = codec.decode(1, &decode_canonical(&bytes).unwrap()).unwrap();

        assert_eq!(decoded, Some(domain));
    }
}
