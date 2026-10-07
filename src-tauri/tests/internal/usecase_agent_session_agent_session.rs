use releash_lib::test_support::integration::platform::install_capturing_logger;
use releash_lib::test_support::integration::sessions::AgentSessionLifecycleUsecase;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use releash_lib::test_support::integration::providers::ProviderKind;
use releash_lib::test_support::integration::providers::ProviderLifecycleUsecase;
use releash_lib::test_support::integration::sessions::AgentSessionArchiveOutcome;
use releash_lib::test_support::integration::sessions::AgentSessionHistoryMetadata;
use releash_lib::test_support::integration::sessions::AgentSessionHistoryResumeOutcome;
use releash_lib::test_support::integration::sessions::AgentSessionHistoryResumeRequest;
use releash_lib::test_support::integration::sessions::AgentSessionLaunchRequest;
use releash_lib::test_support::integration::sessions::AgentSessionLaunchUsecase;
use releash_lib::test_support::integration::sessions::AgentSessionLaunchUsecaseError;
use releash_lib::test_support::integration::sessions::AgentSessionLifecycle;
use releash_lib::test_support::integration::sessions::AgentSessionProcessExitOutcome;
use releash_lib::test_support::integration::sessions::AgentSessionRecoveryResult;
use releash_lib::test_support::integration::sessions::AgentSessionUsecase;
use releash_lib::test_support::integration::sessions::AgentSessionUsecaseError;
use releash_lib::test_support::integration::sessions::ExecutionTreeCacheReleaseError;
use releash_lib::test_support::integration::sessions::ManagedPtyPresence;
use releash_lib::test_support::integration::sessions::ProviderAgentTerminalGatewayError;
use releash_lib::test_support::integration::sessions::ProviderSessionLaunch;
use releash_lib::test_support::integration::sessions::StartedExecutionTreeRegistrationError;
use releash_lib::test_support::integration::sessions::WorkflowAgentSessionLaunchRequest;
use releash_lib::test_support::integration::workspace::WorkspaceIdentity;

use releash_lib::test_support::integration::sessions::{
    captured_terminal_spawn_failure, hook_health_usecase, provider_runtime, session_location,
    started_execution_trees, workflow_location, BlockingLaunchTerminal,
    FailingFirstLifecycleEvents, FixedAvailability, FixedHistory, RecordingLaunchGateway,
    RecordingLifecycleEvents, RecordingStartedExecutionTrees, RecordingTerminal,
};

#[tokio::test]
pub async fn test_agent_session_history_resume_実行木登録失敗ではcreateをrollbackする() {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(store),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let execution_trees = Arc::new(RecordingStartedExecutionTrees {
        failure: Some(StartedExecutionTreeRegistrationError::Unavailable),
        ..RecordingStartedExecutionTrees::default()
    });
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions.clone(),
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-registration-failure".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
hook_health_usecase(),
execution_trees.clone(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);

    let error = usecase
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            provider_session_id: "provider-registration-failure".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-registration-failure".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, AgentSessionLaunchUsecaseError::StorageUnavailable);
    let expected_id = execution_trees.tree_ids.lock().unwrap()[0].clone();
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(launch_gateway.launches.lock().unwrap().is_empty());
    assert!(launch_gateway.cleanups.lock().unwrap().is_empty());
    assert!(terminal.spawns.lock().unwrap().is_empty());
    assert_eq!(*terminal.deletes.lock().unwrap(), 0);
}

#[tokio::test]
pub async fn test_agent_session_history_resume_同一要求の再送は既存sessionへ収束する() {
    // Given: provider history に再開対象が存在する
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions,
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events,
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            terminal.clone(),
        ),
Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Claude,
                provider_session_id: "provider-history-idempotent".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
hook_health_usecase(),
started_execution_trees(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::SqliteWorkspaceTreeRepository::new(store.clone()),);
    let request = AgentSessionHistoryResumeRequest {
        workspace: WorkspaceIdentity::new("/repo"),
        worktree_path: "/repo/worktree".to_string(),
        provider: ProviderKind::Claude,
        provider_session_id: "provider-history-idempotent".to_string(),
        rows: 24,
        cols: 80,
        caller_request_id: "history-resume-idempotent".to_string(),
    };

    // When: 同じ履歴 resume 要求を二度送る
    let first = usecase.resume_history(request.clone()).await.unwrap();
    let second = usecase.resume_history(request.clone()).await.unwrap();

    // Then: どちらも caller request から導出した同じ既存 Session を返す
    let AgentSessionHistoryResumeOutcome::Open(first) = first else {
        panic!("first history resume must keep the session open");
    };
    let AgentSessionHistoryResumeOutcome::Open(second) = second else {
        panic!("repeated history resume must keep the session open");
    };
    let expected_id = releash_lib::test_support::integration::sessions::launch_resource_id(
        "agent-session",
        &request.caller_request_id,
    )
    .unwrap();
    assert_eq!(first.session().id(), expected_id);
    assert_eq!(second.session().id(), expected_id);
    assert_eq!(
        second.session().provider_session_id(),
        Some(request.provider_session_id.as_str())
    );
    assert_eq!(terminal.spawns.lock().unwrap().len(), 2);
    let selected = Arc::new(usecase)
        .resume_history_selection(request)
        .await
        .unwrap();
    assert_eq!(selected.0, expected_id);
    assert_eq!(selected.1.id, expected_id);
}

#[tokio::test]
pub async fn test_agent_session_history_resumeは新しいsessionを作り失敗時もidを保持する() {
    install_capturing_logger();
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    let availability = Arc::new(FixedAvailability {
        available: true,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.spawn_error.lock().unwrap() = Some(ProviderAgentTerminalGatewayError::Technical(
        releash_lib::test_support::integration::platform::TechnicalFailure {
            nature: releash_lib::test_support::integration::platform::TechnicalFailureNature::Other,
            message: "checkpoint restore failed".to_string(),
        },
    ));
    let hook_health = hook_health_usecase();
    let execution_trees = started_execution_trees();
    let usecase = AgentSessionLaunchUsecase::new(
        std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
        sessions.clone(),
        lifecycle,
        provider_runtime(availability, launch_gateway.clone(), terminal),
        Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-history-1".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health,
        execution_trees.clone(),
        tokio::sync::mpsc::unbounded_channel().0,
        releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),
    );

    let outcome = usecase
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            provider_session_id: "provider-history-1".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-resume-1".to_string(),
        })
        .await
        .unwrap();

    let expected_id = match outcome {
        AgentSessionHistoryResumeOutcome::Paused(session) => session.session().id().to_string(),
        AgentSessionHistoryResumeOutcome::Open(_) => panic!("expected paused session"),
    };
    let saved = sessions.find(&expected_id).await.unwrap().unwrap();
    assert_eq!(saved.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert_eq!(
        saved.session().provider_session_id(),
        Some("provider-history-1")
    );
    assert_eq!(
        launch_gateway.launches.lock().unwrap().as_slice(),
        &[ProviderSessionLaunch::resume("provider-history-1").unwrap()]
    );
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.tree_ids.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    let record = captured_terminal_spawn_failure(&expected_id).unwrap();
    assert!(record.ends_with("checkpoint restore failed"));
}

#[tokio::test]
pub async fn test_agent_session_history_resume_lifecycle準備失敗でもpausedへ収束する() {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(FailingFirstLifecycleEvents::default()),
    ));
    let usecase = AgentSessionLaunchUsecase::new(
        std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
        sessions.clone(),
        lifecycle,
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            Arc::new(RecordingTerminal::default()),
        ),
        Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-history-arm-failure".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health_usecase(),
        started_execution_trees(),
        tokio::sync::mpsc::unbounded_channel().0,
        releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),
    );

    let outcome = usecase
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            provider_session_id: "provider-history-arm-failure".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-arm-failure".to_string(),
        })
        .await
        .unwrap();

    let expected_id = match outcome {
        AgentSessionHistoryResumeOutcome::Paused(session) => session.session().id().to_string(),
        AgentSessionHistoryResumeOutcome::Open(_) => panic!("expected paused session"),
    };
    let saved = sessions.find(&expected_id).await.unwrap().unwrap();
    assert_eq!(saved.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert_eq!(
        saved.session().provider_session_id(),
        Some("provider-history-arm-failure")
    );
}

fn durable_usecase(
    directory: &tempfile::TempDir,
) -> (
    Arc<releash_lib::test_support::integration::persistence::LocalEventStore>,
    AgentSessionUsecase,
) {
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let repository = Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    );
    (store, AgentSessionUsecase::new(repository))
}

#[tokio::test]
pub async fn test_agent_session_usecase_process_exit_resume_archive_deleteを永続化する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-session-1"),
            "create-1",
        )
        .await
        .unwrap();
    usecase
        .associate_provider_session("agent-session-1", "claude-session-1", None, "associate-1")
        .await
        .unwrap();

    assert_eq!(
        usecase
            .observe_process_exit("agent-session-1", Some(0), "exit-1")
            .await
            .unwrap(),
        AgentSessionProcessExitOutcome::Paused
    );
    assert_eq!(
        usecase
            .find("agent-session-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    usecase
        .complete_resume(
            "agent-session-1",
            AgentSessionRecoveryResult::Failed,
            "resume-failed-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase
            .find("agent-session-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    usecase
        .complete_resume(
            "agent-session-1",
            AgentSessionRecoveryResult::Succeeded,
            "resume-success-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase
            .archive("agent-session-1", "archive-1")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    usecase.delete("agent-session-1", "delete-1").await.unwrap();
    assert!(usecase.find("agent-session-1").await.unwrap().is_none());
}

#[tokio::test]
pub async fn test_agent_session_usecase_id不明archiveも確認なしで記録する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-unknown",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-session-unknown"),
            "create-unknown",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .archive("agent-session-unknown", "archive-unknown")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    assert!(usecase
        .find("agent-session-unknown")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
pub async fn test_agent_session_usecase_gcはpty不在確定時だけunknown_idを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-gc",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-session-gc"),
            "create-gc",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .garbage_collect(
                "agent-session-gc",
                ManagedPtyPresence::Unknown,
                "gc-unknown"
            )
            .await
            .unwrap_err(),
        AgentSessionUsecaseError::InvalidOperation
    );
    usecase
        .garbage_collect(
            "agent-session-gc",
            ManagedPtyPresence::ConfirmedAbsent,
            "gc-confirmed",
        )
        .await
        .unwrap();
    assert!(usecase.find("agent-session-gc").await.unwrap().is_none());
}

#[tokio::test]
pub async fn test_provider_agent_workflow_session_launch_workflow関連付け後に初回指示付きprovider_tuiを起動する(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions,
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
Arc::new(FixedHistory { entries: Vec::new() }),
hook_health_usecase(),
started_execution_trees(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);

    let launched = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-1".to_string(),
            node_execution_id: "node-1".to_string(),
            initial_instruction: "Implement the workflow node.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-launch-1".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(
        launched.session().tree_location(),
        &workflow_location("workflow-1", "node-1")
    );
    assert_eq!(
        launch_gateway.launches.lock().unwrap().as_slice(),
        &[
            ProviderSessionLaunch::new_with_initial_instruction("Implement the workflow node.")
                .unwrap()
        ]
    );
    assert!(launched.session().initial_instruction_admitted());
    assert!(terminal.spawns.lock().unwrap().is_empty());

    usecase
        .activate_workflow_node(launched.session().id())
        .await
        .unwrap();
    {
        let spawns = terminal.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 1);
        assert_eq!((spawns[0].rows, spawns[0].cols), (24, 80));
    }
    usecase
        .confirm_workflow_node_attachment(launched.session().id())
        .await
        .unwrap();
    assert_eq!(
        usecase
            .confirm_workflow_node_attachment(launched.session().id())
            .await,
        Err(AgentSessionLaunchUsecaseError::InvalidInput)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
pub async fn test_provider_agent_workflow_session_launch_別sessionのactivateを起動待ちで直列化しない(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(store),
    )));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let terminal = Arc::new(BlockingLaunchTerminal {
        presence: Mutex::new(ManagedPtyPresence::ConfirmedAbsent),
        spawn_entered: Mutex::new(Some(entered_sender)),
        spawn_release: Mutex::new(Some(release_receiver)),
        spawns: AtomicUsize::new(0),
        deletes: Mutex::new(0),
    });
    let usecase = Arc::new(AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions,
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            terminal.clone(),
        ),
Arc::new(FixedHistory { entries: Vec::new() }),
hook_health_usecase(),
started_execution_trees(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),));
    let first = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree-first".to_string(),
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-parallel".to_string(),
            node_execution_id: "node-first".to_string(),
            initial_instruction: "Implement first.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-parallel-first".to_string(),
        })
        .await
        .unwrap();
    let second = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree-second".to_string(),
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-parallel".to_string(),
            node_execution_id: "node-second".to_string(),
            initial_instruction: "Implement second.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-parallel-second".to_string(),
        })
        .await
        .unwrap();

    let first_session_id = first.session().id().to_string();
    let first_usecase = Arc::clone(&usecase);
    let first_activation = tokio::spawn(async move {
        first_usecase
            .activate_workflow_node(&first_session_id)
            .await
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let second_session_id = second.session().id().to_string();
    let second_usecase = Arc::clone(&usecase);
    let mut second_activation = tokio::spawn(async move {
        second_usecase
            .activate_workflow_node(&second_session_id)
            .await
    });
    let second_result = tokio::time::timeout(Duration::from_secs(5), &mut second_activation).await;
    release_sender.send(()).unwrap();
    first_activation.await.unwrap().unwrap();

    match second_result {
        Ok(result) => {
            result.unwrap().unwrap();
        }
        Err(_) => {
            second_activation.await.unwrap().unwrap();
            panic!("unrelated workflow activation waited for another terminal spawn");
        }
    }
    assert_eq!(terminal.spawns.load(Ordering::SeqCst), 2);
}

#[tokio::test]
pub async fn test_provider_agent_workflow_session_launch_activate後のrollbackで起動資源を解放する()
{
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(store),
    )));
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions,
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events.clone(),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
Arc::new(FixedHistory { entries: Vec::new() }),
hook_health_usecase(),
started_execution_trees(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);
    let launched = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-rollback".to_string(),
            node_execution_id: "node-rollback".to_string(),
            initial_instruction: "Implement rollback.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-launch-rollback".to_string(),
        })
        .await
        .unwrap();
    usecase
        .activate_workflow_node(launched.session().id())
        .await
        .unwrap();

    usecase
        .rollback_workflow_node(launched.session().id(), "rollback-request")
        .await
        .unwrap();

    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        &[launched.session().id().to_string()]
    );
    assert!(lifecycle_events.events.lock().unwrap().iter().any(|event| {
        matches!(
            event.clone().into_parts().1,
            releash_lib::test_support::integration::providers::ProviderLifecycleEvent::BindingExpired { .. }
        )
    }));
}

#[tokio::test]
pub async fn test_agent_session_launch_spawn失敗時はsessionとlaunch資源をrollbackする() {
    install_capturing_logger();
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.spawn_error.lock().unwrap() = Some(ProviderAgentTerminalGatewayError::Technical(
        releash_lib::test_support::integration::platform::TechnicalFailure {
            nature: releash_lib::test_support::integration::platform::TechnicalFailureNature::Other,
            message: "openpty failed".to_string(),
        },
    ));
    let hook_health = hook_health_usecase();
    let execution_trees = started_execution_trees();
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions.clone(),
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal,
        ),
Arc::new(FixedHistory { entries: Vec::new() }),
hook_health,
execution_trees.clone(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "failed-launch-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::Terminal(ProviderAgentTerminalGatewayError::Technical(
            releash_lib::test_support::integration::platform::TechnicalFailure {
                nature:
                    releash_lib::test_support::integration::platform::TechnicalFailureNature::Other,
                message: "openpty failed".to_string()
            }
        ))
    );
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    let record = captured_terminal_spawn_failure(&expected_id).unwrap();
    assert!(record.ends_with("openpty failed"));
}

#[tokio::test]
pub async fn test_agent_session_launch_prepare失敗時のrollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    *launch_gateway.fail_prepare.lock().unwrap() = true;
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.fail_delete.lock().unwrap() = true;
    let execution_trees = started_execution_trees();
    *execution_trees.release_failure.lock().unwrap() =
        Some(ExecutionTreeCacheReleaseError::Unavailable);
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions.clone(),
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
hook_health_usecase(),
execution_trees.clone(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "rollback-delete-failure-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::Launch(
            releash_lib::test_support::integration::sessions::ProviderAgentLaunchGatewayError::Technical(
                releash_lib::test_support::integration::platform::TechnicalFailure {
                    nature: releash_lib::test_support::integration::platform::TechnicalFailureNature::Transient,
                    message: "unavailable".into()
                }
            )
        )
    );
    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
}

#[tokio::test]
pub async fn test_agent_session_launch_spawn失敗時のrollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.spawn_error.lock().unwrap() = Some(ProviderAgentTerminalGatewayError::Technical(
        releash_lib::test_support::integration::platform::TechnicalFailure {
            nature: releash_lib::test_support::integration::platform::TechnicalFailureNature::Other,
            message: "openpty failed".to_string(),
        },
    ));
    *terminal.fail_delete.lock().unwrap() = true;
    let execution_trees = started_execution_trees();
    *execution_trees.release_failure.lock().unwrap() =
        Some(ExecutionTreeCacheReleaseError::Corrupt);
    let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions.clone(),
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
hook_health_usecase(),
execution_trees.clone(),
tokio::sync::mpsc::unbounded_channel().0, releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),);

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "rollback-spawn-delete-failure-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::Terminal(ProviderAgentTerminalGatewayError::Technical(
            releash_lib::test_support::integration::platform::TechnicalFailure {
                nature:
                    releash_lib::test_support::integration::platform::TechnicalFailureNature::Other,
                message: "openpty failed".to_string()
            }
        ))
    );
    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
}

#[tokio::test]
pub async fn test_session選択_欠落はcorruptとしqueryの技術的性質を保持する() {
    use releash_lib::test_support::integration::platform::LocalEventQueryError;
    use releash_lib::test_support::integration::platform::TechnicalFailure;
    use releash_lib::test_support::integration::platform::TechnicalFailureNature;
    use releash_lib::test_support::integration::workspace::WorkspaceTreeNode;
    use releash_lib::test_support::integration::workspace::WorkspaceTreeRepository;
    struct Trees(Option<LocalEventQueryError>);
    #[async_trait::async_trait]
    impl WorkspaceTreeRepository for Trees {
        async fn load_trees(
            &self,
            _: &[WorkspaceIdentity],
        ) -> Vec<
            Result<
                releash_lib::test_support::integration::workspace::WorkspaceTree,
                releash_lib::test_support::integration::workflow::WorkflowError,
            >,
        > {
            panic!("unexpected tree read")
        }
        async fn load_node(
            &self,
            _: &WorkspaceIdentity,
            _: &str,
        ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
            panic!("unexpected node read")
        }
        async fn load_node_by_session_id(
            &self,
            workspace: &WorkspaceIdentity,
            id: &str,
        ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
            assert_eq!(workspace.as_str(), "/repo");
            assert!(!id.is_empty());
            match &self.0 {
                Some(error) => Err(error.clone()),
                None => Ok(None),
            }
        }
    }
    let mut cases = vec![
        (None, AgentSessionLaunchUsecaseError::Corrupt),
        (
            Some(LocalEventQueryError::QueryBusy),
            AgentSessionLaunchUsecaseError::Store(LocalEventQueryError::QueryBusy.into()),
        ),
    ];
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "query failed".into(),
        };
        cases.push((
            Some(LocalEventQueryError::Technical(failure.clone())),
            AgentSessionLaunchUsecaseError::Technical(failure),
        ));
    }
    for (error, expected) in cases {
        // Given: provider history に再開対象が存在する
        let directory = tempfile::tempdir().unwrap();
        let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
            releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
                std::sync::Arc::new(
                    releash_lib::test_support::integration::platform::RetryLimiter::new(),
                ),
            ),
        )
        .unwrap();
        let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
            releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
                store.clone(),
            ),
        )));
        let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let usecase = AgentSessionLaunchUsecase::new(
std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
sessions,
Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events,
        )),
provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            terminal.clone(),
        ),
Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Claude,
                provider_session_id: "provider-history-idempotent".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
hook_health_usecase(),
started_execution_trees(),
tokio::sync::mpsc::unbounded_channel().0, Arc::new(Trees(error.clone())),);
        let request = AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Claude,
            provider_session_id: "provider-history-idempotent".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-resume-idempotent".to_string(),
        };

        let usecase = Arc::new(usecase);
        assert_eq!(
            usecase
                .clone()
                .launch_standalone_selection(AgentSessionLaunchRequest {
                    workspace: WorkspaceIdentity::new("/repo"),
                    worktree_path: "/repo/worktree".into(),
                    provider: ProviderKind::Claude,
                    rows: 24,
                    cols: 80,
                    caller_request_id: "selection-create".into(),
                })
                .await
                .unwrap_err(),
            expected
        );
        assert_eq!(
            usecase.resume_history_selection(request).await.unwrap_err(),
            expected
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
pub async fn test_agent_session_launch_pty起動中のsessionをgcしない() {
    let directory = tempfile::tempdir().unwrap();
    let store = releash_lib::test_support::integration::persistence::LocalEventStore::open(
        releash_lib::test_support::integration::persistence::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
            std::sync::Arc::new(
                releash_lib::test_support::integration::platform::RetryLimiter::new(),
            ),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        releash_lib::test_support::integration::sessions::LocalAgentSessionRepository::new(
            store.clone(),
        ),
    )));
    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            releash_lib::test_support::integration::providers::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let terminal = Arc::new(BlockingLaunchTerminal {
        presence: Mutex::new(ManagedPtyPresence::ConfirmedAbsent),
        spawn_entered: Mutex::new(Some(entered_sender)),
        spawn_release: Mutex::new(Some(release_receiver)),
        spawns: AtomicUsize::new(0),
        deletes: Mutex::new(0),
    });
    let launches = Arc::new(RecordingLaunchGateway::default());
    let hook_health = hook_health_usecase();
    let launch = Arc::new(AgentSessionLaunchUsecase::new(
        std::sync::Arc::new(releash_lib::test_support::integration::telemetry::TelemetryGateway),
        sessions.clone(),
        provider_lifecycle.clone(),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launches.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health.clone(),
        started_execution_trees(),
        tokio::sync::mpsc::unbounded_channel().0,
        releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),
    ));
    let lifecycle = Arc::new(AgentSessionLifecycleUsecase::new(
        std::sync::Arc::new(releash_lib::test_support::integration::platform::RandomIdentityIssuer),
        sessions.clone(),
        provider_lifecycle,
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launches.clone(),
            terminal.clone(),
        ),
        hook_health,
        releash_lib::test_support::integration::subscriptions::test_subscriptions(),
        started_execution_trees(),
        releash_lib::test_support::integration::workspace::TestWorkspaceTreeRepository::new(),
    ));

    let launching = tokio::spawn(async move {
        launch
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Claude,
                rows: 24,
                cols: 80,
                caller_request_id: "launch-gc".to_string(),
            })
            .await
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let expected_id = launches.armed.lock().unwrap()[0]
        .scope()
        .agent_session_id()
        .to_string();
    let gc_agent_session_id = expected_id.clone();
    let collecting = tokio::spawn(async move {
        lifecycle
            .reconcile_garbage_collection(&gc_agent_session_id, "gc-during-launch")
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    launching.await.unwrap().unwrap();
    assert_eq!(
        collecting.await.unwrap().unwrap(),
        releash_lib::test_support::integration::sessions::AgentSessionGarbageCollectionOutcome::Retained
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_some());
    assert_eq!(*terminal.deletes.lock().unwrap(), 0);
}
