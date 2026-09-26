#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionUsecaseError;

    // Given / When / Then
    for source in [
        AgentSessionUsecaseError::Conflict,
        AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
            agent_session_id: "owner".into(),
        },
    ] {
        let expected = source.clone().into();
        assert_eq!(
            super::map_session_error(source),
            super::AgentSessionInitialInstructionError::Conflict(expected)
        );
    }
}
