mod memory_tests {
    use crate::usecase::agent_session::test_helpers::*;
    use std::sync::Arc;

    use crate::domain::agent_session::aggregates::agent_session::AgentSession;
    use crate::domain::provider_lifecycle::value_objects::provider_kind::ProviderKind;
    use crate::domain::workspace_tree::value_objects::WorkspaceIdentity;
    use crate::usecase::agent_session::usecase::AgentSessionUsecase;
    use crate::usecase::agent_session::usecase::AgentSessionUsecaseError;

    #[tokio::test]
    pub async fn test_agent_session_usecase選択されたproviderでstandalone_sessionを作成する() {
        let seed = AgentSession::create(
            "seed",
            WorkspaceIdentity::new("/seed"),
            "/seed",
            ProviderKind::Claude,
            session_location("seed"),
        )
        .unwrap();
        let repository = Arc::new(FailingSaveRepository::new(seed));
        *repository.stored.lock().unwrap() = None;
        let usecase = AgentSessionUsecase::new(repository);

        let created = usecase
            .create(
                "agent-session-1",
                WorkspaceIdentity::new("/repo"),
                "/repo/.worktrees/feature",
                ProviderKind::Codex,
                session_location("agent-session-1"),
                "create-request-1",
            )
            .await
            .unwrap();

        assert_eq!(created.session().provider(), ProviderKind::Codex);
        assert_eq!(
            created.session().tree_location(),
            &session_location("agent-session-1")
        );
        assert_eq!(created.revision(), 1);
    }

    #[tokio::test]
    pub async fn test_agent_session_usecase永続化失敗で共有状態を進めない() {
        let session = AgentSession::create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/.worktrees/feature",
            ProviderKind::Codex,
            session_location("agent-session-1"),
        )
        .unwrap();
        let repository = Arc::new(FailingSaveRepository::new(session));
        let usecase = AgentSessionUsecase::new(repository.clone());

        let result = usecase
            .associate_provider_session(
                "agent-session-1",
                "provider-session-1",
                None,
                "associate-request-1",
            )
            .await;

        assert_eq!(result.unwrap_err(), AgentSessionUsecaseError::Unavailable);
        let unchanged = repository.snapshot();
        assert_eq!(unchanged.revision(), 1);
        assert_eq!(unchanged.session().provider_session_id(), None);
    }
}
