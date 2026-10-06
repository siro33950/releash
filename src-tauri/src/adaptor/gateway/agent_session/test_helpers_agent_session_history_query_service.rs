use crate::domain::agent_session::AgentSessionHistoryMetadata;
use crate::domain::provider_lifecycle::ProviderKind;
pub fn metadata(
    provider: ProviderKind,
    provider_session_id: &str,
    updated_at_ms: i64,
) -> AgentSessionHistoryMetadata {
    AgentSessionHistoryMetadata {
        provider,
        provider_session_id: provider_session_id.to_string(),
        worktree_path: "/repo/worktree".to_string(),
        updated_at_ms,
    }
}
