use super::ProviderLifecycleScope;

#[test]
fn test_providerライフサイクルscope_agent_sessionだけを起動相関として保持する() {
    let scope = ProviderLifecycleScope::new("agent-session-1").unwrap();

    assert_eq!(scope.agent_session_id(), "agent-session-1");
}

#[test]
fn test_providerライフサイクルscope_空のagent_sessionを拒否する() {
    assert!(ProviderLifecycleScope::new(" ").is_err());
}
