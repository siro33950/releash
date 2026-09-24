use super::*;
use crate::domain::failure::FailureKind;

#[test]
fn test_所有済みセッション_ingressの両経路で分類を保持する() {
    // Given
    for (repository, usecase, expected) in [
        (
            AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned {
                agent_session_id: "owner".into(),
            },
            FailureKind::StateRequired,
        ),
        (
            AgentSessionRepositoryError::Conflict,
            AgentSessionUsecaseError::Conflict,
            FailureKind::RestartRequired,
        ),
        (
            AgentSessionRepositoryError::Unavailable,
            AgentSessionUsecaseError::Unavailable,
            FailureKind::Temporary,
        ),
    ] {
        // When / Then
        assert_eq!(
            map_session_repository_error(repository).failure_kind(),
            expected
        );
        assert_eq!(map_session_error(usecase).failure_kind(), expected);
    }
}
