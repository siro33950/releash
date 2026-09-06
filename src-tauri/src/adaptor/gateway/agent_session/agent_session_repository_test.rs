use std::sync::Arc;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::agent_session_repository::{
    map_commit_batch_error, open_session_title_candidates, OPEN_SESSION_LIFECYCLE_EVENT_TYPES,
};
use super::{workspace_session_items, LocalAgentSessionQueryService, LocalAgentSessionRepository};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::fact_log;
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycle, AgentSessionTreeLocation,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError,
};
use crate::domain::agent_session::AgentSessionOwnershipQuery;
use crate::domain::local_event::CommitBatchError;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
};
use crate::domain::workflow::{
    AgentSessionActivity, ExecutionOrigin, ExecutionTreeLaunch, NodeFact, NodeKindName,
    StopReceivedFact, TreeRootFact,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    AgentSessionLifecycleDto, AgentSessionQueryService, AgentSessionUsecase,
};
use crate::usecase::provider_lifecycle::ProviderSessionStartTransaction;

fn open_store(directory: &TempDir) -> Arc<LocalEventStore> {
    LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap()
}

fn new_repository(store: &Arc<LocalEventStore>) -> LocalAgentSessionRepository {
    LocalAgentSessionRepository::new(store.clone())
}

fn session_location(id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::session_tree_root(id).unwrap()
}

fn workflow_location(tree_id: &str, node_execution_id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::workflow_node(tree_id, node_execution_id).unwrap()
}

#[test]
fn test_agent_session_repository_commit_errorの非競合障害をcorruptに分類する() {
    for error in [
        CommitBatchError::CapacityExceeded,
        CommitBatchError::SequenceExhausted,
        CommitBatchError::Corrupt {
            correlation_id: "corrupt-commit".to_string(),
        },
    ] {
        assert_eq!(
            map_commit_batch_error(error),
            crate::domain::agent_session::repository::AgentSessionRepositoryError::Corrupt
        );
    }
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

fn tree_event_types(store: &Arc<LocalEventStore>, tree_id: &str) -> Vec<&'static str> {
    fact_log::read_tree_records(store, tree_id)
        .unwrap()
        .iter()
        .map(|record| record.fact.event_type())
        .collect()
}

fn ownership_stream(
    provider: ProviderKind,
    provider_session_id: &str,
) -> crate::domain::local_event::StreamId {
    let provider = match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    };
    let digest = hex::encode(Sha256::digest(provider_session_id.as_bytes()));
    crate::domain::local_event::StreamId::provider_session_ownership(provider, &digest).unwrap()
}

#[tokio::test]
async fn test_agent_session_repository_単独session作成をnode_eventsへ記録し再起動後のfindで同じ状態を導出する(
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
    let records = fact_log::read_tree_records(&store, "agent-session-1").unwrap();
    assert_eq!(records.len(), 2);
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
        .node_by_name("session")
        .and_then(crate::domain::workflow::NodeDefinition::session)
        .unwrap();
    assert_eq!(session.provider, ProviderKind::Codex);
    assert!(matches!(
        &records[1].fact,
        NodeFact::SessionAttached(attached)
            if attached.session_id == "agent-session-1"
                && attached.provider_session_id.is_none()
                && attached.transcript_ref.is_none()
    ));
    drop(repository);
    drop(store);

    let reopened = open_store(&directory);
    let loaded = new_repository(&reopened)
        .find("agent-session-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.revision(), 2);
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
async fn test_agent_session_repository_活動遷移だけをnode行へ追記し同値観測は追記しない() {
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::AlreadyApplied
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::AlreadyApplied
    );

    // Then: 遷移だけが事実として同じ Session Node へ追記される
    let records = fact_log::read_tree_records(&store, "agent-session-activity").unwrap();
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
async fn test_agent_session_repository_process_exit後のworking再観測を活動遷移として追記する() {
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
    );
    sessions
        .observe_process_exit(
            "agent-session-activity-exit",
            Some(0),
            "activity-process-exit",
        )
        .await
        .unwrap();
    let before = fact_log::read_tree_records(&store, "agent-session-activity-exit").unwrap();
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
    );
    let after = fact_log::read_tree_records(&store, "agent-session-activity-exit").unwrap();
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
async fn test_agent_session_repository_stop事実後のworking再観測をbounded_readから活動遷移として追記する(
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
    let records = fact_log::read_tree_records(&store, session_id).unwrap();
    let meta = records.last().unwrap().meta.clone();
    fact_log::append_single_fact(
        &store,
        &meta,
        &NodeFact::StopReceived(StopReceivedFact {
            result_summary: None,
            token_usage: None,
        }),
        10,
    )
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
        crate::domain::agent_session::aggregates::AgentSessionMutationOutcome::Applied
    );
    let records = fact_log::read_tree_records(&store, session_id).unwrap();
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
async fn test_agent_session_repository_workflow子sessionも同じ活動保存経路を使う() {
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

    let records = fact_log::read_tree_records(&store, "workflow-activity").unwrap();
    assert!(matches!(
        &records.last().unwrap().fact,
        NodeFact::AgentActivityObserved(fact)
            if fact.activity == AgentSessionActivity::Working
                && records.last().unwrap().meta.node_execution_id == "workflow-session-node"
    ));
}

#[tokio::test]
async fn test_agent_session_repository_workspace同定子はworktreeと独立に往復する() {
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
async fn test_agent_session_repository_同一idの再createを拒否する() {
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
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict
    );
    assert_eq!(
        tree_event_types(&store, "agent-session-1"),
        ["started", "session_attached"]
    );
}

#[tokio::test]
async fn test_agent_session_repository_workflow子sessionのcreateは木に行を追加しない() {
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
    assert!(fact_log::read_tree_records(&store, "workflow-1")
        .unwrap()
        .is_empty());
    assert!(
        fact_log::read_tree_records(&store, "agent-session-workflow")
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_agent_session_repository_attach前に再起動したworkflow子sessionを再armする() {
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
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: crate::domain::local_event::StreamId::provider_lifecycle(
                "agent-session-workflow-rearm",
            )
            .unwrap(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(stream.events.len(), 2);
    assert!(fact_log::read_tree_records(&store, "workflow-1")
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn test_agent_session_repository_provider紐付けと状態遷移を事実行として永続化する() {
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
        tree_event_types(&store, "agent-session-1"),
        [
            "started",
            "session_attached",
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

    assert_eq!(loaded.revision(), 4);
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
async fn test_agent_session_repository_openかつprovider_session確定済みのsessionだけを軽量列挙する()
{
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
    let lifecycle_records = fact_log::read_records_for_event_types(
        &fact_log::FactLogReadBackend::Live(Arc::clone(&store)),
        OPEN_SESSION_LIFECYCLE_EVENT_TYPES,
    )
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
    assert_eq!(candidate.location.tree_id, "open-session");
    assert_eq!(candidate.location.node_execution_id, "open-session");
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
    assert_eq!(sessions[0].revision(), 5);
}

#[tokio::test]
async fn test_agent_session_repository_renameとproviderタイトルを対応する事実へ保存し再取得する() {
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
        tree_event_types(&store, "named-session"),
        [
            "started",
            "session_attached",
            "session_attached",
            "session_node_renamed",
            "provider_session_title_observed",
        ]
    );
}

#[tokio::test]
async fn test_agent_session_repository_providerタイトル軽量保存は他のeventが混ざる要求を拒否する() {
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
    let rows_before = tree_event_types(&store, "invalid-title-save");

    let result = repository
        .save_provider_session_title(saved, "invalid-title-observation")
        .await;

    assert_eq!(result, Err(AgentSessionRepositoryError::InvalidRequest));
    assert_eq!(tree_event_types(&store, "invalid-title-save"), rows_before);
}

#[tokio::test]
async fn test_agent_session_repository_異常exitをfailure付きprocess_exitedとして記録する() {
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

    let records = fact_log::read_tree_records(&store, "agent-session-abnormal").unwrap();
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
async fn test_agent_session_repository_異常終了したsession起動木をresumeする() {
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
    let failed = fact_log::fold_tree_from(
        &fact_log::FactLogReadBackend::Live(store.clone()),
        "agent-session-abnormal-resume",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        failed
            .aggregate
            .node_execution("agent-session-abnormal-resume")
            .unwrap()
            .status,
        crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Failed
    );

    saved
        .session_mut()
        .complete_resume(
            crate::domain::agent_session::aggregates::AgentSessionRecoveryResult::Succeeded,
        )
        .unwrap();
    repository
        .save(saved, "resume-after-abnormal-exit")
        .await
        .unwrap();

    let resumed = fact_log::fold_tree_from(
        &fact_log::FactLogReadBackend::Live(store),
        "agent-session-abnormal-resume",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        resumed
            .aggregate
            .node_execution("agent-session-abnormal-resume")
            .unwrap()
            .status,
        crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Running
    );
}

#[tokio::test]
async fn test_agent_session_repository_restore後の指示待ちを事実から復元する() {
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
    let mut saved = repository
        .save(saved, "archive-working-session")
        .await
        .unwrap();

    saved
        .session_mut()
        .complete_restore(
            crate::domain::agent_session::aggregates::AgentSessionRecoveryResult::Succeeded,
        )
        .unwrap();
    repository
        .save(saved, "restore-working-session")
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
    let activities = fact_log::read_tree_records(&store, "agent-session-restore-activity")
        .unwrap()
        .into_iter()
        .filter_map(|record| match record.fact {
            NodeFact::AgentActivityObserved(fact) => Some(fact.activity),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        activities,
        [
            AgentSessionActivity::Working,
            AgentSessionActivity::AwaitingInstruction,
        ]
    );
}

#[tokio::test]
async fn test_agent_session_repository_resumeとarchiveとrestoreを行として記録し導出する() {
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
            crate::domain::agent_session::aggregates::AgentSessionRecoveryResult::Succeeded,
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
    let mut saved = repository.save(saved, "archive-request-1").await.unwrap();
    let archived = repository
        .find("agent-session-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        archived.session().lifecycle(),
        AgentSessionLifecycle::Archived
    );

    saved
        .session_mut()
        .complete_restore(
            crate::domain::agent_session::aggregates::AgentSessionRecoveryResult::Succeeded,
        )
        .unwrap();
    repository.save(saved, "restore-request-1").await.unwrap();
    let restored = repository
        .find("agent-session-flow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.session().lifecycle(), AgentSessionLifecycle::Open);
    assert_eq!(
        tree_event_types(&store, "agent-session-flow"),
        [
            "started",
            "session_attached",
            "session_attached",
            "process_exited",
            "resume_requested",
            "archive_requested",
            "restore_requested",
        ]
    );
}

#[tokio::test]
async fn test_agent_session_repository同じprovider_session_idの同時所有を原子的に拒否する() {
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
        crate::domain::agent_session::repository::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
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
        crate::domain::agent_session::repository::AgentSessionRepositoryError::ProviderSessionAlreadyOwned {
            agent_session_id: winner.session().id().to_string(),
        }
    );
}

#[tokio::test]
async fn test_agent_session_repository削除で木の行を物理削除しprovider所有権を解放する() {
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
    assert!(fact_log::read_tree_records(&store, "agent-session-1")
        .unwrap()
        .is_empty());
    assert!(!repository
        .is_owned(ProviderKind::Claude, "provider-session-1")
        .await
        .unwrap());
    let ownership_page = store
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: ownership_stream(ProviderKind::Claude, "provider-session-1"),
            after: None,
            limit: 16,
        })
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
async fn test_agent_session_repository削除失敗時に木とprovider所有権を原子的に維持する() {
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

    assert_eq!(
        result.unwrap_err(),
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Unavailable
    );
    let retained = repository
        .find("agent-session-atomic-delete")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !fact_log::read_tree_records(&store, "agent-session-atomic-delete")
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
async fn test_agent_session_repository永続化失敗時に所有権も導出状態も進めない() {
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

    assert_eq!(
        result.unwrap_err(),
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Unavailable
    );
    let unchanged = repository.find("agent-session-1").await.unwrap().unwrap();
    assert_eq!(unchanged.revision(), 2);
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
async fn test_agent_session_repository_session_startをlifecycleと原子的に永続化する() {
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

    assert_eq!(
        failed.unwrap_err(),
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Unavailable
    );
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
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: crate::domain::local_event::StreamId::provider_lifecycle(
                "agent-session-atomic",
            )
            .unwrap(),
            after: None,
            limit: 16,
        })
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
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: crate::domain::local_event::StreamId::provider_lifecycle(
                "agent-session-atomic",
            )
            .unwrap(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(lifecycle_stream.events.len(), 1);
}

#[tokio::test]
async fn test_agent_session_repository_単独rootとprovider_lifecycleを原子的に永続化する() {
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

    assert_eq!(
        failed.unwrap_err(),
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Unavailable
    );
    assert!(repository
        .find("agent-session-create-atomic")
        .await
        .unwrap()
        .is_none());
    let stream_id =
        crate::domain::local_event::StreamId::provider_lifecycle("agent-session-create-atomic")
            .unwrap();
    assert!(store
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: stream_id.clone(),
            after: None,
            limit: 16,
        })
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
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id,
                after: None,
                limit: 16,
            })
            .await
            .unwrap()
            .events
            .len(),
        1
    );
}

#[tokio::test]
async fn test_agent_session_repository_session起動由来の同一要求を既存sessionへ再armする() {
    // Given: caller request から導出した id の Session が provider history と紐付いている
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let caller_request_id = "standalone-restart-request";
    let session_id =
        crate::domain::agent_session::launch_resource_id("agent-session", caller_request_id)
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
        tree_event_types(&store, &session_id),
        ["started", "session_attached", "session_attached"]
    );
    let stream = store
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: crate::domain::local_event::StreamId::provider_lifecycle(&session_id)
                .unwrap(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(stream.events.len(), 2);
    assert!(matches!(
        &stream.events[1].event,
        crate::domain::local_event::LoadedDomainEvent::Known(event)
            if matches!(
                event.as_ref(),
                crate::domain::local_event::LocalDomainEvent::ProviderLifecycle(
                    ProviderLifecycleEvent::BindingArmed { binding_id, .. }
                ) if binding_id == "binding-2"
            )
    ));
}

#[tokio::test]
async fn test_agent_session_repository_workflow起動由来sessionをsession起動要求で再armしない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let caller_request_id = "launch-origin-collision";
    let session_id =
        crate::domain::agent_session::launch_resource_id("agent-session", caller_request_id)
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
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict
    );
    let stream = store
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: crate::domain::local_event::StreamId::provider_lifecycle(&session_id)
                .unwrap(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(stream.events.len(), 1);
}

#[tokio::test]
async fn test_agent_session_repository_workflow起動由来sessionのtree所在不一致を再armしない() {
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
            crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict
        );
        let stream = store
            .load_stream(crate::domain::local_event::LoadStreamRequest {
                stream_id: crate::domain::local_event::StreamId::provider_lifecycle(session_id)
                    .unwrap(),
                after: None,
                limit: 16,
            })
            .await
            .unwrap();
        assert_eq!(stream.events.len(), 1);
    }
}

#[tokio::test]
async fn test_workspace共通read_modelは同じworkspaceのsessionをid昇順で返す() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    for (id, worktree) in [
        ("agent-session-2", "/repo"),
        ("agent-session-1", "/repo"),
        ("agent-session-3", "/other"),
    ] {
        repository
            .create(
                standalone_session(id, worktree, ProviderKind::Codex),
                &format!("create-{id}"),
            )
            .await
            .unwrap();
    }
    let items = workspace_session_items(
        &fact_log::FactLogReadBackend::Live(store),
        &[
            "agent-session-1".to_string(),
            "agent-session-2".to_string(),
            "agent-session-3".to_string(),
        ],
        "/repo",
    )
    .unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["agent-session-1", "agent-session-2"]
    );
}

#[tokio::test]
async fn test_agent_session_query_service_idで一件の表示モデルを返す() {
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

    let detail = query_service
        .get("agent-session-detail")
        .await
        .unwrap()
        .unwrap();
    let blocking_detail = query_service
        .get_blocking("agent-session-detail")
        .unwrap()
        .unwrap();

    assert_eq!(detail.id, "agent-session-detail");
    assert_eq!(blocking_detail, detail);
    assert_eq!(detail.workspace_identity, "/repo/worktree");
    assert_eq!(detail.worktree_path, "/repo/worktree");
    assert_eq!(
        detail.provider,
        crate::usecase::agent_session::AgentSessionProviderDto::Claude
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
    assert!(!detail.operations.can_resume);
}

#[tokio::test]
async fn test_agent_session_query_service_resume可否をprovider参照の有無から返す() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);

    for (execution_id, node_execution_id, session_id, provider_session_id) in [
        ("workflow-unknown", "node-unknown", "session-unknown", None),
        (
            "workflow-known",
            "node-known",
            "session-known",
            Some("provider-known"),
        ),
    ] {
        seed_workflow_session_facts(
            &store,
            WorkflowSessionFactSeed {
                workflow_name: "workflow",
                request: "test",
                worktree_path: "/repo/worktree",
                provider: ProviderKind::Codex,
                workflow_execution_id: execution_id,
                node_execution_id,
                session_id,
                initial_instruction_admitted: true,
            },
        )
        .unwrap();
        let mut saved = repository
            .create(
                AgentSession::create(
                    session_id,
                    WorkspaceIdentity::new("/repo"),
                    "/repo/worktree",
                    ProviderKind::Codex,
                    workflow_location(execution_id, node_execution_id),
                )
                .unwrap(),
                &format!("create-{session_id}"),
            )
            .await
            .unwrap();
        if let Some(provider_session_id) = provider_session_id {
            saved
                .session_mut()
                .associate_provider_session(provider_session_id, None)
                .unwrap();
            saved = repository
                .save(saved, &format!("associate-{session_id}"))
                .await
                .unwrap();
        }
        saved
            .session_mut()
            .stop_for_terminal_execution_tree_node(node_execution_id)
            .unwrap();
        repository
            .save(saved, &format!("stop-{session_id}"))
            .await
            .unwrap();
    }

    let query_service = LocalAgentSessionQueryService::new(store);
    let unknown = query_service.get("session-unknown").await.unwrap().unwrap();
    let known = query_service.get("session-known").await.unwrap().unwrap();

    assert!(!unknown.operations.can_resume);
    assert!(!unknown.operations.can_archive);
    assert!(!unknown.operations.can_restore);
    assert!(!unknown.operations.can_delete);
    assert!(known.operations.can_resume);
    assert!(!known.operations.can_archive);
    assert!(!known.operations.can_restore);
    assert!(!known.operations.can_delete);
}

#[tokio::test]
async fn test_workspace共通read_modelは全lifecycleを返す() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    for id in ["open-session", "paused-session", "archived-session"] {
        let session = standalone_session(id, "/repo", ProviderKind::Claude);
        let mut saved = repository
            .create(session, &format!("create-{id}"))
            .await
            .unwrap();
        saved
            .session_mut()
            .associate_provider_session(format!("provider-{id}"), None)
            .unwrap();
        let mut saved = repository
            .save(saved, &format!("associate-{id}"))
            .await
            .unwrap();
        match id {
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
    let items = workspace_session_items(
        &fact_log::FactLogReadBackend::Live(store),
        &[
            "open-session".to_string(),
            "paused-session".to_string(),
            "archived-session".to_string(),
        ],
        "/repo",
    )
    .unwrap();
    assert_eq!(items.len(), 3);
    let open = items.iter().find(|item| item.id == "open-session").unwrap();
    assert_eq!(open.lifecycle, AgentSessionLifecycleDto::Open);
    assert!(open.operations.can_archive);
    assert!(!open.operations.can_restore);
    assert!(!open.operations.can_delete);
    assert!(!open.operations.can_resume);
    let paused = items
        .iter()
        .find(|item| item.id == "paused-session")
        .unwrap();
    assert_eq!(paused.lifecycle, AgentSessionLifecycleDto::Paused);
    assert!(paused.operations.can_archive);
    assert!(!paused.operations.can_restore);
    assert!(!paused.operations.can_delete);
    assert!(paused.operations.can_resume);
    let archived = items
        .iter()
        .find(|item| item.id == "archived-session")
        .unwrap();
    assert_eq!(archived.lifecycle, AgentSessionLifecycleDto::Archived);
    assert!(!archived.operations.can_archive);
    assert!(archived.operations.can_restore);
    assert!(archived.operations.can_delete);
    assert!(!archived.operations.can_resume);
}

#[tokio::test]
async fn test_agent_session_query_service_workflow木のsessionを一覧に出さない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            standalone_session("standalone-session", "/repo", ProviderKind::Claude),
            "create-standalone",
        )
        .await
        .unwrap();
    // workflow engine が所有する木（同じ worktree に root を植えた workflow tree）。
    let workflow_meta = crate::domain::workflow::NodeFactMeta {
        tree_id: "workflow-1".to_string(),
        node_execution_id: "workflow-1".to_string(),
        parent_id: None,
        node_name: "main".to_string(),
        kind: NodeKindName::Sequence,
        attempt: 1,
    };
    let workflow_root = NodeFact::Started(crate::domain::workflow::StartedFact {
        parent: None,
        root: Some(TreeRootFact {
            definition_resolution: Default::default(),
            workspace_identity: "/repo".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: "please work".to_string(),
            definition: crate::domain::workflow::WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: Vec::new(),
                entry: "main".to_string(),
            },
            launched_as: ExecutionTreeLaunch::Workflow,
        }),
    });
    fact_log::append_single_fact(&store, &workflow_meta, &workflow_root, 1).unwrap();
    let items = workspace_session_items(
        &fact_log::FactLogReadBackend::Live(store),
        &["standalone-session".to_string(), "workflow-1".to_string()],
        "/repo",
    )
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "standalone-session");
}

#[tokio::test]
async fn test_agent_session_repository_workflow子sessionの事実は元nodeのattemptを保持する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let meta = crate::domain::workflow::NodeFactMeta {
        tree_id: "workflow-attempt".to_string(),
        node_execution_id: "session-attempt-3".to_string(),
        parent_id: None,
        node_name: "session".to_string(),
        kind: NodeKindName::Session,
        attempt: 3,
    };
    let root = NodeFact::Started(crate::domain::workflow::StartedFact {
        parent: None,
        root: Some(TreeRootFact {
            definition_resolution: Default::default(),
            workspace_identity: "/repo".to_string(),
            worktree_path: "/repo".to_string(),
            created_from: ExecutionOrigin::DesktopUi,
            request: String::new(),
            definition: crate::domain::workflow::WorkflowDefinition {
                name: "workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![crate::domain::workflow::NodeDefinition {
                    name: "session".to_string(),
                    kind: crate::domain::workflow::NodeKind::Session(
                        crate::domain::workflow::SessionSpec {
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
            launched_as: ExecutionTreeLaunch::Workflow,
        }),
    });
    fact_log::append_single_fact(&store, &meta, &root, 1).unwrap();
    fact_log::append_single_fact(
        &store,
        &meta,
        &NodeFact::SessionAttached(crate::domain::workflow::SessionAttachedFact {
            session_id: "workflow-session".to_string(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: true,
        }),
        2,
    )
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

    let records = fact_log::read_tree_records(&store, "workflow-attempt").unwrap();
    assert_eq!(records.last().unwrap().meta.attempt, 3);
}

#[tokio::test]
async fn test_agent_session読取_未対応node定義があってもqueryと操作用repositoryを利用できる() {
    // Given
    for unavailable in ["main", "session", "command", "unused"] {
        let directory = TempDir::new().unwrap();
        let store = open_store(&directory);
        crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition(
            &store,
            "tree",
            "/repo",
            unavailable,
        );
        let repository = new_repository(&store);
        let query = LocalAgentSessionQueryService::new(store.clone());
        let read_store =
            crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                directory.path(),
            )
            .unwrap();
        let read_query = LocalAgentSessionQueryService::new_read_only(read_store);

        // When
        let session = repository.find("tree-session").await.unwrap().unwrap();
        let activity = repository
            .find_for_activity("tree-session")
            .await
            .unwrap()
            .unwrap();
        let item = query.get("tree-session").await.unwrap().unwrap();
        let read_item = read_query.get("tree-session").await.unwrap().unwrap();
        let candidates = repository
            .list_open_for_provider_session_title()
            .await
            .unwrap();

        // Then
        assert_eq!(session.session().worktree_path(), "/repo");
        assert_eq!(session.session(), activity.session());
        assert_eq!(session.session().lifecycle(), AgentSessionLifecycle::Open);
        assert_eq!(item, read_item);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session(), session.session());
    }
}
