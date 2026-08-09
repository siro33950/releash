use super::{ProviderSessionAlreadyOwned, ProviderSessionOwnership};
use crate::domain::provider_lifecycle::ProviderKind;

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
