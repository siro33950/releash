use releashd::test_support::integration::sessions::session_location;
use releashd::test_support::integration::sessions::workflow_location;

use std::sync::Arc;

use sha2::Digest;
use sha2::Sha256;
use tempfile::TempDir;

use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::sessions::open_session_title_candidates;
use releashd::test_support::integration::sessions::LocalAgentSessionQueryService;
use releashd::test_support::integration::sessions::LocalAgentSessionRepository;
use releashd::test_support::integration::sessions::OPEN_SESSION_LIFECYCLE_EVENT_TYPES;

use releashd::test_support::integration::platform::CommitBatchError;
use releashd::test_support::integration::providers::ProviderKind;
use releashd::test_support::integration::providers::ProviderLifecycleEvent;
use releashd::test_support::integration::providers::ProviderLifecycleScope;
use releashd::test_support::integration::providers::ProviderSessionStartTransaction;
use releashd::test_support::integration::providers::ScopedProviderLifecycleEvent;
use releashd::test_support::integration::repository::LocalEventTransactionRepository;
use releashd::test_support::integration::sessions::AgentSession;
use releashd::test_support::integration::sessions::AgentSessionInitialInstructionOutcome;
use releashd::test_support::integration::sessions::AgentSessionLifecycle;
use releashd::test_support::integration::sessions::AgentSessionLifecycleDto;
use releashd::test_support::integration::sessions::AgentSessionOwnershipQuery;
use releashd::test_support::integration::sessions::AgentSessionQueryService;
use releashd::test_support::integration::sessions::AgentSessionRepository;
use releashd::test_support::integration::sessions::AgentSessionRepositoryError;
use releashd::test_support::integration::sessions::AgentSessionUsecase;
use releashd::test_support::integration::workflow::seed_workflow_session_facts;
use releashd::test_support::integration::workflow::AgentSessionActivity;
use releashd::test_support::integration::workflow::ExecutionOrigin;
use releashd::test_support::integration::workflow::ExecutionTreeLaunch;
use releashd::test_support::integration::workflow::NodeFact;
use releashd::test_support::integration::workflow::NodeKindName;
use releashd::test_support::integration::workflow::StopReceivedFact;
use releashd::test_support::integration::workflow::TreeRootFact;
use releashd::test_support::integration::workflow::WorkflowSessionFactSeed;
use releashd::test_support::integration::workspace::WorkspaceIdentity;

fn open_store(directory: &TempDir) -> Arc<LocalEventStore> {
    LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap()
}

fn new_repository(store: &Arc<LocalEventStore>) -> LocalAgentSessionRepository {
    LocalAgentSessionRepository::new(store.clone())
}

fn standalone_session(id: &str, worktree_path: &str, provider: ProviderKind) -> AgentSession {
    AgentSession::create(
        id,
        WorkspaceIdentity::new(worktree_path),
        worktree_path,
        provider,
        session_location(id),
    )
    .unwrap()
}

async fn tree_event_types(store: &Arc<LocalEventStore>, tree_id: &str) -> Vec<&'static str> {
    releashd::test_support::integration::workflow::read_tree_records(store, tree_id)
        .await
        .unwrap()
        .iter()
        .map(|record| releashd::test_support::integration::workflow::event_type(&record.fact))
        .collect()
}

fn ownership_stream(
    provider: ProviderKind,
    provider_session_id: &str,
) -> releashd::test_support::integration::platform::StreamId {
    let provider = match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    };
    let digest = hex::encode(Sha256::digest(provider_session_id.as_bytes()));
    releashd::test_support::integration::platform::StreamId::provider_session_ownership(
        provider, &digest,
    )
    .unwrap()
}

#[tokio::test]
pub async fn test_agent_session_repository_単独session作成をnode_eventsへ記録し再起動後のfindで同じ状態を導出する(
) {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = standalone_session(
        "agent-session-1",
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
    );

    let saved = repository
        .create(session, "create-request-1")
        .await
        .unwrap();

    assert_eq!(saved.revision(), 1);
    assert!(saved.session().uncommitted_events().is_empty());
    let records =
        releashd::test_support::integration::workflow::read_tree_records(&store, "agent-session-1")
            .await
            .unwrap();
    assert_eq!(records.len(), 3);
    let record = &records[0];
    assert_eq!(record.meta.tree_id, "agent-session-1");
    assert_eq!(record.meta.node_execution_id, "agent-session-1");
    assert_eq!(record.meta.parent_id, None);
    assert_eq!(record.meta.node_name, "session");
    assert_eq!(record.meta.kind, NodeKindName::Session);
    assert_eq!(record.meta.attempt, 1);
    let NodeFact::Started(started) = &record.fact else {
        panic!("session root row must be a started fact: {record:?}");
    };
    assert_eq!(started.parent, None);
    let Some(root) = &started.root else {
        panic!("session root fact must carry the session tree root: {started:?}");
    };
    assert_eq!(root.workspace_identity, "/repo/.worktrees/feature");
    assert_eq!(root.worktree_path, "/repo/.worktrees/feature");
    assert_eq!(root.launched_as, ExecutionTreeLaunch::Session);
    assert_eq!(root.created_from, ExecutionOrigin::DesktopUi);
    let session = root
        .definition
        .as_ref()
        .unwrap()
        .node_by_name("session")
        .and_then(releashd::test_support::integration::workflow::NodeDefinition::session)
        .unwrap();
    assert_eq!(session.provider, ProviderKind::Codex);
    assert!(matches!(
        &records[1].fact,
        NodeFact::SessionAttached(attached)
            if attached.session_id == "agent-session-1"
                && attached.provider_session_id.is_none()
                && attached.transcript_ref.is_none()
    ));
    assert!(matches!(
        &records[2].fact,
        NodeFact::StandaloneSessionNodeCompleted
    ));
    drop(repository);
    drop(store);

    let reopened = open_store(&directory);
    let loaded = new_repository(&reopened)
        .find("agent-session-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.revision(), 3);
    assert_eq!(loaded.session().id(), "agent-session-1");
    assert_eq!(
        loaded.session().workspace().as_str(),
        "/repo/.worktrees/feature"
    );
    assert_eq!(loaded.session().worktree_path(), "/repo/.worktrees/feature");
    assert_eq!(loaded.session().provider(), ProviderKind::Codex);
    assert_eq!(
        loaded.session().tree_location(),
        &session_location("agent-session-1")
    );
    assert_eq!(loaded.session().lifecycle(), AgentSessionLifecycle::Open);
    assert!(loaded.session().uncommitted_events().is_empty());
}

#[tokio::test]
pub async fn test_agent_session_repository_活動遷移だけをnode行へ追記し同値観測は追記しない() {
    // Given: 活動観測を永続化する単独 Session
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let sessions = AgentSessionUsecase::new(Arc::new(new_repository(&store)));
    sessions
        .create(
            "agent-session-activity",
            WorkspaceIdentity::new("workspace-activity"),
            "/repo/activity",
            ProviderKind::Claude,
            session_location("agent-session-activity"),
            "create-activity-session",
        )
        .await
        .unwrap();

    // When: Working と AwaitingInstruction を2往復し、同値も再観測する
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::Working,
                "activity-working",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::Working,
                "activity-working-duplicate",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::AlreadyApplied
    );
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::AwaitingInstruction,
                "activity-awaiting-instruction-1",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::Working,
                "activity-working-2",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::AwaitingInstruction,
                "activity-awaiting-instruction-2",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::Working,
                "activity-working-3",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity",
                AgentSessionActivity::Working,
                "activity-working-3-duplicate",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::AlreadyApplied
    );

    // Then: 遷移だけが事実として同じ Session Node へ追記される
    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-activity",
    )
    .await
    .unwrap();
    let activities = records
        .iter()
        .filter_map(|record| match &record.fact {
            NodeFact::AgentActivityObserved(fact) => Some(fact.activity),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        activities,
        [
            AgentSessionActivity::Working,
            AgentSessionActivity::AwaitingInstruction,
            AgentSessionActivity::Working,
            AgentSessionActivity::AwaitingInstruction,
            AgentSessionActivity::Working,
        ]
    );
    assert!(records
        .iter()
        .filter(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_)))
        .all(|record| record.meta.node_execution_id == "agent-session-activity"));

    // When: store を開き直して Session を復元する
    drop(sessions);
    drop(store);

    let reopened = open_store(&directory);
    let loaded = new_repository(&reopened)
        .find("agent-session-activity")
        .await
        .unwrap()
        .unwrap();

    // Then: 最後に観測した活動状態が復元される
    assert_eq!(loaded.session().activity(), AgentSessionActivity::Working);
}

#[tokio::test]
pub async fn test_agent_session_repository_process_exit後のworking再観測を活動遷移として追記する() {
    // Given: Working の活動観測後に ProcessExited を記録した単独 Session
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = Arc::new(new_repository(&store));
    let sessions = AgentSessionUsecase::new(repository.clone());
    sessions
        .create(
            "agent-session-activity-exit",
            WorkspaceIdentity::new("workspace-activity-exit"),
            "/repo/activity-exit",
            ProviderKind::Codex,
            session_location("agent-session-activity-exit"),
            "create-activity-exit-session",
        )
        .await
        .unwrap();
    let mut saved = repository
        .find("agent-session-activity-exit")
        .await
        .unwrap()
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-activity-exit", None)
        .unwrap();
    repository
        .save(saved, "associate-activity-exit-session")
        .await
        .unwrap();
    assert_eq!(
        sessions
            .observe_activity(
                "agent-session-activity-exit",
                AgentSessionActivity::Working,
                "activity-before-exit",
            )
            .await
            .unwrap()
            .outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    sessions
        .observe_process_exit(
            "agent-session-activity-exit",
            Some(0),
            "activity-process-exit",
        )
        .await
        .unwrap();
    let before = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-activity-exit",
    )
    .await
    .unwrap();
    assert!(matches!(
        before.last().unwrap().fact,
        NodeFact::ProcessExited(_)
    ));
    assert_eq!(
        before
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_)))
            .count(),
        1
    );

    // When: bounded read 経路から同じ Working を再観測する
    let observation = sessions
        .observe_activity(
            "agent-session-activity-exit",
            AgentSessionActivity::Working,
            "activity-after-exit",
        )
        .await
        .unwrap();

    // Then: ProcessExited 後は遷移として受理され、活動事実が1件増える
    assert_eq!(
        observation.outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    let after = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-activity-exit",
    )
    .await
    .unwrap();
    assert_eq!(
        after
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_)))
            .count(),
        2
    );
    assert!(matches!(
        after.last().unwrap().fact,
        NodeFact::AgentActivityObserved(ref fact)
            if fact.activity == AgentSessionActivity::Working
    ));
}

#[tokio::test]
pub async fn test_agent_session_repository_stop事実後のworking再観測をbounded_readから活動遷移として追記する(
) {
    // Given: Working の活動観測後に StopReceived だけを記録した単独 Session
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = Arc::new(new_repository(&store));
    let sessions = AgentSessionUsecase::new(repository.clone());
    let session_id = "agent-session-activity-stop";
    sessions
        .create(
            session_id,
            WorkspaceIdentity::new("workspace-activity-stop"),
            "/repo/activity-stop",
            ProviderKind::Codex,
            session_location(session_id),
            "create-activity-stop-session",
        )
        .await
        .unwrap();
    sessions
        .observe_activity(
            session_id,
            AgentSessionActivity::Working,
            "activity-before-stop",
        )
        .await
        .unwrap();
    let records =
        releashd::test_support::integration::workflow::read_tree_records(&store, session_id)
            .await
            .unwrap();
    let meta = records.last().unwrap().meta.clone();
    releashd::test_support::integration::workflow::append_single_fact(
        &store,
        &meta,
        &NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
        10,
    )
    .await
    .unwrap();

    let restored = repository
        .find_for_activity(session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.session().activity(),
        AgentSessionActivity::AwaitingInstruction
    );

    // When: Stop より後に Working を観測する
    let observation = sessions
        .observe_activity(
            session_id,
            AgentSessionActivity::Working,
            "activity-after-stop",
        )
        .await
        .unwrap();

    // Then: bounded read が Stop を最新活動入力として読み、Working を新しい遷移として追記する
    assert_eq!(
        observation.outcome,
        releashd::test_support::integration::sessions::AgentSessionMutationOutcome::Applied
    );
    let records =
        releashd::test_support::integration::workflow::read_tree_records(&store, session_id)
            .await
            .unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::StopReceived(_)))
            .count(),
        1
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_)))
            .count(),
        2
    );
    assert!(matches!(
        records.last().unwrap().fact,
        NodeFact::AgentActivityObserved(ref fact)
            if fact.activity == AgentSessionActivity::Working
    ));
}

#[tokio::test]
pub async fn test_agent_session_repository_workflow子sessionも同じ活動保存経路を使う() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "activity-workflow",
            request: "implement",
            worktree_path: "/repo/workflow-activity",
            provider: ProviderKind::Codex,
            workflow_execution_id: "workflow-activity",
            node_execution_id: "workflow-session-node",
            session_id: "workflow-agent-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let sessions = AgentSessionUsecase::new(Arc::new(new_repository(&store)));

    sessions
        .observe_activity(
            "workflow-agent-session",
            AgentSessionActivity::Working,
            "workflow-activity-working",
        )
        .await
        .unwrap();

    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "workflow-activity",
    )
    .await
    .unwrap();
    assert!(matches!(
        &records.last().unwrap().fact,
        NodeFact::AgentActivityObserved(fact)
            if fact.activity == AgentSessionActivity::Working
                && records.last().unwrap().meta.node_execution_id == "workflow-session-node"
    ));
}

#[tokio::test]
pub async fn test_agent_session_repository_続行指示の受理を木の事実として保存し再読込後も同じ識別子を拒む(
) {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "delegate-workflow",
            request: "implement",
            worktree_path: "/repo/workflow-delegate",
            provider: ProviderKind::Codex,
            workflow_execution_id: "workflow-delegate",
            node_execution_id: "workflow-session-node",
            session_id: "workflow-agent-session",
            initial_instruction_admitted: true,
        },
    )
    .await
    .unwrap();
    let repository = new_repository(&store);
    let mut saved = repository
        .find("workflow-agent-session")
        .await
        .unwrap()
        .unwrap();
    saved
        .session_mut()
        .admit_continuation("workflow-delegate-continuation-child-1")
        .unwrap();
    repository
        .save(saved, "delegate-continuation-child-1")
        .await
        .unwrap();

    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "workflow-delegate",
    )
    .await
    .unwrap();
    assert!(matches!(
        &records.last().unwrap().fact,
        NodeFact::SessionContinuationAdmitted(fact)
            if fact.session_id == "workflow-agent-session"
                && fact.request_id == "workflow-delegate-continuation-child-1"
                && records.last().unwrap().meta.node_execution_id == "workflow-session-node"
    ));
    let mut reloaded = repository
        .find("workflow-agent-session")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded
            .session_mut()
            .admit_continuation("workflow-delegate-continuation-child-1")
            .unwrap(),
        AgentSessionInitialInstructionOutcome::AlreadyAdmitted
    );
    assert_eq!(
        reloaded
            .session_mut()
            .admit_continuation("workflow-delegate-continuation-child-2")
            .unwrap(),
        AgentSessionInitialInstructionOutcome::Admitted
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_workspace同定子はworktreeと独立に往復する() {
    // Given: workspace 同定子が worktree パスと異なる session
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = AgentSession::create(
        "agent-session-ws",
        WorkspaceIdentity::new("workspace-1"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        session_location("agent-session-ws"),
    )
    .unwrap();
    repository
        .create(session, "create-request-1")
        .await
        .unwrap();

    // When: 事実列から導出する
    let found = repository.find("agent-session-ws").await.unwrap().unwrap();

    // Then: launch 時の workspace が往復する（terminal surface の owner 鍵になる）
    assert_eq!(found.session().workspace().as_str(), "workspace-1");
    assert_eq!(found.session().worktree_path(), "/repo/.worktrees/feature");
}

#[tokio::test]
pub async fn test_agent_session_repository_同一idの再createを拒否する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            standalone_session("agent-session-1", "/repo", ProviderKind::Codex),
            "create-request-1",
        )
        .await
        .unwrap();

    let error = repository
        .create(
            standalone_session("agent-session-1", "/repo", ProviderKind::Codex),
            "create-request-2",
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        releashd::test_support::integration::sessions::AgentSessionRepositoryError::Conflict
    );
    assert_eq!(
        tree_event_types(&store, "agent-session-1").await,
        [
            "started",
            "session_attached",
            "standalone_session_node_completed"
        ]
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_workflow子sessionのcreateは木に行を追加しない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut session = AgentSession::create(
        "agent-session-workflow",
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Codex,
        workflow_location("workflow-1", "node-execution-1"),
    )
    .unwrap();
    session.admit_initial_instruction().unwrap();
    let saved = repository
        .create_with_lifecycle_events(session, Vec::new(), "create-workflow-request-1")
        .await
        .unwrap();

    assert!(saved.session().initial_instruction_admitted());
    assert!(
        releashd::test_support::integration::workflow::read_tree_records(&store, "workflow-1")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        releashd::test_support::integration::workflow::read_tree_records(
            &store,
            "agent-session-workflow"
        )
        .await
        .unwrap()
        .is_empty()
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_attach前に再起動したworkflow子sessionを再armする() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = || {
        let mut session = AgentSession::create(
            "agent-session-workflow-rearm",
            WorkspaceIdentity::new("/repo"),
            "/repo",
            ProviderKind::Codex,
            workflow_location("workflow-1", "node-execution-1"),
        )
        .unwrap();
        session.admit_initial_instruction().unwrap();
        session
    };
    let lifecycle = |slot: &str, binding: &str| {
        let scope = ProviderLifecycleScope::new("agent-session-workflow-rearm").unwrap();
        vec![ScopedProviderLifecycleEvent::new(
            scope.clone(),
            ProviderLifecycleEvent::binding_armed(slot, binding, ProviderKind::Codex, scope)
                .unwrap(),
        )]
    };

    repository
        .create_with_lifecycle_events(
            session(),
            lifecycle("slot-1", "binding-1"),
            "workflow-rearm-request",
        )
        .await
        .unwrap();
    repository
        .create_with_lifecycle_events(
            session(),
            lifecycle("slot-1", "binding-2"),
            "workflow-rearm-request",
        )
        .await
        .unwrap();

    let stream = store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id:
                    releashd::test_support::integration::platform::StreamId::provider_lifecycle(
                        "agent-session-workflow-rearm",
                    )
                    .unwrap(),
                after: None,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert_eq!(stream.events.len(), 2);
    assert!(
        releashd::test_support::integration::workflow::read_tree_records(&store, "workflow-1")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_provider紐付けと状態遷移を事実行として永続化する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = standalone_session(
        "agent-session-1",
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
    );
    let mut saved = repository
        .create(session, "create-request-1")
        .await
        .unwrap();

    saved
        .session_mut()
        .associate_provider_session("provider-session-1", Some("provider://transcript/1"))
        .unwrap();
    saved.session_mut().observe_provider_process_exit(Some(0));
    let updated = repository.save(saved, "pause-request-1").await.unwrap();

    assert_eq!(updated.revision(), 3);
    assert!(updated.session().uncommitted_events().is_empty());
    assert_eq!(
        tree_event_types(&store, "agent-session-1").await,
        [
            "started",
            "session_attached",
            "standalone_session_node_completed",
            "session_attached",
            "process_exited"
        ]
    );
    drop(repository);
    drop(store);

    let reopened = open_store(&directory);
    let loaded = new_repository(&reopened)
        .find("agent-session-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.revision(), 5);
    assert_eq!(
        loaded.session().provider_session_id(),
        Some("provider-session-1")
    );
    assert_eq!(
        loaded.session().transcript_ref(),
        Some("provider://transcript/1")
    );
    assert_eq!(loaded.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert!(!loaded.session().last_exit_abnormal());
}

#[tokio::test]
pub async fn test_agent_session_repository_openかつprovider_session確定済みのsessionだけを軽量列挙する(
) {
    // Given: Open・paused・archived と、provider session id 未確定の Session
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    for id in ["open-session", "paused-session", "archived-session"] {
        let mut saved = repository
            .create(
                standalone_session(id, "/repo", ProviderKind::Claude),
                &format!("create-{id}"),
            )
            .await
            .unwrap();
        let transcript_ref = format!("provider://transcript/{id}");
        saved
            .session_mut()
            .associate_provider_session(format!("provider-{id}"), Some(&transcript_ref))
            .unwrap();
        let mut saved = repository
            .save(saved, &format!("associate-{id}"))
            .await
            .unwrap();
        match id {
            "open-session" => {
                saved
                    .session_mut()
                    .observe_provider_session_title("Old title")
                    .unwrap();
                let mut saved = repository
                    .save_provider_session_title(saved, "old-open-title")
                    .await
                    .unwrap();
                saved
                    .session_mut()
                    .observe_provider_session_title("Current title")
                    .unwrap();
                repository
                    .save_provider_session_title(saved, "current-open-title")
                    .await
                    .unwrap();
            }
            "paused-session" => {
                saved.session_mut().observe_provider_process_exit(Some(0));
                repository.save(saved, "pause-session").await.unwrap();
            }
            "archived-session" => {
                saved.session_mut().archive().unwrap();
                repository.save(saved, "archive-session").await.unwrap();
            }
            _ => {}
        }
    }
    repository
        .create(
            standalone_session("unattached-session", "/repo", ProviderKind::Codex),
            "create-unattached-session",
        )
        .await
        .unwrap();

    // When: 一括取得した lifecycle 事実から追加読み対象を絞り、Session を列挙する
    let lifecycle_records =
        releashd::test_support::integration::workflow::read_records_for_event_types(
            &releashd::test_support::integration::workflow::FactLogReadBackend::Live(Arc::clone(
                &store,
            )),
            OPEN_SESSION_LIFECYCLE_EVENT_TYPES,
        )
        .await
        .unwrap();
    let candidates = open_session_title_candidates(lifecycle_records);
    let sessions = repository
        .list_open_for_provider_session_title()
        .await
        .unwrap();

    // Then: 追加読み候補にも返却値にも provider id 付きの Open だけが残る
    assert_eq!(
        candidates.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["open-session"]
    );
    let candidate = candidates.get("open-session").unwrap();
    assert_eq!(candidate.test_location().tree_id, "open-session");
    assert_eq!(candidate.test_location().node_execution_id, "open-session");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session().id(), "open-session");
    assert_eq!(sessions[0].session().provider(), ProviderKind::Claude);
    assert_eq!(sessions[0].session().worktree_path(), "/repo");
    assert_eq!(
        sessions[0].session().lifecycle(),
        AgentSessionLifecycle::Open
    );
    assert_eq!(
        sessions[0].session().provider_session_id(),
        Some("provider-open-session")
    );
    assert_eq!(
        sessions[0].session().transcript_ref(),
        Some("provider://transcript/open-session")
    );
    assert_eq!(sessions[0].session().manual_name(), None);
    assert_eq!(
        sessions[0].session().provider_session_title(),
        Some("Current title")
    );
    assert_eq!(sessions[0].revision(), 6);
}

#[tokio::test]
pub async fn test_agent_session_repository_renameとproviderタイトルを対応する事実へ保存し再取得する(
) {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            standalone_session("named-session", "/repo", ProviderKind::Codex),
            "create-named-session",
        )
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-named-session", None)
        .unwrap();
    let mut saved = repository
        .save(saved, "associate-named-session")
        .await
        .unwrap();
    saved.session_mut().rename("  release review  ").unwrap();
    let saved = repository.save(saved, "rename-session").await.unwrap();
    let mut observed = repository.find("named-session").await.unwrap().unwrap();
    observed
        .session_mut()
        .observe_provider_session_title("  Generated title  ")
        .unwrap();
    repository
        .save_provider_session_title(observed, "observe-provider-title")
        .await
        .unwrap();

    let restored = repository.find("named-session").await.unwrap().unwrap();

    assert_eq!(restored.session().manual_name(), Some("release review"));
    assert_eq!(
        restored.session().provider_session_title(),
        Some("Generated title")
    );
    assert!(saved.session().uncommitted_events().is_empty());
    assert_eq!(
        tree_event_types(&store, "named-session").await,
        [
            "started",
            "session_attached",
            "standalone_session_node_completed",
            "session_attached",
            "session_node_renamed",
            "provider_session_title_observed",
        ]
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_providerタイトル軽量保存は他のeventが混ざる要求を拒否する(
) {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            standalone_session("invalid-title-save", "/repo", ProviderKind::Claude),
            "create-invalid-title-save",
        )
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-invalid-title-save", None)
        .unwrap();
    let mut saved = repository
        .save(saved, "associate-invalid-title-save")
        .await
        .unwrap();
    saved.session_mut().rename("manual name").unwrap();
    saved
        .session_mut()
        .observe_provider_session_title("provider title")
        .unwrap();
    let rows_before = tree_event_types(&store, "invalid-title-save").await;

    let result = repository
        .save_provider_session_title(saved, "invalid-title-observation")
        .await;

    assert_eq!(result, Err(AgentSessionRepositoryError::InvalidRequest));
    assert_eq!(
        tree_event_types(&store, "invalid-title-save").await,
        rows_before
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_異常exitをfailure付きprocess_exitedとして記録する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            standalone_session("agent-session-abnormal", "/repo", ProviderKind::Codex),
            "create-request-1",
        )
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-abnormal", None)
        .unwrap();
    saved.session_mut().observe_provider_process_exit(Some(1));

    repository.save(saved, "abnormal-exit-1").await.unwrap();

    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-abnormal",
    )
    .await
    .unwrap();
    let NodeFact::ProcessExited(exited) = &records.last().unwrap().fact else {
        panic!("abnormal exit must be recorded as a process_exited fact: {records:?}");
    };
    assert_eq!(exited.exit_code, Some(1));
    assert!(exited.failure_reason.is_some());
    let loaded = repository
        .find("agent-session-abnormal")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert!(loaded.session().last_exit_abnormal());
}

#[tokio::test]
pub async fn test_agent_session_repository_異常終了したsession起動木をresumeする() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            standalone_session(
                "agent-session-abnormal-resume",
                "/repo",
                ProviderKind::Codex,
            ),
            "create-abnormal-resume",
        )
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-abnormal-resume", None)
        .unwrap();
    saved.session_mut().observe_provider_process_exit(Some(1));
    let mut saved = repository
        .save(saved, "abnormal-exit-before-resume")
        .await
        .unwrap();
    let failed = releashd::test_support::integration::workflow::fold_tree_from(
        &releashd::test_support::integration::workflow::FactLogReadBackend::Live(store.clone()),
        "agent-session-abnormal-resume",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        failed
            .aggregate
            .node_execution("agent-session-abnormal-resume")
            .unwrap()
            .status,
        releashd::test_support::integration::workflow::NodeExecutionStatus::Succeeded
    );

    saved
        .session_mut()
        .complete_resume(
            releashd::test_support::integration::sessions::AgentSessionRecoveryResult::Succeeded,
        )
        .unwrap();
    repository
        .save(saved, "resume-after-abnormal-exit")
        .await
        .unwrap();

    let resumed = releashd::test_support::integration::workflow::fold_tree_from(
        &releashd::test_support::integration::workflow::FactLogReadBackend::Live(store),
        "agent-session-abnormal-resume",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        resumed
            .aggregate
            .node_execution("agent-session-abnormal-resume")
            .unwrap()
            .status,
        releashd::test_support::integration::workflow::NodeExecutionStatus::Succeeded
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_restore後の指示待ちを事実から復元する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            standalone_session(
                "agent-session-restore-activity",
                "/repo",
                ProviderKind::Claude,
            ),
            "create-restore-activity",
        )
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-restore-activity", None)
        .unwrap();
    saved
        .session_mut()
        .observe_activity(AgentSessionActivity::Working);
    let mut saved = repository
        .save(saved, "working-before-archive")
        .await
        .unwrap();
    saved.session_mut().archive().unwrap();
    repository
        .save(saved, "archive-working-session")
        .await
        .unwrap();

    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-restore-activity",
    )
    .await
    .unwrap();
    releashd::test_support::integration::workflow::append_single_fact(
        &store,
        &records[0].meta,
        &NodeFact::RestoreRequested,
        100,
    )
    .await
    .unwrap();
    let restored = repository
        .find("agent-session-restore-activity")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.session().activity(),
        AgentSessionActivity::AwaitingInstruction
    );
    let activities = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-restore-activity",
    )
    .await
    .unwrap()
    .into_iter()
    .filter_map(|record| match record.fact {
        NodeFact::AgentActivityObserved(fact) => Some(fact.activity),
        _ => None,
    })
    .collect::<Vec<_>>();
    assert_eq!(activities, [AgentSessionActivity::Working]);
}

#[tokio::test]
pub async fn test_agent_session_repository_resumeとarchiveとrestoreを行として記録し導出する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            standalone_session("agent-session-flow", "/repo", ProviderKind::Claude),
            "create-request-1",
        )
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-flow", None)
        .unwrap();
    saved.session_mut().observe_provider_process_exit(Some(0));
    let mut saved = repository.save(saved, "pause-request-1").await.unwrap();

    saved
        .session_mut()
        .complete_resume(
            releashd::test_support::integration::sessions::AgentSessionRecoveryResult::Succeeded,
        )
        .unwrap();
    let mut saved = repository.save(saved, "resume-request-1").await.unwrap();
    let resumed = repository
        .find("agent-session-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.session().lifecycle(), AgentSessionLifecycle::Open);
    assert!(!resumed.session().last_exit_abnormal());

    saved.session_mut().archive().unwrap();
    repository.save(saved, "archive-request-1").await.unwrap();
    let archived = repository
        .find("agent-session-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        archived.session().lifecycle(),
        AgentSessionLifecycle::Archived
    );

    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "agent-session-flow",
    )
    .await
    .unwrap();
    releashd::test_support::integration::workflow::append_single_fact(
        &store,
        &records[0].meta,
        &NodeFact::RestoreRequested,
        100,
    )
    .await
    .unwrap();
    let restored = repository
        .find("agent-session-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored.session().lifecycle(),
        AgentSessionLifecycle::Paused
    );
    assert_eq!(
        tree_event_types(&store, "agent-session-flow").await,
        [
            "started",
            "session_attached",
            "standalone_session_node_completed",
            "session_attached",
            "process_exited",
            "resume_requested",
            "archive_requested",
            "restore_requested",
        ]
    );
}

#[tokio::test]
pub async fn test_agent_session_repository同じprovider_session_idの同時所有を原子的に拒否する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = Arc::new(new_repository(&store));
    let first = standalone_session(
        "agent-session-1",
        "/repo/.worktrees/first",
        ProviderKind::Codex,
    );
    let second = standalone_session(
        "agent-session-2",
        "/repo/.worktrees/second",
        ProviderKind::Codex,
    );
    let mut first = repository.create(first, "create-request-1").await.unwrap();
    let mut second = repository.create(second, "create-request-2").await.unwrap();
    first
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    second
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();

    let (first_result, second_result) = tokio::join!(
        repository.save(first, "associate-request-1"),
        repository.save(second, "associate-request-2")
    );

    let (winner, loser) = match (first_result, second_result) {
        (Ok(winner), Err(loser)) | (Err(loser), Ok(winner)) => (winner, loser),
        results => panic!("exactly one association must win: {results:?}"),
    };
    // 同時実行の敗者は CAS 敗北後に勝者を読み直し、所有者付きで決定的に拒否される。
    assert_eq!(
        loser,
        releashd::test_support::integration::sessions::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
            agent_session_id: winner.session().id().to_string(),
        }
    );
    let loser_id = if winner.session().id() == "agent-session-1" {
        "agent-session-2"
    } else {
        "agent-session-1"
    };
    let mut retried = repository.find(loser_id).await.unwrap().unwrap();
    assert_eq!(retried.session().provider_session_id(), None);

    // 所有が確定した後の再試行は所有者付きで決定的に拒否される。
    retried
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    let retry_error = repository
        .save(retried, "associate-request-retry")
        .await
        .unwrap_err();
    assert_eq!(
        retry_error,
        releashd::test_support::integration::sessions::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
            agent_session_id: winner.session().id().to_string(),
        }
    );
}

#[tokio::test]
pub async fn test_agent_session_repository削除で木の行を物理削除しprovider所有権を解放する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let first = standalone_session(
        "agent-session-1",
        "/repo/.worktrees/first",
        ProviderKind::Claude,
    );
    let mut first = repository.create(first, "create-request-1").await.unwrap();
    first
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    let mut first = repository.save(first, "associate-request-1").await.unwrap();
    assert!(repository
        .is_owned(ProviderKind::Claude, "provider-session-1")
        .await
        .unwrap());
    first.session_mut().archive().unwrap();
    let first = repository.save(first, "archive-request-1").await.unwrap();
    let authorization = first.session().authorize_delete().unwrap();

    repository
        .remove(first, authorization, "delete-request-1")
        .await
        .unwrap();

    assert!(repository.find("agent-session-1").await.unwrap().is_none());
    assert!(
        releashd::test_support::integration::workflow::read_tree_records(&store, "agent-session-1")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!repository
        .is_owned(ProviderKind::Claude, "provider-session-1")
        .await
        .unwrap());
    let ownership_page = store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id: ownership_stream(ProviderKind::Claude, "provider-session-1"),
                after: None,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert!(ownership_page.events.is_empty());
    assert_eq!(ownership_page.head.value(), 0);

    let second = standalone_session(
        "agent-session-2",
        "/repo/.worktrees/second",
        ProviderKind::Claude,
    );
    let mut second = repository.create(second, "create-request-2").await.unwrap();
    second
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    let second = repository
        .save(second, "associate-request-2")
        .await
        .unwrap();
    assert_eq!(
        second.session().provider_session_id(),
        Some("provider-session-1")
    );
}

#[tokio::test]
pub async fn test_agent_session_repository削除失敗時に木とprovider所有権を原子的に維持する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = standalone_session(
        "agent-session-atomic-delete",
        "/repo/.worktrees/atomic-delete",
        ProviderKind::Claude,
    );
    let mut session = repository
        .create(session, "create-atomic-delete")
        .await
        .unwrap();
    session
        .session_mut()
        .associate_provider_session("provider-session-atomic-delete", None)
        .unwrap();
    let mut session = repository
        .save(session, "associate-atomic-delete")
        .await
        .unwrap();
    session.session_mut().archive().unwrap();
    let session = repository
        .save(session, "archive-atomic-delete")
        .await
        .unwrap();
    let authorization = session.session().authorize_delete().unwrap();
    store.fault_injector().arm_fail_before_commit();

    let result = repository
        .remove(session, authorization, "delete-atomic")
        .await;

    assert!(
        matches!(result.unwrap_err(), releashd::test_support::integration::sessions::AgentSessionRepositoryError::Store(failure)
        if failure.nature == releashd::test_support::integration::platform::TechnicalFailureNature::Transient
        && matches!(failure.source, releashd::test_support::integration::platform::StorageFailureSource::Commit(CommitBatchError::StorageUnavailable { .. })))
    );
    let retained = repository
        .find("agent-session-atomic-delete")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !releashd::test_support::integration::workflow::read_tree_records(
            &store,
            "agent-session-atomic-delete"
        )
        .await
        .unwrap()
        .is_empty()
    );
    assert!(repository
        .is_owned(ProviderKind::Claude, "provider-session-atomic-delete")
        .await
        .unwrap());

    repository
        .remove(
            retained.clone(),
            retained.session().authorize_delete().unwrap(),
            "delete-atomic",
        )
        .await
        .unwrap();
    assert!(repository
        .find("agent-session-atomic-delete")
        .await
        .unwrap()
        .is_none());
    assert!(!repository
        .is_owned(ProviderKind::Claude, "provider-session-atomic-delete")
        .await
        .unwrap());
}

#[tokio::test]
pub async fn test_agent_session_repository永続化失敗時に所有権も導出状態も進めない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = standalone_session(
        "agent-session-1",
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
    );
    let mut saved = repository
        .create(session, "create-request-1")
        .await
        .unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    store.fault_injector().arm_fail_before_commit();

    let result = repository.save(saved, "associate-request-1").await;

    assert!(
        matches!(result.unwrap_err(), releashd::test_support::integration::sessions::AgentSessionRepositoryError::Store(failure)
        if failure.nature == releashd::test_support::integration::platform::TechnicalFailureNature::Transient
        && matches!(failure.source, releashd::test_support::integration::platform::StorageFailureSource::Commit(CommitBatchError::StorageUnavailable { .. })))
    );
    let unchanged = repository.find("agent-session-1").await.unwrap().unwrap();
    assert_eq!(unchanged.revision(), 3);
    assert_eq!(unchanged.session().provider_session_id(), None);

    let second = standalone_session(
        "agent-session-2",
        "/repo/.worktrees/second",
        ProviderKind::Codex,
    );
    let mut second = repository.create(second, "create-request-2").await.unwrap();
    second
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    assert!(repository.save(second, "associate-request-2").await.is_ok());
}

#[tokio::test]
pub async fn test_agent_session_repository_session_startをlifecycleと原子的に永続化する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = standalone_session(
        "agent-session-atomic",
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
    );
    let mut saved = repository.create(session, "create-atomic").await.unwrap();
    saved
        .session_mut()
        .associate_provider_session("provider-session-atomic", None)
        .unwrap();
    let scope = ProviderLifecycleScope::new("agent-session-atomic").unwrap();
    let lifecycle_events = vec![ScopedProviderLifecycleEvent::new(
        scope,
        ProviderLifecycleEvent::session_associated(
            "binding-atomic",
            "provider-session-atomic",
            None,
        )
        .unwrap(),
    )];
    store.fault_injector().arm_fail_before_commit();

    let failed = repository
        .commit_session_started(
            saved.clone(),
            lifecycle_events.clone(),
            "session-start-atomic",
        )
        .await;

    assert!(matches!(failed.unwrap_err(),
        releashd::test_support::integration::sessions::AgentSessionRepositoryError::Store(failure)
        if failure.nature == releashd::test_support::integration::platform::TechnicalFailureNature::Transient
        && matches!(failure.source, releashd::test_support::integration::platform::StorageFailureSource::Commit(CommitBatchError::StorageUnavailable { .. }))));
    assert_eq!(
        repository
            .find("agent-session-atomic")
            .await
            .unwrap()
            .unwrap()
            .session()
            .provider_session_id(),
        None
    );
    let lifecycle_stream = store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id:
                    releashd::test_support::integration::platform::StreamId::provider_lifecycle(
                        "agent-session-atomic",
                    )
                    .unwrap(),
                after: None,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert!(lifecycle_stream.events.is_empty());

    repository
        .commit_session_started(saved, lifecycle_events, "session-start-atomic-retry")
        .await
        .unwrap();
    assert_eq!(
        repository
            .find("agent-session-atomic")
            .await
            .unwrap()
            .unwrap()
            .session()
            .provider_session_id(),
        Some("provider-session-atomic")
    );
    let lifecycle_stream = store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id:
                    releashd::test_support::integration::platform::StreamId::provider_lifecycle(
                        "agent-session-atomic",
                    )
                    .unwrap(),
                after: None,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert_eq!(lifecycle_stream.events.len(), 1);
}

#[tokio::test]
pub async fn test_agent_session_repository_単独rootとprovider_lifecycleを原子的に永続化する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = standalone_session(
        "agent-session-create-atomic",
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
    );
    let scope = ProviderLifecycleScope::new("agent-session-create-atomic").unwrap();
    let lifecycle_events = vec![ScopedProviderLifecycleEvent::new(
        scope.clone(),
        ProviderLifecycleEvent::binding_armed(
            "slot-create-atomic",
            "binding-create-atomic",
            ProviderKind::Codex,
            scope,
        )
        .unwrap(),
    )];
    store.fault_injector().arm_fail_after_participant_write(1);

    let failed = repository
        .create_with_lifecycle_events(session, lifecycle_events.clone(), "create-atomic-request")
        .await;

    assert!(matches!(failed.unwrap_err(),
        releashd::test_support::integration::sessions::AgentSessionRepositoryError::Store(failure)
        if failure.nature == releashd::test_support::integration::platform::TechnicalFailureNature::Transient
        && matches!(failure.source, releashd::test_support::integration::platform::StorageFailureSource::Commit(CommitBatchError::StorageUnavailable { .. }))));
    assert!(repository
        .find("agent-session-create-atomic")
        .await
        .unwrap()
        .is_none());
    let stream_id = releashd::test_support::integration::platform::StreamId::provider_lifecycle(
        "agent-session-create-atomic",
    )
    .unwrap();
    assert!(store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id: stream_id.clone(),
                after: None,
                limit: 16,
            }
        )
        .await
        .unwrap()
        .events
        .is_empty());

    repository
        .create_with_lifecycle_events(
            standalone_session(
                "agent-session-create-atomic",
                "/repo/.worktrees/feature",
                ProviderKind::Codex,
            ),
            lifecycle_events,
            "create-atomic-request",
        )
        .await
        .unwrap();
    assert!(repository
        .find("agent-session-create-atomic")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .load_stream(
                releashd::test_support::integration::platform::LoadStreamRequest {
                    stream_id,
                    after: None,
                    limit: 16,
                }
            )
            .await
            .unwrap()
            .events
            .len(),
        1
    );
}

#[tokio::test]
pub async fn test_agent_session_repository_session起動由来の同一要求を既存sessionへ再armする() {
    // Given: caller request から導出した id の Session が provider history と紐付いている
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let caller_request_id = "standalone-restart-request";
    let session_id = releashd::test_support::integration::sessions::launch_resource_id(
        "agent-session",
        caller_request_id,
    )
    .unwrap();
    let lifecycle = |binding: &str| {
        let scope = ProviderLifecycleScope::new(&session_id).unwrap();
        vec![ScopedProviderLifecycleEvent::new(
            scope.clone(),
            ProviderLifecycleEvent::binding_armed(
                "standalone-restart-slot",
                binding,
                ProviderKind::Codex,
                scope,
            )
            .unwrap(),
        )]
    };

    let mut created = repository
        .create_with_lifecycle_events(
            standalone_session(&session_id, "/repo", ProviderKind::Codex),
            lifecycle("binding-1"),
            caller_request_id,
        )
        .await
        .unwrap();
    created
        .session_mut()
        .associate_provider_session("provider-history-1", None)
        .unwrap();
    repository
        .save(created, "standalone-restart-request.associate")
        .await
        .unwrap();

    // When: 同じ caller request で Session の create を再送する
    let rearmed = repository
        .create_with_lifecycle_events(
            standalone_session(&session_id, "/repo", ProviderKind::Codex),
            lifecycle("binding-2"),
            caller_request_id,
        )
        .await
        .unwrap();

    // Then: provider history と紐付いた既存 Session を返し、rearm の lifecycle event を追記する
    assert_eq!(rearmed.session().id(), session_id);
    assert_eq!(
        rearmed.session().provider_session_id(),
        Some("provider-history-1")
    );
    assert_eq!(
        rearmed.session().tree_location(),
        &session_location(&session_id)
    );
    assert_eq!(
        tree_event_types(&store, &session_id).await,
        [
            "started",
            "session_attached",
            "standalone_session_node_completed",
            "session_attached"
        ]
    );
    let stream = store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id:
                    releashd::test_support::integration::platform::StreamId::provider_lifecycle(
                        &session_id,
                    )
                    .unwrap(),
                after: None,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert_eq!(stream.events.len(), 2);
    assert!(matches!(
        &stream.events[1].event,
        releashd::test_support::integration::platform::LoadedDomainEvent::Known(event)
            if matches!(
                event.as_ref(),
                releashd::test_support::integration::platform::LocalDomainEvent::ProviderLifecycle(
                    ProviderLifecycleEvent::BindingArmed { binding_id, .. }
                ) if binding_id == "binding-2"
            )
    ));
}

#[tokio::test]
pub async fn test_agent_session_repository_workflow起動由来sessionをsession起動要求で再armしない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let caller_request_id = "launch-origin-collision";
    let session_id = releashd::test_support::integration::sessions::launch_resource_id(
        "agent-session",
        caller_request_id,
    )
    .unwrap();
    let lifecycle = |binding: &str| {
        let scope = ProviderLifecycleScope::new(&session_id).unwrap();
        vec![ScopedProviderLifecycleEvent::new(
            scope.clone(),
            ProviderLifecycleEvent::binding_armed(
                "launch-origin-slot",
                binding,
                ProviderKind::Codex,
                scope,
            )
            .unwrap(),
        )]
    };
    let workflow_session = AgentSession::create(
        &session_id,
        WorkspaceIdentity::new("/repo"),
        "/repo",
        ProviderKind::Codex,
        workflow_location("workflow-1", "workflow-node-1"),
    )
    .unwrap();
    repository
        .create_with_lifecycle_events(workflow_session, lifecycle("binding-1"), caller_request_id)
        .await
        .unwrap();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "work",
            worktree_path: "/repo",
            provider: ProviderKind::Codex,
            workflow_execution_id: "workflow-1",
            node_execution_id: "workflow-node-1",
            session_id: &session_id,
            initial_instruction_admitted: false,
        },
    )
    .await
    .unwrap();

    let error = repository
        .create_with_lifecycle_events(
            standalone_session(&session_id, "/repo", ProviderKind::Codex),
            lifecycle("binding-2"),
            caller_request_id,
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        releashd::test_support::integration::sessions::AgentSessionRepositoryError::Conflict
    );
    let stream = store
        .load_stream(
            releashd::test_support::integration::platform::LoadStreamRequest {
                stream_id:
                    releashd::test_support::integration::platform::StreamId::provider_lifecycle(
                        &session_id,
                    )
                    .unwrap(),
                after: None,
                limit: 16,
            },
        )
        .await
        .unwrap();
    assert_eq!(stream.events.len(), 1);
}

#[tokio::test]
pub async fn test_agent_session_repository_workflow起動由来sessionのtree所在不一致を再armしない() {
    for (requested_tree_id, requested_node_execution_id) in [
        ("workflow-2", "workflow-node-1"),
        ("workflow-1", "workflow-node-2"),
    ] {
        let directory = TempDir::new().unwrap();
        let store = open_store(&directory);
        let repository = new_repository(&store);
        let session_id = "workflow-location-rearm";
        let lifecycle = |binding: &str| {
            let scope = ProviderLifecycleScope::new(session_id).unwrap();
            vec![ScopedProviderLifecycleEvent::new(
                scope.clone(),
                ProviderLifecycleEvent::binding_armed(
                    "workflow-location-slot",
                    binding,
                    ProviderKind::Codex,
                    scope,
                )
                .unwrap(),
            )]
        };
        let existing = AgentSession::create(
            session_id,
            WorkspaceIdentity::new("/repo"),
            "/repo",
            ProviderKind::Codex,
            workflow_location("workflow-1", "workflow-node-1"),
        )
        .unwrap();
        repository
            .create_with_lifecycle_events(
                existing,
                lifecycle("binding-1"),
                "workflow-location-request",
            )
            .await
            .unwrap();
        seed_workflow_session_facts(
            &store,
            WorkflowSessionFactSeed {
                workflow_name: "workflow",
                request: "work",
                worktree_path: "/repo",
                provider: ProviderKind::Codex,
                workflow_execution_id: "workflow-1",
                node_execution_id: "workflow-node-1",
                session_id,
                initial_instruction_admitted: false,
            },
        )
        .await
        .unwrap();
        let requested = AgentSession::create(
            session_id,
            WorkspaceIdentity::new("/repo"),
            "/repo",
            ProviderKind::Codex,
            workflow_location(requested_tree_id, requested_node_execution_id),
        )
        .unwrap();

        let error = repository
            .create_with_lifecycle_events(
                requested,
                lifecycle("binding-2"),
                "workflow-location-request",
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            releashd::test_support::integration::sessions::AgentSessionRepositoryError::Conflict
        );
        let stream = store
            .load_stream(
                releashd::test_support::integration::platform::LoadStreamRequest {
                    stream_id:
                        releashd::test_support::integration::platform::StreamId::provider_lifecycle(
                            session_id,
                        )
                        .unwrap(),
                    after: None,
                    limit: 16,
                },
            )
            .await
            .unwrap();
        assert_eq!(stream.events.len(), 1);
    }
}

#[tokio::test]
pub async fn test_エージェントセッション読取_idで一件の表示モデルを返す() {
    // Given
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            standalone_session(
                "agent-session-detail",
                "/repo/worktree",
                ProviderKind::Claude,
            ),
            "create-detail",
        )
        .await
        .unwrap();
    let query_service = LocalAgentSessionQueryService::new(store.clone());

    // When
    let detail = query_service
        .get("agent-session-detail")
        .await
        .unwrap()
        .unwrap();
    let repeated_detail = query_service
        .get("agent-session-detail")
        .await
        .unwrap()
        .unwrap();

    // Then
    assert_eq!(detail.id, "agent-session-detail");
    assert_eq!(repeated_detail, detail);
    assert_eq!(detail.workspace_identity, "/repo/worktree");
    assert_eq!(detail.worktree_path, "/repo/worktree");
    assert_eq!(
        detail.provider,
        releashd::test_support::integration::platform::AgentSessionProviderDto::Claude
    );
    assert_eq!(detail.tree_location.tree_id, "agent-session-detail");
    assert_eq!(
        detail.tree_location.node_execution_id,
        "agent-session-detail"
    );
    assert_eq!(detail.lifecycle, AgentSessionLifecycleDto::Open);
    assert_eq!(detail.provider_session_id, None);
    assert_eq!(detail.transcript_ref, None);
    assert!(!detail.last_exit_abnormal);
    assert!(detail.operations.can_archive);
    assert!(!detail.operations.can_restore);
    assert!(!detail.operations.can_delete);
}

#[tokio::test]
pub async fn test_agent_session_repository_workflow子sessionの事実は元nodeのattemptを保持する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let meta = releashd::test_support::integration::workflow::NodeFactMeta {
        tree_id: "workflow-attempt".to_string(),
        node_execution_id: "session-attempt-3".to_string(),
        parent_id: None,
        node_name: "session".to_string(),
        kind: NodeKindName::Session,
        attempt: 3,
    };
    let root =
        NodeFact::Started(releashd::test_support::integration::workflow::StartedFact {
            worktree: None,
            parent: None,
            root: Some(Box::new(TreeRootFact {
                repository_root: None,
                workspace_identity: "/repo".to_string(),
                worktree_path: "/repo".to_string(),
                created_from: ExecutionOrigin::DesktopUi,
                request: String::new(),
                workflow_name: "workflow".to_string(),
                definition: Some(
                    releashd::test_support::integration::workflow::WorkflowDefinition {
                        name: "workflow".to_string(),
                        description: String::new(),
                        builtin: false,
                        schemas: Default::default(),
                        nodes: vec![releashd::test_support::integration::workflow::NodeDefinition {
                    name: "session".to_string(),
                    kind: releashd::test_support::integration::workflow::NodeKind::Session(
                        releashd::test_support::integration::workflow::SessionSpec {
                            provider: ProviderKind::Codex,
                            model: None,
                            permission: None,
                            facets: Default::default(),
                        },
                    ),
                    ..Default::default()
                }],
                        entry: "session".to_string(),
                    },
                ),
                launched_as: ExecutionTreeLaunch::Workflow,
            })),
        });
    releashd::test_support::integration::workflow::append_single_fact(&store, &meta, &root, 1)
        .await
        .unwrap();
    releashd::test_support::integration::workflow::append_single_fact(
        &store,
        &meta,
        &NodeFact::SessionAttached(
            releashd::test_support::integration::workflow::SessionAttachedFact {
                session_id: "workflow-session".to_string(),
                provider_session_id: None,
                transcript_ref: None,
                initial_instruction_admitted: true,
            },
        ),
        2,
    )
    .await
    .unwrap();
    let mut session = repository.find("workflow-session").await.unwrap().unwrap();
    session
        .session_mut()
        .associate_provider_session("provider-session", None)
        .unwrap();

    repository
        .save(session, "associate-attempt-3")
        .await
        .unwrap();

    let records = releashd::test_support::integration::workflow::read_tree_records(
        &store,
        "workflow-attempt",
    )
    .await
    .unwrap();
    assert_eq!(records.last().unwrap().meta.attempt, 3);
}

#[tokio::test]
pub async fn test_agent_session読取_未対応node定義があってもqueryと操作用repositoryを利用できる() {
    // Given
    for unavailable in ["main", "session", "command", "unused"] {
        let directory = TempDir::new().unwrap();
        let store = open_store(&directory);
        releashd::test_support::integration::workflow::seed_unavailable_definition(
            &store,
            "tree",
            "/repo",
            unavailable,
        )
        .await;
        let repository = new_repository(&store);
        let query = LocalAgentSessionQueryService::new(store.clone());
        // When
        let session = repository.find("tree-session").await.unwrap().unwrap();
        let activity = repository
            .find_for_activity("tree-session")
            .await
            .unwrap()
            .unwrap();
        let item = query.get("tree-session").await.unwrap().unwrap();
        let candidates = repository
            .list_open_for_provider_session_title()
            .await
            .unwrap();

        // Then
        assert_eq!(session.session().worktree_path(), "/repo");
        assert_eq!(item.worktree_path, session.session().worktree_path());
        assert_eq!(item.id, "tree-session");
        assert_eq!(session.session(), activity.session());
        assert_eq!(session.session().lifecycle(), AgentSessionLifecycle::Open);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session(), session.session());
    }
}

#[tokio::test]
pub async fn test_agent_session_repository_所属repoの取得失敗では作成事実を書かない() {
    // Given
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let bare = directory.path().join("bare");
    git2::Repository::init_bare(&bare).unwrap();
    let session = standalone_session("session-bare", bare.to_str().unwrap(), ProviderKind::Codex);
    // When
    let result = repository.create(session, "create-bare").await;
    // Then
    assert_eq!(
        result,
        Err(AgentSessionRepositoryError::Store(
            releashd::test_support::integration::repository::RepositoryError::Rule(
                "bare repository".into()
            )
            .into()
        ))
    );
    assert!(
        releashd::test_support::integration::workflow::read_tree_records(&store, "session-bare")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
pub async fn test_review文脈読取_書き込み側storeを保持したままsessionを復元する() {
    // Given
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            standalone_session("agent-session-1", "/repo", ProviderKind::Codex),
            "create-request-1",
        )
        .await
        .unwrap();
    let context = releashd::test_support::integration::platform::ReviewContextUsecase::new(
        Arc::new(new_repository(&store)),
        Arc::new(
            releashd::test_support::integration::workflow::StoredWorkspaceWorktreePathQuery::new(
                directory.path().to_path_buf(),
                Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
            ),
        ),
    );
    // When
    let resolved = context.session_for_read("agent-session-1").await.unwrap();
    let missing = context.session_for_read("missing").await.unwrap();
    // Then
    assert_eq!(
        resolved,
        Some((
            "/repo".into(),
            releashd::test_support::integration::platform::ReviewActor::provider_agent(
                "codex".into(),
                Some("agent-session-1".into())
            )
        ))
    );
    assert!(missing.is_none());
    assert!(repository.find("agent-session-1").await.unwrap().is_some());
}
