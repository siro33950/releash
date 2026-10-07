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
            "/invalid" => Err(crate::domain::workflow::WorkflowError::Validation(
                "invalid path".into(),
            )),
            "/failure" => Err(crate::domain::workflow::WorkflowError::external(
                "unavailable",
            )),
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
    let (path, actor) = context.session_for_write("session").await.unwrap();
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
    assert!(context.session_for_read("session").await.unwrap().is_some());
    assert!(matches!(
        context.session_for_write("session").await,
        Err(ReviewError::SessionNotOpen(_))
    ));
    assert!(matches!(
        context.session_for_read("").await,
        Err(ReviewError::InvalidInput(_))
    ));
    *repository.stored.lock().unwrap() = None;
    assert!(context.session_for_read("missing").await.unwrap().is_none());
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
    assert!(matches!(
        context.worktree("/invalid").await,
        Err(ReviewError::InvalidInput(_))
    ));
    assert!(matches!(
        context.worktree("/failure").await,
        Err(ReviewError::Technical(_))
    ));
    assert_eq!(
        context.worktree("/isolated").await.unwrap(),
        ("/workspace".into(), ReviewActor::human())
    );
    assert!(matches!(
        context.worktree("/missing").await,
        Err(ReviewError::NotFound(_))
    ));
}

struct FailedSessions(crate::domain::agent_session::repository::AgentSessionRepositoryError);
#[async_trait::async_trait]
impl AgentSessionRepository for FailedSessions {
    async fn find(
        &self,
        _: &str,
    ) -> Result<
        Option<crate::domain::agent_session::repository::VersionedAgentSession>,
        crate::domain::agent_session::repository::AgentSessionRepositoryError,
    > {
        Err(self.0.clone())
    }
    async fn create(
        &self,
        _: AgentSession,
        _: &str,
    ) -> Result<
        crate::domain::agent_session::repository::VersionedAgentSession,
        crate::domain::agent_session::repository::AgentSessionRepositoryError,
    > {
        unreachable!()
    }
    async fn create_with_lifecycle_events(
        &self,
        _: AgentSession,
        _: Vec<crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent>,
        _: &str,
    ) -> Result<
        crate::domain::agent_session::repository::VersionedAgentSession,
        crate::domain::agent_session::repository::AgentSessionRepositoryError,
    > {
        unreachable!()
    }
    async fn save(
        &self,
        _: crate::domain::agent_session::repository::VersionedAgentSession,
        _: &str,
    ) -> Result<
        crate::domain::agent_session::repository::VersionedAgentSession,
        crate::domain::agent_session::repository::AgentSessionRepositoryError,
    > {
        unreachable!()
    }
    async fn remove(
        &self,
        _: crate::domain::agent_session::repository::VersionedAgentSession,
        _: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        _: &str,
    ) -> Result<(), crate::domain::agent_session::repository::AgentSessionRepositoryError> {
        unreachable!()
    }
}
struct FailedWorktrees(crate::domain::workflow::WorkflowError);
#[async_trait::async_trait]
impl WorkspaceWorktreePathQuery for FailedWorktrees {
    async fn workspace_worktree_path(
        &self,
        _: &str,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        Err(self.0.clone())
    }
}

#[tokio::test]
async fn test_review文脈_sessionとworktreeの技術的失敗の分類と理由を保持する() {
    use crate::domain::agent_session::repository::AgentSessionRepositoryError;
    use crate::domain::failure::{
        StorageFailure, TechnicalFailure, TechnicalFailureNature as Nature,
    };
    use crate::domain::workflow::WorkflowError;
    fn technical(error: ReviewError) -> TechnicalFailure {
        let ReviewError::Technical(failure) = error else {
            panic!("expected technical failure: {error:?}");
        };
        failure
    }
    for nature in [
        Nature::Transient,
        Nature::TimedOut,
        Nature::Cancelled,
        Nature::Other,
    ] {
        // Given
        let failure = TechnicalFailure {
            nature,
            message: "read failed".into(),
        };
        let storage: StorageFailure = failure.clone().into();
        let expected = failure.clone();
        let context = ReviewContextUsecase::new(
            Arc::new(FailedSessions(AgentSessionRepositoryError::Store(
                storage.clone(),
            ))),
            Arc::new(FailedWorktrees(WorkflowError::Store(storage))),
        );
        // When / Then
        assert_eq!(
            technical(context.session_for_read("session").await.unwrap_err()),
            expected
        );
        assert_eq!(
            technical(context.session_for_write("session").await.unwrap_err()),
            expected
        );
        assert_eq!(
            technical(context.worktree("/workspace").await.unwrap_err()),
            expected
        );
        let context = ReviewContextUsecase::new(
            Arc::new(FailedSessions(AgentSessionRepositoryError::Unavailable)),
            Arc::new(FailedWorktrees(WorkflowError::Technical(failure))),
        );
        assert_eq!(
            technical(context.worktree("/workspace").await.unwrap_err()),
            expected
        );
    }
    // Given
    let context = ReviewContextUsecase::new(
        Arc::new(FailedSessions(AgentSessionRepositoryError::Unavailable)),
        Arc::new(Worktrees),
    );
    // When / Then
    assert_eq!(
        technical(context.session_for_read("session").await.unwrap_err()),
        TechnicalFailure {
            nature: Nature::Transient,
            message: "Unavailable".into()
        }
    );
}
