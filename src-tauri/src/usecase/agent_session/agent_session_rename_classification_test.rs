#[test]
fn test_session所有済みと保存競合を区別して伝播する() {
    use super::AgentSessionRepositoryError;
    use crate::domain::failure::{ClassifiedFailure, FailureKind};
    // Given / When / Then
    for (source, expected) in [
        (
            AgentSessionRepositoryError::Conflict,
            FailureKind::RestartRequired,
        ),
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            FailureKind::StateRequired,
        ),
    ] {
        assert_eq!(super::map_repository_error(source).failure_kind(), expected);
    }
}
