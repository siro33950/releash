#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionUsecaseError;
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given / When / Then
    for (source, expected) in [
        (
            AgentSessionUsecaseError::Conflict,
            FailureKind::RestartRequired,
        ),
        (
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            FailureKind::StateRequired,
        ),
    ] {
        assert_eq!(super::map_session_error(source).failure_kind(), expected);
    }
}
