#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionRepositoryError;

    // Given / When / Then
    for (source, expected) in [
        (
            AgentSessionRepositoryError::Conflict,
            super::AgentSessionRenameError::Conflict,
        ),
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            super::AgentSessionRenameError::ProviderSessionAlreadyOwned,
        ),
    ] {
        assert_eq!(super::map_repository_error(source), expected);
    }
}
