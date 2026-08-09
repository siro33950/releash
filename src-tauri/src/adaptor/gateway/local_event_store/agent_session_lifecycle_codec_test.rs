use super::AgentSessionLifecycleEventCodec;
use crate::adaptor::gateway::local_event_store::canonical_cbor::{
    decode_canonical, encode_canonical,
};
use crate::adaptor::gateway::local_event_store::envelope::LocalEventPayloadCodec;
use crate::domain::agent_session::aggregates::{
    AgentSessionLifecycle, AgentSessionLifecycleEvent, AgentSessionOrigin,
};
use crate::domain::local_event::LocalDomainEvent;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;

#[test]
fn test_agent_session_lifecycle_codec_version付きcanonical_payloadを往復できる() {
    let events = [
        AgentSessionLifecycleEvent::Created {
            id: "agent-session-1".to_string(),
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/.worktrees/feature".to_string(),
            provider: ProviderKind::Codex,
            origin: AgentSessionOrigin::Standalone,
        },
        AgentSessionLifecycleEvent::ProviderSessionAssociated {
            provider_session_id: "provider-session-1".to_string(),
            transcript_ref: Some("provider://transcript/1".to_string()),
        },
        AgentSessionLifecycleEvent::LifecycleChanged {
            lifecycle: AgentSessionLifecycle::Paused,
            last_exit_abnormal: true,
        },
        AgentSessionLifecycleEvent::InitialInstructionAdmitted,
    ];
    let codec = AgentSessionLifecycleEventCodec;

    for event in events {
        let domain = LocalDomainEvent::AgentSessionLifecycle(event);
        let value = codec.encode(&domain).unwrap();
        let bytes = encode_canonical(&value).unwrap();
        let decoded_value = decode_canonical(&bytes).unwrap();

        assert_eq!(codec.decode(1, &decoded_value).unwrap(), Some(domain));
        assert_eq!(codec.decode(2, &decoded_value).unwrap(), None);
    }
}

#[test]
fn test_agent_session_lifecycle_codec_last_exit_abnormalなしの旧eventをfalseとして復元する() {
    let codec = AgentSessionLifecycleEventCodec;
    let legacy = crate::adaptor::gateway::local_event_store::canonical_cbor::CborValue::Text(
        r#"{"event":"lifecycle_changed","lifecycle":"paused"}"#.to_string(),
    );

    assert_eq!(
        codec.decode(1, &legacy).unwrap(),
        Some(LocalDomainEvent::AgentSessionLifecycle(
            AgentSessionLifecycleEvent::LifecycleChanged {
                lifecycle: AgentSessionLifecycle::Paused,
                last_exit_abnormal: false,
            }
        ))
    );
}
