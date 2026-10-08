use super::{
    wait_for_activation, StandaloneLaunchRequestRegistry, COMPLETED_STANDALONE_LAUNCH_CAPACITY,
};

#[test]
fn test_冪等レジストリ_完了記録が容量を超えると最古のrequest_idを追い出す() {
    let mut registry = StandaloneLaunchRequestRegistry::default();
    for index in 0..=COMPLETED_STANDALONE_LAUNCH_CAPACITY {
        registry.record_completed(format!("request-{index}"), Ok(format!("agent-{index}")));
    }

    assert_eq!(registry.recall_completed("request-0"), None);
    assert_eq!(
        registry.recall_completed("request-1"),
        Some(Ok("agent-1".to_string()))
    );
    assert_eq!(
        registry.recall_completed(&format!("request-{COMPLETED_STANDALONE_LAUNCH_CAPACITY}")),
        Some(Ok(format!("agent-{COMPLETED_STANDALONE_LAUNCH_CAPACITY}")))
    );
}

#[test]
fn test_冪等レジストリ_recall済みrequest_idは追い出し順が更新され残存する() {
    let mut registry = StandaloneLaunchRequestRegistry::default();
    for index in 0..COMPLETED_STANDALONE_LAUNCH_CAPACITY {
        registry.record_completed(format!("request-{index}"), Ok(format!("agent-{index}")));
    }
    assert!(registry.recall_completed("request-0").is_some());

    registry.record_completed("request-new".to_string(), Ok("agent-new".to_string()));

    assert_eq!(
        registry.recall_completed("request-0"),
        Some(Ok("agent-0".to_string()))
    );
    assert_eq!(registry.recall_completed("request-1"), None);
    assert_eq!(
        registry.recall_completed("request-new"),
        Some(Ok("agent-new".to_string()))
    );
}

#[tokio::test]
async fn test_workflow_activation待機_sender消失を完了として扱わない() {
    let (completion_tx, completion_rx) = tokio::sync::watch::channel(false);
    drop(completion_tx);

    assert!(!wait_for_activation(completion_rx).await);
}

#[tokio::test]
async fn test_workflow_activation待機_true通知を完了として扱う() {
    let (completion_tx, completion_rx) = tokio::sync::watch::channel(false);
    completion_tx.send(true).unwrap();

    assert!(wait_for_activation(completion_rx).await);
}

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
        let expected = match &source {
            AgentSessionUsecaseError::Conflict => {
                crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict
                    .into()
            }
            AgentSessionUsecaseError::ProviderSessionAlreadyOwned { agent_session_id } => {
                crate::domain::agent_session::repository::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
                    agent_session_id: agent_session_id.clone(),
                }
                .into()
            }
            _ => unreachable!(),
        };
        assert_eq!(
            super::map_session_error(source),
            super::AgentSessionLaunchUsecaseError::Conflict(expected)
        );
    }
}

#[test]
fn test_実行木登録の失敗_起動エラーへ変換しても元の分類を保持する() {
    // Given / When / Then
    for source in [
        crate::domain::local_event::LocalEventQueryError::Internal {
            correlation_id: "id".into(),
        },
        crate::domain::local_event::LocalEventQueryError::QueryBusy,
        crate::domain::local_event::LocalEventQueryError::Technical(
            crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "timeout".into(),
            },
        ),
        crate::domain::local_event::LocalEventQueryError::IncompatibleStoredEvent {
            correlation_id: "id".into(),
        },
    ] {
        let expected = source.clone().into();
        let error = super::map_execution_tree_registration_error(
            super::StartedExecutionTreeRegistrationError::Store(source.into()),
        );
        assert_eq!(
            error,
            super::AgentSessionLaunchUsecaseError::Store(expected)
        );
    }
}

#[test]
fn test_session失敗_全変種から技術的な失敗だけを参照する() {
    use super::AgentSessionLaunchUsecaseError as E;
    use crate::domain::agent_session::{
        ProviderAgentLaunchGatewayError as L, ProviderAgentTerminalGatewayError as T,
    };
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given
    let storage = crate::domain::failure::StorageFailure::from(
        crate::domain::local_event::CommitBatchError::QueueBusy,
    );
    // When / Then
    for error in [
        E::ProviderUnavailable,
        E::InvalidInput,
        E::StorageUnavailable,
        E::Corrupt,
        E::Store(storage.clone()),
        E::Conflict(storage.clone()),
        E::Launch(L::InvalidInput),
        E::Terminal(T::NotFound("missing".into())),
        E::Terminal(T::InvalidOperation("invalid".into())),
        E::Terminal(T::StaleAttachment),
        E::Terminal(T::OwnerConflict),
    ] {
        assert_eq!(error.technical_failure(), None);
    }
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "source failure".into(),
        };
        for error in [
            E::Launch(L::Technical(failure.clone())),
            E::Terminal(T::Technical(failure.clone())),
            E::Technical(failure.clone()),
        ] {
            assert_eq!(error.technical_failure(), Some(&failure));
        }
    }
}

#[tokio::test]
async fn test_workflow起動保持_期限が来てもactivating記録を消さない() {
    // Given
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let launches = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::from([
        (
            "session".to_string(),
            super::WorkflowLaunchActivation::Activating(receiver),
        ),
    ])));
    let request = super::LaunchRetention {
        launches: launches.clone(),
        session: "session".to_string(),
    };
    // When
    request.expire().await;
    // Then
    assert!(matches!(
        launches.lock().await.get("session"),
        Some(super::WorkflowLaunchActivation::Activating(_))
    ));
}

mod memory_tests {
    use crate::usecase::agent_session::test_helpers::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::domain::agent_session::aggregates::agent_session::AgentSession;
    use crate::domain::agent_session::aggregates::agent_session::ManagedPtyPresence;
    use crate::domain::agent_session::repository::AgentSessionRepositoryError;
    use crate::domain::provider_lifecycle::value_objects::provider_kind::ProviderKind;
    use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_unavailable::ProviderLifecycleUnavailableReason;
    use crate::domain::terminal_surface::value_objects::terminal_surface_owner::TerminalSurfaceOwner;
    use crate::domain::workspace_tree::value_objects::WorkspaceIdentity;
    use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchRequest;
    use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchUsecase;
    use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchUsecaseError;
    use crate::usecase::agent_session::agent_session_launch::StartedExecutionTreeRegistrationError;
    use crate::usecase::agent_session::usecase::AgentSessionUsecase;
    use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthUsecase;
    use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;

    #[tokio::test]
    pub async fn test_agent_session_launch_利用可能な選択providerをterminal_root_processへ接続する()
    {
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
        let availability = Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        });
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let execution_trees = started_execution_trees();
        let usecase = launch_usecase_with_tree_registrar(
            repository,
            availability.clone(),
            launch_gateway.clone(),
            terminal.clone(),
            hook_health_usecase(),
            execution_trees.clone(),
        );

        let launched = usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/.worktrees/feature".to_string(),
                provider: ProviderKind::Codex,
                rows: 30,
                cols: 120,
                caller_request_id: "launch-request-1".to_string(),
            })
            .await
            .unwrap();
        let expected_id = launched.session().id();

        assert_eq!(launched.session().provider(), ProviderKind::Codex);
        assert_eq!(
            availability.checks.lock().unwrap().as_slice(),
            &[ProviderKind::Codex]
        );
        let armed = launch_gateway.armed.lock().unwrap();
        assert_eq!(armed.len(), 1);
        assert_eq!(armed[0].scope().agent_session_id(), expected_id);
        let spawns = terminal.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 1);
        assert_eq!(
            spawns[0].owner,
            TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), expected_id).unwrap()
        );
        assert_eq!(spawns[0].worktree_path, "/repo/.worktrees/feature");
        assert_eq!(spawns[0].process.executable(), "/opt/bin/provider");
        assert_eq!((spawns[0].rows, spawns[0].cols), (30, 120));
        assert_eq!(
            execution_trees.tree_ids.lock().unwrap().as_slice(),
            &[expected_id.to_string()]
        );
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_create_commit失敗では実行木を登録しない() {
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
        *repository.atomic_create_failure.lock().unwrap() =
            Some(AgentSessionRepositoryError::Unavailable);
        let execution_trees = started_execution_trees();
        let usecase = launch_usecase_with_tree_registrar(
            repository,
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            Arc::new(RecordingTerminal::default()),
            hook_health_usecase(),
            execution_trees.clone(),
        );

        let error = usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Codex,
                rows: 24,
                cols: 80,
                caller_request_id: "create-commit-failure".to_string(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, AgentSessionLaunchUsecaseError::StorageUnavailable);
        assert!(execution_trees.tree_ids.lock().unwrap().is_empty());
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_実行木登録失敗ではcreateと起動資源をrollbackする() {
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
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
        let execution_trees = Arc::new(RecordingStartedExecutionTrees {
            failure: Some(StartedExecutionTreeRegistrationError::Unavailable),
            ..RecordingStartedExecutionTrees::default()
        });
        let usecase = AgentSessionLaunchUsecase::new(
            std::sync::Arc::new(crate::usecase::test_helpers::NoopPerformance),
            Arc::new(AgentSessionUsecase::new(repository.clone())),
            Arc::new(ProviderLifecycleUsecase::new(
                Arc::new(crate::usecase::test_helpers::TestCredentials),
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
            Arc::new(FixedHistory {
                entries: Vec::new(),
            }),
            hook_health_usecase(),
            execution_trees.clone(),
            tokio::sync::mpsc::unbounded_channel().0,
            crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        );

        let error = usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Codex,
                rows: 24,
                cols: 80,
                caller_request_id: "registration-failure".to_string(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, AgentSessionLaunchUsecaseError::StorageUnavailable);
        assert!(repository.stored.lock().unwrap().is_none());
        assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
        let expected_id = execution_trees.tree_ids.lock().unwrap()[0].clone();
        assert_eq!(
            execution_trees.releases.lock().unwrap().as_slice(),
            std::slice::from_ref(&expected_id)
        );
        assert_eq!(
            launch_gateway.cleanups.lock().unwrap().as_slice(),
            std::slice::from_ref(&expected_id)
        );
        assert_eq!(*terminal.deletes.lock().unwrap(), 1);
        assert!(launch_gateway.launches.lock().unwrap().is_empty());
        assert!(terminal.spawns.lock().unwrap().is_empty());
        assert!(lifecycle_events.events.lock().unwrap().iter().any(|event| {
        matches!(
            event.clone().into_parts().1,
            crate::domain::provider_lifecycle::value_objects::provider_lifecycle_event::ProviderLifecycleEvent::BindingExpired { .. }
        )
    }));
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_session作成とlifecycle_armを一回のrepository操作で永続化する(
    ) {
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
        let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
        let usecase = AgentSessionLaunchUsecase::new(
            std::sync::Arc::new(crate::usecase::test_helpers::NoopPerformance),
            Arc::new(AgentSessionUsecase::new(repository.clone())),
            Arc::new(ProviderLifecycleUsecase::new(
                Arc::new(crate::usecase::test_helpers::TestCredentials),
                lifecycle_events.clone(),
            )),
            provider_runtime(
                Arc::new(FixedAvailability {
                    available: true,
                    checks: Mutex::new(Vec::new()),
                }),
                Arc::new(RecordingLaunchGateway::default()),
                Arc::new(RecordingTerminal::default()),
            ),
            Arc::new(FixedHistory {
                entries: Vec::new(),
            }),
            hook_health_usecase(),
            started_execution_trees(),
            tokio::sync::mpsc::unbounded_channel().0,
            crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        );

        usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Claude,
                rows: 24,
                cols: 80,
                caller_request_id: "atomic-launch-request".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(repository.create_calls.load(Ordering::SeqCst), 0);
        assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
        assert_eq!(repository.launch_lifecycle_events.lock().unwrap().len(), 1);
        assert!(lifecycle_events.events.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    pub async fn test_agent_session_launch_hookwarning保存完了を待たずpty起動済みsessionを返す() {
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
        let barrier = Arc::new(HookHealthSaveBarrier::default());
        let hook_repository = Arc::new(MemoryHookHealthRepository {
            block_first_save: Some(barrier.clone()),
            ..Default::default()
        });
        let hook_health = Arc::new(ProviderHookHealthUsecase::new(hook_repository));
        let usecase = launch_usecase_with_hook_health(
            repository,
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            Arc::new(RecordingTerminal::default()),
            hook_health.clone(),
        );
        let mut launch = tokio::spawn(async move {
            usecase
                .launch_standalone(AgentSessionLaunchRequest {
                    workspace: WorkspaceIdentity::new("/repo"),
                    worktree_path: "/repo/worktree".to_string(),
                    provider: ProviderKind::Codex,
                    rows: 24,
                    cols: 80,
                    caller_request_id: "launch-hook-warning".to_string(),
                })
                .await
        });

        tokio::time::timeout(Duration::from_secs(1), barrier.started.notified())
            .await
            .unwrap();
        let returned_before_warning_save =
            tokio::time::timeout(Duration::from_millis(100), &mut launch)
                .await
                .is_ok();
        barrier.release.notify_one();
        if !returned_before_warning_save {
            launch.await.unwrap().unwrap();
        }
        tokio::time::timeout(Duration::from_secs(1), barrier.completed.notified())
            .await
            .unwrap();

        assert!(returned_before_warning_save);
        assert_eq!(hook_health.warnings().await.unwrap().len(), 1);
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_利用不可providerではsessionもptyも作らない() {
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
        let availability = Arc::new(FixedAvailability {
            available: false,
            checks: Mutex::new(Vec::new()),
        });
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let usecase = launch_usecase(
            repository.clone(),
            availability,
            launch_gateway.clone(),
            terminal.clone(),
        );

        let result = usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/.worktrees/feature".to_string(),
                provider: ProviderKind::Claude,
                rows: 24,
                cols: 80,
                caller_request_id: "launch-request-1".to_string(),
            })
            .await;

        assert_eq!(
            result.unwrap_err(),
            AgentSessionLaunchUsecaseError::ProviderUnavailable
        );
        assert!(repository.stored.lock().unwrap().is_none());
        assert!(launch_gateway.armed.lock().unwrap().is_empty());
        assert!(terminal.spawns.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    pub async fn test_agent_session_launch_同一request_idの並行呼び出しはsessionを一度だけ作成し同じ結果を返す(
    ) {
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
            std::sync::Arc::new(crate::usecase::test_helpers::NoopPerformance),
            Arc::new(AgentSessionUsecase::new(repository.clone())),
            Arc::new(ProviderLifecycleUsecase::new(
                Arc::new(crate::usecase::test_helpers::TestCredentials),
                Arc::new(RecordingLifecycleEvents::default()),
            )),
            provider_runtime(
                Arc::new(FixedAvailability {
                    available: true,
                    checks: Mutex::new(Vec::new()),
                }),
                Arc::new(RecordingLaunchGateway::default()),
                terminal,
            ),
            Arc::new(FixedHistory {
                entries: Vec::new(),
            }),
            hook_health_usecase(),
            started_execution_trees(),
            tokio::sync::mpsc::unbounded_channel().0,
            crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        ));

        let first = tokio::spawn(
            Arc::clone(&usecase)
                .launch_standalone_idempotent(idempotent_launch_request("request-dup")),
        );
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let second = tokio::spawn(
            Arc::clone(&usecase)
                .launch_standalone_idempotent(idempotent_launch_request("request-dup")),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        release_sender.send(()).unwrap();

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with("agent-session-"));
        assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_完了済みrequest_id再送は再作成せず同じsession識別子を返す(
    ) {
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
        let availability = Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        });
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let usecase = Arc::new(launch_usecase(
            repository.clone(),
            availability,
            launch_gateway,
            terminal.clone(),
        ));

        let first = Arc::clone(&usecase)
            .launch_standalone_idempotent(idempotent_launch_request("request-1"))
            .await
            .unwrap();
        let replay = Arc::clone(&usecase)
            .launch_standalone_idempotent(idempotent_launch_request("request-1"))
            .await
            .unwrap();

        assert_eq!(first, replay);
        assert!(first.starts_with("agent-session-"));
        assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
        assert_eq!(terminal.spawns.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_異なるrequest_idは別のsessionを作成する() {
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
        let availability = Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        });
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let usecase = Arc::new(launch_usecase(
            repository.clone(),
            availability,
            launch_gateway,
            terminal.clone(),
        ));

        let first = Arc::clone(&usecase)
            .launch_standalone_idempotent(idempotent_launch_request("request-1"))
            .await
            .unwrap();
        let second = Arc::clone(&usecase)
            .launch_standalone_idempotent(idempotent_launch_request("request-2"))
            .await
            .unwrap();

        assert_ne!(first, second);
        assert!(first.starts_with("agent-session-"));
        assert!(second.starts_with("agent-session-"));
        assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 2);
        assert_eq!(terminal.spawns.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_失敗結果も記録し同一request_id再送へ同じ失敗を返す() {
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
        let availability = Arc::new(FixedAvailability {
            available: false,
            checks: Mutex::new(Vec::new()),
        });
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let usecase = Arc::new(launch_usecase(
            repository.clone(),
            availability.clone(),
            launch_gateway,
            terminal.clone(),
        ));

        let first = Arc::clone(&usecase)
            .launch_standalone_idempotent(idempotent_launch_request("request-fail"))
            .await;
        let replay = Arc::clone(&usecase)
            .launch_standalone_idempotent(idempotent_launch_request("request-fail"))
            .await;

        assert_eq!(
            first.unwrap_err(),
            AgentSessionLaunchUsecaseError::ProviderUnavailable
        );
        assert_eq!(
            replay.unwrap_err(),
            AgentSessionLaunchUsecaseError::ProviderUnavailable
        );
        assert_eq!(availability.checks.lock().unwrap().len(), 1);
        assert!(repository.stored.lock().unwrap().is_none());
        assert!(terminal.spawns.lock().unwrap().is_empty());
    }

    #[test]
    fn test_agent_session_launch_起動panic後はin_flightに残さず同一request_id再送へ同じ失敗を返す()
    {
        let _guard = crate::test_support::lock_crash_telemetry();
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
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
                let availability = Arc::new(PanicOnFirstCheckAvailability {
                    checks: AtomicUsize::new(0),
                });
                let usecase = Arc::new(AgentSessionLaunchUsecase::new(
                    std::sync::Arc::new(crate::usecase::test_helpers::NoopPerformance),
                    Arc::new(AgentSessionUsecase::new(repository.clone())),
                    Arc::new(ProviderLifecycleUsecase::new(
                        Arc::new(crate::usecase::test_helpers::TestCredentials),
                        Arc::new(RecordingLifecycleEvents::default()),
                    )),
                    provider_runtime(
                        availability.clone(),
                        Arc::new(RecordingLaunchGateway::default()),
                        Arc::new(RecordingTerminal::default()),
                    ),
                    Arc::new(FixedHistory {
                        entries: Vec::new(),
                    }),
                    hook_health_usecase(),
                    started_execution_trees(),
                    tokio::sync::mpsc::unbounded_channel().0,
                    crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(
                    ),
                ));

                let first = Arc::clone(&usecase)
                    .launch_standalone_idempotent(idempotent_launch_request("request-panic"))
                    .await;
                let replay = Arc::clone(&usecase)
                    .launch_standalone_idempotent(idempotent_launch_request("request-panic"))
                    .await;

                assert_eq!(first.unwrap_err(), AgentSessionLaunchUsecaseError::Corrupt);
                assert_eq!(replay.unwrap_err(), AgentSessionLaunchUsecaseError::Corrupt);
                assert_eq!(usecase.standalone_in_flight_request_count().await, 0);
                assert_eq!(availability.checks.load(Ordering::SeqCst), 1);
                assert!(repository.stored.lock().unwrap().is_none());
            });
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_prepare失敗時のrollbackでgc失敗なら実行木を解放しない() {
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
        *repository.remove_failure.lock().unwrap() = Some(AgentSessionRepositoryError::Unavailable);
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        *launch_gateway.fail_prepare.lock().unwrap() = true;
        let execution_trees = started_execution_trees();
        let usecase = launch_usecase_with_tree_registrar(
            repository.clone(),
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway,
            Arc::new(RecordingTerminal::default()),
            hook_health_usecase(),
            execution_trees.clone(),
        );

        let error = usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Codex,
                rows: 24,
                cols: 80,
                caller_request_id: "rollback-gc-failure".to_string(),
            })
            .await
            .unwrap_err();

        assert_eq!(
        error,
        AgentSessionLaunchUsecaseError::Launch(
            crate::domain::agent_session::provider_launch_gateway::ProviderAgentLaunchGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into()
                }
            )
        )
    );
        assert!(repository.stored.lock().unwrap().is_some());
        assert!(execution_trees.releases.lock().unwrap().is_empty());
    }

    #[tokio::test]
    pub async fn test_agent_session_launch_codexのhook_delivery未確認を警告しprocessを起動する() {
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
        let availability = Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        });
        let launch_gateway = Arc::new(RecordingLaunchGateway::default());
        let terminal = Arc::new(RecordingTerminal::default());
        let hook_health = hook_health_usecase();
        let usecase = AgentSessionLaunchUsecase::new(
            std::sync::Arc::new(crate::usecase::test_helpers::NoopPerformance),
            Arc::new(AgentSessionUsecase::new(repository)),
            Arc::new(ProviderLifecycleUsecase::new(
                Arc::new(crate::usecase::test_helpers::TestCredentials),
                Arc::new(RecordingLifecycleEvents::default()),
            )),
            provider_runtime(availability, launch_gateway.clone(), terminal.clone()),
            Arc::new(FixedHistory {
                entries: Vec::new(),
            }),
            hook_health.clone(),
            started_execution_trees(),
            tokio::sync::mpsc::unbounded_channel().0,
            crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
        );

        let launched = usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Codex,
                rows: 24,
                cols: 80,
                caller_request_id: "codex-launch-1".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(
            launched.session().id(),
            launch_gateway.armed.lock().unwrap()[0]
                .scope()
                .agent_session_id()
        );
        assert_eq!(terminal.spawns.lock().unwrap().len(), 1);
        let warnings = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let warnings = hook_health.warnings().await.unwrap();
                if !warnings.is_empty() {
                    break warnings;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            warnings[0].reason,
            ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed
        );
    }
}
