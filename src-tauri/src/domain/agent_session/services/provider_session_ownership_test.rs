use super::{
    ProviderSessionAlreadyOwned, ProviderSessionOwnership, ProviderSessionOwnershipClaimOutcome,
    ProviderSessionOwnershipEvent, ProviderSessionOwnershipReleaseOutcome,
};
use crate::domain::provider_lifecycle::ProviderKind;

#[test]
fn test_provider_session所有_release後は別sessionがclaimできる() {
    let mut ownership =
        ProviderSessionOwnership::new(ProviderKind::Claude, "provider-session-1").unwrap();
    ownership.claim("agent-session-1").unwrap();
    ownership.take_uncommitted_events();

    let outcome = ownership.release("agent-session-1").unwrap();

    assert_eq!(outcome, ProviderSessionOwnershipReleaseOutcome::Released);
    assert_eq!(
        ownership.take_uncommitted_events(),
        vec![ProviderSessionOwnershipEvent::Released {
            provider: ProviderKind::Claude,
            provider_session_id: "provider-session-1".to_string(),
            agent_session_id: "agent-session-1".to_string(),
        }]
    );
    assert_eq!(
        ownership.claim("agent-session-2").unwrap(),
        ProviderSessionOwnershipClaimOutcome::Claimed
    );
}

#[test]
fn test_provider_session所有_snapshotから未commit_eventなしで復元する() {
    let mut ownership = ProviderSessionOwnership::restore(
        ProviderKind::Codex,
        "provider-session-1",
        Some("agent-session-1"),
    )
    .unwrap();

    let error = ownership.claim("agent-session-2").unwrap_err();

    assert_eq!(
        error,
        ProviderSessionAlreadyOwned {
            agent_session_id: "agent-session-1".to_string(),
        }
    );
    assert!(ownership.take_uncommitted_events().is_empty());
}
