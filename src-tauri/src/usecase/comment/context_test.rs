use super::*;
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::agent_session::test_helpers::session_location;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::test_helpers::FailingSaveRepository;

struct Worktrees;
#[async_trait::async_trait]
impl WorkspaceWorktreePathQuery for Worktrees {
    async fn workspace_worktree_path(
        &self,
        path: &str,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        match path {
            "/isolated" => Ok("/workspace".into()),
            "/missing" => Err(crate::domain::workflow::WorkflowError::NotFound(
                "missing owner".into(),
            )),
            _ => Ok(path.into()),
        }
    }
}

#[tokio::test]
async fn test_review文脈_sessionの書き手とworkspaceを解決し閉じたsessionは読み取りだけ許す() {
    // Given
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/workspace"),
        "/isolated",
        ProviderKind::Codex,
        session_location("session"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(session));
    let context = ReviewContextUsecase::new(repository.clone(), Arc::new(Worktrees));
    // When
    let (path, actor) = context
        .resolve(ReviewContextTarget::Session("session"), true)
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(path, "/workspace");
    assert_eq!(
        actor,
        ReviewActor::provider_agent("codex".into(), Some("session".into()))
    );
    // When
    repository
        .stored
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .session_mut()
        .archive()
        .unwrap();
    // Then
    assert!(context
        .resolve(ReviewContextTarget::Session("session"), false)
        .await
        .unwrap()
        .is_some());
    assert!(matches!(
        context
            .resolve(ReviewContextTarget::Session("session"), true)
            .await,
        Err(ReviewError::SessionNotOpen(_))
    ));
    assert!(matches!(
        context
            .resolve(ReviewContextTarget::Session(""), false)
            .await,
        Err(ReviewError::InvalidInput(_))
    ));
    *repository.stored.lock().unwrap() = None;
    assert!(context
        .resolve(ReviewContextTarget::Session("missing"), false)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_review文脈_worktreeは親workspaceと人へ解決し解決失敗を保つ() {
    // Given
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/workspace"),
        "/isolated",
        ProviderKind::Claude,
        session_location("session"),
    )
    .unwrap();
    let context = ReviewContextUsecase::new(
        Arc::new(FailingSaveRepository::new(session)),
        Arc::new(Worktrees),
    );
    // When / Then
    assert_eq!(
        context
            .resolve(ReviewContextTarget::Worktree("/isolated"), false)
            .await
            .unwrap(),
        Some(("/workspace".into(), ReviewActor::human()))
    );
    assert!(matches!(
        context
            .resolve(ReviewContextTarget::Worktree("/missing"), false)
            .await,
        Err(ReviewError::Store(_))
    ));
}
