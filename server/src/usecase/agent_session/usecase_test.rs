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
            crate::usecase::workspace_state::test_support::test_session_tabs(),
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
            .send_thread("session", vec!["two".into(), "one".into()], "releash-dev")
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
        crate::domain::agent_session::test_helpers::archive_derived(&mut session);
        let (usecase, input) = send_usecase(
            Arc::new(FailingSaveRepository::new(session)),
            vec![thread("one")],
        );
        // When
        let result = usecase
            .send_thread("session", vec!["one".into()], "releash")
            .await;
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
            .send_thread("session", vec!["one".into(), "outside".into()], "releash")
            .await;
        // Then
        assert!(matches!(result, Err(SendThreadUsecaseError::Thread(
            Error::NotFound(id)
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
            .send_thread("session", vec!["one".into(), "missing".into()], "releash")
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
        let result = usecase
            .send_thread("missing", vec!["one".into()], "releash")
            .await;
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
            crate::usecase::workspace_state::test_support::test_session_tabs(),
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
            crate::usecase::workspace_state::test_support::test_session_tabs(),
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

    fn tab_usecase(
        repository: Arc<FailingSaveRepository>,
        tabs: Arc<crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository>,
    ) -> AgentSessionUsecase {
        AgentSessionUsecase::new(
            repository,
            Arc::new(FakeThreadRepository::default()),
            AgentSessionService::new(
                Arc::new(RecordingTerminal::default()),
                Arc::new(RecordingTerminalInput::default()),
            ),
            crate::usecase::workspace_state::session_tabs::SessionTabsUsecase::new(
                tabs,
                crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
                crate::test_support::state_subscription::test_subscriptions(),
            ),
        )
    }

    #[tokio::test]
    async fn test_session保存_保存した事実だけ追従しタブ保存失敗でもsession保存は成功する() {
        use crate::domain::agent_session::repository::VersionedAgentSession;
        use crate::domain::workspace_state::{
            WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
        };
        use crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository;
        // Given
        for (save_success, tab_failure) in [(true, false), (false, false), (true, true)] {
            let repository = Arc::new(FailingSaveRepository::new(session()));
            repository
                .save_success
                .store(save_success, std::sync::atomic::Ordering::SeqCst);
            let tabs = Arc::new(MemoryWorkspaceStateRepository::default());
            let mut opened = WorkspaceState::default();
            opened.add_session_tab("session", "session", None);
            tabs.save(
                "/repo",
                opened.clone(),
                crate::domain::local_event::StreamVersion::zero(),
            )
            .await
            .unwrap();
            if tab_failure {
                *tabs.next_save_error.lock() = Some(WorkspaceStateError::InvalidLayout);
            }
            let usecase = tab_usecase(repository.clone(), tabs.clone());
            let mut changed = repository.snapshot().session().clone();
            changed
                .associate_provider_session("provider", None)
                .unwrap();
            changed.observe_provider_process_exit(Some(0));
            // When
            let result = usecase
                .save_if_changed(VersionedAgentSession::restored(changed, 1), "exit")
                .await;
            // Then
            if save_success {
                let saved = result.unwrap();
                assert_eq!(saved.revision(), 2);
                assert!(saved.session().uncommitted_events().is_empty());
                assert_eq!(saved, repository.snapshot());
            } else {
                assert_eq!(result.unwrap_err(), AgentSessionUsecaseError::Unavailable);
                assert_eq!(repository.snapshot().revision(), 1);
            }
            assert_eq!(
                tabs.read("/repo").await.unwrap().1,
                Some(if save_success && !tab_failure {
                    WorkspaceState::default()
                } else {
                    opened
                })
            );
        }
    }

    #[tokio::test]
    async fn test_session保存_未保存の事実がなければ保存せず開き直したタブを維持する() {
        use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};
        // Given
        let mut stopped = session();
        stopped
            .associate_provider_session("provider", None)
            .unwrap();
        stopped.observe_provider_process_exit(Some(0));
        let repository = Arc::new(FailingSaveRepository::new(stopped));
        let tabs = Arc::new(
            crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository::default(
            ),
        );
        let mut opened = WorkspaceState::default();
        opened.add_session_tab("session", "session", None);
        tabs.save(
            "/repo",
            opened.clone(),
            crate::domain::local_event::StreamVersion::zero(),
        )
        .await
        .unwrap();
        let usecase = tab_usecase(repository.clone(), tabs.clone());
        let original = repository.snapshot();
        // When
        let saved = usecase
            .save_if_changed(original.clone(), "unchanged")
            .await
            .unwrap();
        // Then
        assert_eq!(saved, original);
        assert_eq!(tabs.read("/repo").await.unwrap().1, Some(opened));
    }

    #[tokio::test]
    async fn test_session削除_archiveとgcは削除成功後に閉じタブ保存失敗は削除を失敗にしない() {
        use crate::domain::agent_session::aggregates::ManagedPtyPresence;
        use crate::domain::agent_session::repository::AgentSessionRepositoryError;
        use crate::domain::workspace_state::{
            WorkspaceState, WorkspaceStateError, WorkspaceStateRepository,
        };
        // Given
        for archive in [false, true] {
            for (remove_failure, tab_failure) in [(false, false), (true, false), (false, true)] {
                let mut target = session();
                if archive {
                    crate::domain::agent_session::test_helpers::archive_derived(&mut target);
                }
                let repository = Arc::new(FailingSaveRepository::new(target));
                if remove_failure {
                    *repository.remove_failure.lock().unwrap() =
                        Some(AgentSessionRepositoryError::Unavailable);
                }
                let tabs = Arc::new(crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository::default());
                let mut opened = WorkspaceState::default();
                opened.add_session_tab("session", "session", None);
                tabs.save(
                    "/repo",
                    opened.clone(),
                    crate::domain::local_event::StreamVersion::zero(),
                )
                .await
                .unwrap();
                if tab_failure {
                    *tabs.next_save_error.lock() = Some(WorkspaceStateError::InvalidLayout);
                }
                let usecase = tab_usecase(repository.clone(), tabs.clone());
                // When
                let result = if archive {
                    usecase.delete("session", "delete").await
                } else {
                    usecase
                        .garbage_collect("session", ManagedPtyPresence::ConfirmedAbsent, "gc")
                        .await
                };
                // Then
                assert_eq!(
                    result,
                    if remove_failure {
                        Err(AgentSessionUsecaseError::Unavailable)
                    } else {
                        Ok(())
                    }
                );
                assert_eq!(repository.stored.lock().unwrap().is_none(), !remove_failure);
                assert_eq!(
                    tabs.read("/repo").await.unwrap().1,
                    Some(if remove_failure || tab_failure {
                        opened
                    } else {
                        WorkspaceState::default()
                    })
                );
            }
        }
    }

    #[tokio::test]
    async fn test_session削除_未archiveと存在するprocessは拒否してタブも保持する() {
        use crate::domain::agent_session::aggregates::ManagedPtyPresence;
        use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};
        // Given
        let repository = Arc::new(FailingSaveRepository::new(session()));
        let tabs = Arc::new(
            crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository::default(
            ),
        );
        let mut opened = WorkspaceState::default();
        opened.add_session_tab("session", "session", None);
        tabs.save(
            "/repo",
            opened.clone(),
            crate::domain::local_event::StreamVersion::zero(),
        )
        .await
        .unwrap();
        let usecase = tab_usecase(repository.clone(), tabs.clone());
        // When / Then
        assert_eq!(
            usecase.delete("session", "delete").await,
            Err(AgentSessionUsecaseError::InvalidOperation)
        );
        assert_eq!(
            usecase
                .garbage_collect("session", ManagedPtyPresence::Live, "gc")
                .await,
            Err(AgentSessionUsecaseError::InvalidOperation)
        );
        assert!(repository.stored.lock().unwrap().is_some());
        assert_eq!(tabs.read("/repo").await.unwrap().1, Some(opened));
    }
    #[tokio::test]
    async fn test_session削除_会話開始済みならprocess不在でもgcを拒否してsessionとタブを保持する() {
        use crate::domain::agent_session::aggregates::ManagedPtyPresence;
        use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};
        // Given
        let mut started = session();
        started
            .associate_provider_session("provider", None)
            .unwrap();
        let repository = Arc::new(FailingSaveRepository::new(started));
        let original = repository.snapshot();
        let tabs = Arc::new(
            crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository::default(
            ),
        );
        let mut opened = WorkspaceState::default();
        opened.add_session_tab("session", "session", None);
        tabs.save(
            "/repo",
            opened,
            crate::domain::local_event::StreamVersion::zero(),
        )
        .await
        .unwrap();
        let original_tabs = tabs.read("/repo").await.unwrap();
        let usecase = tab_usecase(repository.clone(), tabs.clone());
        // When
        let result = usecase
            .garbage_collect("session", ManagedPtyPresence::ConfirmedAbsent, "gc")
            .await;
        // Then
        assert_eq!(result, Err(AgentSessionUsecaseError::InvalidOperation));
        assert_eq!(repository.snapshot(), original);
        assert_eq!(tabs.read("/repo").await.unwrap(), original_tabs);
    }

    #[tokio::test]
    async fn test_session再開_成功した事実だけ保存してタブを開く() {
        use crate::domain::agent_session::aggregates::AgentSessionRecoveryResult;
        use crate::domain::workspace_state::{WorkspaceState, WorkspaceStateRepository};
        // Given
        for result in [
            AgentSessionRecoveryResult::Succeeded,
            AgentSessionRecoveryResult::Failed,
        ] {
            let mut stopped = session();
            stopped
                .associate_provider_session("provider", None)
                .unwrap();
            stopped.observe_provider_process_exit(Some(0));
            let repository = Arc::new(FailingSaveRepository::new(stopped));
            repository
                .save_success
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let tabs = Arc::new(crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository::default());
            let usecase = tab_usecase(repository.clone(), tabs.clone());
            // When
            usecase
                .complete_resume("session", result, "resume")
                .await
                .unwrap();
            // Then
            let saved = tabs.read("/repo").await.unwrap().1;
            if result == AgentSessionRecoveryResult::Succeeded {
                let mut expected = WorkspaceState::default();
                expected.add_session_tab("session", "session", None);
                assert_eq!(saved, Some(expected));
                assert_eq!(repository.snapshot().revision(), 2);
            } else {
                assert_eq!(saved, None);
                assert_eq!(repository.snapshot().revision(), 1);
            }
        }
    }
}
