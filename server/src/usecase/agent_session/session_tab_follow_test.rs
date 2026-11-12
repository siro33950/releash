use super::super::*;
use crate::domain::agent_session::services::AgentSessionService;
use crate::usecase::agent_session::test_helpers::*;
use crate::usecase::workspace_state::test_support::MemoryWorkspaceStateRepository;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn test_session保存_provider関連付けだけならタブ配置を読まない() {
    // Given
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        session_location("session"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(session));
    repository.save_success.store(true, Ordering::SeqCst);
    let tabs = Arc::new(MemoryWorkspaceStateRepository::default());
    let usecase = AgentSessionUsecase::new(
        repository.clone(),
        Arc::new(FakeThreadRepository::default()),
        AgentSessionService::new(
            Arc::new(RecordingTerminal::default()),
            Arc::new(RecordingTerminalInput::default()),
        ),
        SessionTabsUsecase::new(
            tabs.clone(),
            crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
            crate::test_support::state_subscription::test_subscriptions(),
        ),
    );
    let mut changed = repository.snapshot().session().clone();
    changed
        .associate_provider_session("provider", None)
        .unwrap();
    // When
    let saved = usecase
        .save_if_changed(VersionedAgentSession::restored(changed, 1), "associate")
        .await
        .unwrap();
    // Then
    assert_eq!(saved.session().provider_session_id(), Some("provider"));
    assert_eq!(saved.revision(), 2);
    assert_eq!(tabs.read_calls.load(Ordering::SeqCst), 0);
}
