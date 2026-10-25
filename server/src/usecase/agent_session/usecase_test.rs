mod memory_tests {
    use crate::domain::agent_session::services::AgentSessionService;

    use crate::usecase::agent_session::test_helpers::*;
    use std::sync::Arc;

    use crate::domain::agent_session::aggregates::agent_session::AgentSession;
    use crate::domain::agent_session::aggregates::AgentSessionInstructionError;
    use crate::domain::agent_session::services::AgentSessionSendThreadError;
    use crate::domain::comment::{Author, Comment, Error, State, Target, Thread};
    use crate::domain::provider_lifecycle::value_objects::provider_kind::ProviderKind;
    use crate::domain::workspace_tree::value_objects::WorkspaceIdentity;
    use crate::usecase::agent_session::usecase::AgentSessionUsecase;
    use crate::usecase::agent_session::usecase::AgentSessionUsecaseError;
    use crate::usecase::agent_session::usecase::SendThreadUsecaseError;

    fn session() -> AgentSession {
        AgentSession::create(
            "session",
            WorkspaceIdentity::new("/repo"),
            "/repo/.worktrees/feature",
            ProviderKind::Codex,
            session_location("session"),
        )
        .unwrap()
    }

    fn thread(id: &str) -> Thread {
        Thread {
            id: id.into(),
            worktree_name: "/repo".into(),
            target: Target {
                file_path: None,
                line_number: None,
                end_line: None,
                side: None,
                end_side: None,
            },
            state: State::Open,
            comments: vec![Comment {
                id: format!("comment-{id}"),
                thread_id: id.into(),
                author: Author::human(),
                content: format!("本文-{id}"),
                created_at: 1.0,
            }],
            resolve: None,
            created_at: 1.0,
            updated_at: 1.0,
            version: 1,
            can_resolve: true,
        }
    }

    fn send_usecase(
        repository: Arc<FailingSaveRepository>,
        threads: Vec<Thread>,
    ) -> (AgentSessionUsecase, Arc<RecordingTerminalInput>) {
        let input = Arc::new(RecordingTerminalInput::default());
        let usecase = AgentSessionUsecase::new(
            repository,
            Arc::new(FakeThreadRepository(std::sync::Mutex::new(threads))),
            AgentSessionService::new(Arc::new(RecordingTerminal::default()), input.clone()),
        );
        (usecase, input)
    }

    #[tokio::test]
    async fn test_thread送信_openのsessionへ全threadのidを含む指示を一回書く() {
        // Given
        let session = session();
        let owner = session.terminal_surface_owner();
        let (usecase, input) = send_usecase(
            Arc::new(FailingSaveRepository::new(session)),
            vec![thread("one"), thread("two")],
        );
        // When
        usecase
            .send_thread("session", vec!["two".into(), "one".into()])
            .await
            .unwrap();
        // Then
        let writes = input.0.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, owner);
        let prompt = &writes[0].1;
        assert!(prompt.contains("Thread ID: one"));
        assert!(prompt.contains("Thread ID: two"));
        assert!(prompt.find("Thread ID: two").unwrap() < prompt.find("Thread ID: one").unwrap());
    }

    #[tokio::test]
    async fn test_thread送信_openでないsessionは拒否し何も書かない() {
        // Given
        let mut session = session();
        session.archive().unwrap();
        let (usecase, input) = send_usecase(
            Arc::new(FailingSaveRepository::new(session)),
            vec![thread("one")],
        );
        // When
        let result = usecase.send_thread("session", vec!["one".into()]).await;
        // Then
        assert!(matches!(
            result,
            Err(SendThreadUsecaseError::Send(
                AgentSessionSendThreadError::Instruction(AgentSessionInstructionError::NotOpen)
            ))
        ));
        assert!(input.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_thread送信_別workspaceのthreadを含む場合は全件書かない() {
        // Given
        let mut outside = thread("outside");
        outside.worktree_name = "/other".into();
        let (usecase, input) = send_usecase(
            Arc::new(FailingSaveRepository::new(session())),
            vec![thread("one"), outside],
        );
        // When
        let result = usecase
            .send_thread("session", vec!["one".into(), "outside".into()])
            .await;
        // Then
        assert!(matches!(result, Err(SendThreadUsecaseError::Send(
            AgentSessionSendThreadError::ThreadOutsideWorkspace(id)
        )) if id == "outside"));
        assert!(input.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_thread送信_無いthreadはnotfoundで何も書かない() {
        // Given
        let (usecase, input) = send_usecase(
            Arc::new(FailingSaveRepository::new(session())),
            vec![thread("one")],
        );
        // When
        let result = usecase
            .send_thread("session", vec!["one".into(), "missing".into()])
            .await;
        // Then
        assert!(matches!(
            result,
            Err(SendThreadUsecaseError::Thread(Error::NotFound(_)))
        ));
        assert!(input.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_thread送信_無いsessionはsessionnotfoundで何も書かない() {
        // Given
        let repository = Arc::new(FailingSaveRepository::new(session()));
        *repository.stored.lock().unwrap() = None;
        let (usecase, input) = send_usecase(repository, vec![thread("one")]);
        // When
        let result = usecase.send_thread("missing", vec!["one".into()]).await;
        // Then
        assert!(matches!(
            result,
            Err(SendThreadUsecaseError::SessionNotFound)
        ));
        assert!(input.0.lock().unwrap().is_empty());
    }

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
        let usecase = AgentSessionUsecase::new(
            repository,
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
        );

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
        let usecase = AgentSessionUsecase::new(
            repository.clone(),
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
        );

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
