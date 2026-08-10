use std::sync::Arc;

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::{LocalProviderAgentSessionQueryService, LocalProviderAgentSessionRepository};
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionOrigin};
use crate::domain::agent_session::repository::ProviderAgentSessionRepository;
use crate::domain::agent_session::ProviderAgentSessionOwnershipQuery;
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::{
    ProviderKind, ProviderLifecycleEvent, ProviderLifecycleScope, ScopedProviderLifecycleEvent,
};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::{
    ProviderAgentSessionLifecycleDto, ProviderAgentSessionListRequest,
    ProviderAgentSessionOriginFilter, ProviderAgentSessionQueryService,
};
use crate::usecase::provider_lifecycle::ProviderSessionStartTransaction;

fn open_store(directory: &TempDir) -> Arc<LocalEventStore> {
    LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap()
}

fn new_repository(store: &Arc<LocalEventStore>) -> LocalProviderAgentSessionRepository {
    LocalProviderAgentSessionRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    )
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
async fn test_provider_agent_session_repository_create後に再起動して同じ集約を復元できる() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();

    let saved = repository
        .create(session, "create-request-1")
        .await
        .unwrap();

    assert_eq!(saved.revision(), 1);
    assert!(saved.session().uncommitted_events().is_empty());
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
    assert_eq!(loaded.session().workspace().as_str(), "/repo");
    assert_eq!(loaded.session().worktree_path(), "/repo/.worktrees/feature");
    assert_eq!(loaded.session().provider(), ProviderKind::Codex);
    assert_eq!(loaded.session().origin(), &AgentSessionOrigin::Standalone);
    assert!(loaded.session().uncommitted_events().is_empty());
}

#[tokio::test]
async fn test_provider_agent_session_repository_workflow生成時の初回指示admissionを同時に永続化する(
) {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let mut session = AgentSession::create(
        "agent-session-workflow",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        AgentSessionOrigin::workflow_node("workflow-1", "node-execution-1").unwrap(),
    )
    .unwrap();
    session.admit_initial_instruction().unwrap();

    let saved = repository
        .create_with_lifecycle_events(session, Vec::new(), "create-workflow-request-1")
        .await
        .unwrap();

    assert_eq!(saved.revision(), 2);
    assert!(saved.session().initial_instruction_admitted());
    let loaded = repository
        .find("agent-session-workflow")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.revision(), 2);
    assert!(loaded.session().initial_instruction_admitted());
}

#[tokio::test]
async fn test_provider_agent_session_repository_provider紐付けと状態遷移を永続化する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
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
    assert_eq!(
        loaded.session().lifecycle(),
        crate::domain::agent_session::aggregates::AgentSessionLifecycle::Paused
    );
}

#[tokio::test]
async fn test_provider_agent_session_repository同じprovider_session_idの同時所有を原子的に拒否する()
{
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = Arc::new(new_repository(&store));
    let first = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/first",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    let second = AgentSession::create(
        "agent-session-2",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/second",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
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
    assert_eq!(
        loser,
        crate::domain::agent_session::repository::ProviderAgentSessionRepositoryError::ProviderSessionAlreadyOwned {
            agent_session_id: winner.session().id().to_string(),
        }
    );
    let loser_id = if winner.session().id() == "agent-session-1" {
        "agent-session-2"
    } else {
        "agent-session-1"
    };
    assert_eq!(
        repository
            .find(loser_id)
            .await
            .unwrap()
            .unwrap()
            .session()
            .provider_session_id(),
        None
    );
}

#[tokio::test]
async fn test_provider_agent_session_repository削除をtombstone化してprovider所有権を解放する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let first = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/first",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
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
    assert!(!repository
        .is_owned(ProviderKind::Claude, "provider-session-1")
        .await
        .unwrap());
    let page = store
        .load_stream(crate::domain::local_event::LoadStreamRequest {
            stream_id: crate::domain::local_event::StreamId::agent_session("agent-session-1")
                .unwrap(),
            after: None,
            limit: 16,
        })
        .await
        .unwrap();
    assert_eq!(page.events.len(), 1);
    assert!(page.events.iter().all(|event| {
        matches!(
            &event.event,
            crate::domain::local_event::LoadedDomainEvent::Known(event)
                if matches!(
                    event.as_ref(),
                    crate::domain::local_event::LocalDomainEvent::AgentSessionLifecycle(
                        crate::domain::agent_session::aggregates::AgentSessionLifecycleEvent::Tombstoned { .. }
                    )
                )
        )
    }));
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

    let second = AgentSession::create(
        "agent-session-2",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/second",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
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
async fn test_provider_agent_session_repository永続化失敗時にprojectionも所有権も進めない() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
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
        crate::domain::agent_session::repository::ProviderAgentSessionRepositoryError::Unavailable
    );
    let unchanged = repository.find("agent-session-1").await.unwrap().unwrap();
    assert_eq!(unchanged.revision(), 1);
    assert_eq!(unchanged.session().provider_session_id(), None);

    let second = AgentSession::create(
        "agent-session-2",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/second",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    let mut second = repository.create(second, "create-request-2").await.unwrap();
    second
        .session_mut()
        .associate_provider_session("provider-session-1", None)
        .unwrap();
    assert!(repository.save(second, "associate-request-2").await.is_ok());
}

#[tokio::test]
async fn test_provider_agent_session_repository_session_startをlifecycleと原子的に永続化する() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    let session = AgentSession::create(
        "agent-session-atomic",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
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
        crate::domain::agent_session::repository::ProviderAgentSessionRepositoryError::Unavailable
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
async fn test_provider_agent_session_query_service同じworkspaceだけをbounded_pageで返す() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    for (id, workspace, worktree) in [
        ("agent-session-1", "/repo", "/repo/.worktrees/first"),
        ("agent-session-2", "/repo", "/repo/.worktrees/second"),
        ("agent-session-3", "/other", "/other/.worktrees/third"),
    ] {
        let session = AgentSession::create(
            id,
            WorkspaceIdentity::new(workspace),
            worktree,
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
        )
        .unwrap();
        repository
            .create(session, &format!("create-{id}"))
            .await
            .unwrap();
    }
    let query_service = LocalProviderAgentSessionQueryService::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>
    );

    let first = query_service
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: None,
            origin: None,
            limit: 1,
            after_session_id: None,
        })
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, "agent-session-1");
    assert_eq!(
        first.next_after_session_id.as_deref(),
        Some("agent-session-1")
    );

    let second = query_service
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: None,
            origin: None,
            limit: 1,
            after_session_id: first.next_after_session_id,
        })
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].id, "agent-session-2");
    assert_eq!(second.next_after_session_id, None);
}

#[tokio::test]
async fn test_provider_agent_session_query_service_idで一件の表示モデルを返す() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            AgentSession::create(
                "agent-session-detail",
                WorkspaceIdentity::new("/repo"),
                "/repo/worktree",
                ProviderKind::Claude,
                AgentSessionOrigin::Standalone,
            )
            .unwrap(),
            "create-detail",
        )
        .await
        .unwrap();
    let query_service = LocalProviderAgentSessionQueryService::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>
    );

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
        crate::usecase::agent_session::ProviderAgentSessionProviderDto::Claude
    );
    assert!(detail.operations.can_archive);
    assert!(!detail.operations.can_restore);
    assert!(!detail.operations.can_delete);
}

#[tokio::test]
async fn test_provider_agent_session_query_service_lifecycleをdata_source側で絞り込む() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    for id in ["open-session", "archived-session"] {
        let session = AgentSession::create(
            id,
            WorkspaceIdentity::new("/repo"),
            format!("/repo/.worktrees/{id}"),
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
        )
        .unwrap();
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
    let query_service = LocalProviderAgentSessionQueryService::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>
    );

    let active_page = query_service
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: None,
            origin: None,
            limit: 10,
            after_session_id: None,
        })
        .await
        .unwrap();
    assert_eq!(active_page.items.len(), 1);
    assert_eq!(active_page.items[0].id, "open-session");

    let page = query_service
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: Some(ProviderAgentSessionLifecycleDto::Archived),
            origin: None,
            limit: 10,
            after_session_id: None,
        })
        .await
        .unwrap();

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, "archived-session");
    assert_eq!(
        page.items[0].lifecycle,
        ProviderAgentSessionLifecycleDto::Archived
    );
    assert!(!page.items[0].operations.can_archive);
    assert!(page.items[0].operations.can_restore);
    assert!(page.items[0].operations.can_delete);
}

#[tokio::test]
async fn test_provider_agent_session_query_service_standaloneをdata_source側で絞り込む() {
    let directory = TempDir::new().unwrap();
    let store = open_store(&directory);
    let repository = new_repository(&store);
    repository
        .create(
            AgentSession::create(
                "standalone-session",
                WorkspaceIdentity::new("/repo"),
                "/repo/worktree",
                ProviderKind::Claude,
                AgentSessionOrigin::Standalone,
            )
            .unwrap(),
            "create-standalone",
        )
        .await
        .unwrap();
    repository
        .create(
            AgentSession::create(
                "workflow-session",
                WorkspaceIdentity::new("/repo"),
                "/repo/worktree",
                ProviderKind::Codex,
                AgentSessionOrigin::workflow_node("workflow-1", "node-1").unwrap(),
            )
            .unwrap(),
            "create-workflow",
        )
        .await
        .unwrap();
    let query_service = LocalProviderAgentSessionQueryService::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>
    );

    let page = query_service
        .list(ProviderAgentSessionListRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            lifecycle: None,
            origin: Some(ProviderAgentSessionOriginFilter::Standalone),
            limit: 10,
            after_session_id: None,
        })
        .await
        .unwrap();

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, "standalone-session");
}
