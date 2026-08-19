use std::sync::Arc;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::{workspace_session_items, LocalAgentSessionQueryService, LocalAgentSessionRepository};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::fact_log;
use crate::domain::agent_session::aggregates::{
    AgentSession, AgentSessionLifecycle, AgentSessionTreeParent,
};
use crate::domain::agent_session::repository::AgentSessionRepository;
use crate::domain::agent_session::AgentSessionOwnershipQuery;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
};
use crate::domain::workflow::{
    ExecutionOrigin, NodeFact, NodeKindName, SessionRootFact, TreeRootFact,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{AgentSessionLifecycleDto, AgentSessionQueryService};
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

fn parentless_session(id: &str, worktree_path: &str, provider: ProviderKind) -> AgentSession {
    AgentSession::create(
        id,
        WorkspaceIdentity::new(worktree_path),
        worktree_path,
        provider,
        None,
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
    let session = parentless_session(
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
    assert_eq!(records.len(), 1);
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
    let Some(TreeRootFact::Session(SessionRootFact {
        workspace_identity,
        worktree_path,
        session,
        created_from,
    })) = &started.root
    else {
        panic!("session root fact must carry the session tree root: {started:?}");
    };
    assert_eq!(workspace_identity, "/repo/.worktrees/feature");
    assert_eq!(worktree_path, "/repo/.worktrees/feature");
    assert_eq!(session.provider, ProviderKind::Codex);
    assert_eq!(*created_from, ExecutionOrigin::DesktopUi);
    drop(repository);
    drop(store);

    let reopened = open_store(&directory);
    let loaded = new_repository(&reopened)
        .find("agent-session-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.revision(), 1);
    assert_eq!(loaded.session().id(), "agent-session-1");
    assert_eq!(
        loaded.session().workspace().as_str(),
        "/repo/.worktrees/feature"
    );
    assert_eq!(loaded.session().worktree_path(), "/repo/.worktrees/feature");
    assert_eq!(loaded.session().provider(), ProviderKind::Codex);
    assert_eq!(loaded.session().tree_parent(), None);
    assert_eq!(loaded.session().lifecycle(), AgentSessionLifecycle::Open);
    assert!(loaded.session().uncommitted_events().is_empty());
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
        None,
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
            parentless_session("agent-session-1", "/repo", ProviderKind::Codex),
            "create-request-1",
        )
        .await
        .unwrap();

    let error = repository
        .create(
            parentless_session("agent-session-1", "/repo", ProviderKind::Codex),
            "create-request-2",
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        crate::domain::agent_session::repository::AgentSessionRepositoryError::Conflict
    );
    assert_eq!(tree_event_types(&store, "agent-session-1"), ["started"]);
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
        Some(AgentSessionTreeParent::new("workflow-1", "node-execution-1").unwrap()),
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
            Some(AgentSessionTreeParent::new("workflow-1", "node-execution-1").unwrap()),
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
    let session = parentless_session(
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
        ["started", "session_attached", "process_exited"]
    );
    drop(repository);
    drop(store);

    let reopened = open_store(&directory);
    let loaded = new_repository(&reopened)
        .find("agent-session-1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.revision(), 3);
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
async fn test_agent_session_repository_異常exitをfailure付きprocess_exitedとして記録する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            parentless_session("agent-session-abnormal", "/repo", ProviderKind::Codex),
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
async fn test_agent_session_repository_resumeとarchiveとrestoreを行として記録し導出する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut saved = repository
        .create(
            parentless_session("agent-session-flow", "/repo", ProviderKind::Claude),
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
    let first = parentless_session(
        "agent-session-1",
        "/repo/.worktrees/first",
        ProviderKind::Codex,
    );
    let second = parentless_session(
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
    let first = parentless_session(
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

    let second = parentless_session(
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
async fn test_agent_session_repository永続化失敗時に所有権も導出状態も進めない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = parentless_session(
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
    assert_eq!(unchanged.revision(), 1);
    assert_eq!(unchanged.session().provider_session_id(), None);

    let second = parentless_session(
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
    let session = parentless_session(
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
    let session = parentless_session(
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
            parentless_session(
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
async fn test_agent_session_repository_同じlaunch要求を再起動後に再armする() {
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

    repository
        .create_with_lifecycle_events(
            parentless_session(&session_id, "/repo", ProviderKind::Codex),
            lifecycle("binding-1"),
            caller_request_id,
        )
        .await
        .unwrap();
    repository
        .create_with_lifecycle_events(
            parentless_session(&session_id, "/repo", ProviderKind::Codex),
            lifecycle("binding-2"),
            caller_request_id,
        )
        .await
        .unwrap();

    assert_eq!(tree_event_types(&store, &session_id), ["started"]);
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
                parentless_session(id, worktree, ProviderKind::Codex),
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
            parentless_session(
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
    assert_eq!(detail.worktree_path, "/repo/worktree");
    assert_eq!(
        detail.provider,
        crate::usecase::agent_session::AgentSessionProviderDto::Claude
    );
    assert_eq!(detail.tree_parent, None);
    assert!(detail.operations.can_archive);
    assert!(!detail.operations.can_restore);
    assert!(!detail.operations.can_delete);
}

#[tokio::test]
async fn test_workspace共通read_modelは全lifecycleを返す() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    for id in ["open-session", "archived-session"] {
        let session = parentless_session(id, "/repo", ProviderKind::Claude);
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
        if id == "archived-session" {
            saved.session_mut().archive().unwrap();
            repository.save(saved, "archive-session").await.unwrap();
        }
    }
    let items = workspace_session_items(
        &fact_log::FactLogReadBackend::Live(store),
        &["open-session".to_string(), "archived-session".to_string()],
        "/repo",
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    let archived = items
        .iter()
        .find(|item| item.id == "archived-session")
        .unwrap();
    assert_eq!(archived.lifecycle, AgentSessionLifecycleDto::Archived);
    assert!(!archived.operations.can_archive);
    assert!(archived.operations.can_restore);
    assert!(archived.operations.can_delete);
}

#[tokio::test]
async fn test_agent_session_query_service_workflow木のsessionを一覧に出さない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            parentless_session("standalone-session", "/repo", ProviderKind::Claude),
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
        root: Some(TreeRootFact::Workflow(
            crate::domain::workflow::WorkflowRootFact {
                workflow_name: "wf".to_string(),
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
            },
        )),
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
        root: Some(TreeRootFact::Workflow(
            crate::domain::workflow::WorkflowRootFact {
                workflow_name: "workflow".to_string(),
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
            },
        )),
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
