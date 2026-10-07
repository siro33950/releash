use super::*;
use crate::domain::failure::{StorageFailure, StorageFailureSource, TechnicalFailureNature};

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
            ProviderLifecycleIngressUsecaseError::Store(StorageFailure {
                nature: TechnicalFailureNature::Other,
                source: StorageFailureSource::AgentSession(Box::new(
                    AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                        agent_session_id: "owner".into(),
                    },
                )),
                context: None,
            }),
        ),
        (
            AgentSessionRepositoryError::Conflict,
            AgentSessionUsecaseError::Conflict,
            ProviderLifecycleIngressUsecaseError::Conflict,
        ),
        (
            AgentSessionRepositoryError::Unavailable,
            AgentSessionUsecaseError::Unavailable,
            ProviderLifecycleIngressUsecaseError::StorageUnavailable,
        ),
    ] {
        // When / Then
        assert_eq!(map_session_repository_error(repository), expected);
        assert_eq!(map_session_error(usecase), expected);
    }
}
