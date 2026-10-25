use super::*;
use crate::domain::agent_session::aggregates::AgentSession;
use crate::domain::agent_session::test_helpers::session_location;
use crate::domain::comment::Author as ReviewActor;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::agent_session::test_helpers::FailingSaveRepository;
use crate::usecase::workspace_tree::WorkspaceWorktreePathQuery;
struct Worktrees;
#[async_trait::async_trait]
impl WorkspaceWorktreePathQuery for Worktrees {
    async fn workspace_worktree_path(
        &self,
        path: &str,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        Ok(if path == "/isolated" {
            "/workspace"
        } else {
            path
        }
        .into())
    }
}

#[derive(Default)]
struct Store(std::sync::Mutex<Vec<crate::domain::comment::ReviewEvent>>);
impl super::super::ReviewEventStore for Store {
    fn load(
        &self,
        _: &std::path::Path,
        _: &str,
    ) -> Result<Vec<crate::domain::comment::ReviewEvent>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn mutate(
        &self,
        _: &std::path::Path,
        _: &str,
        mutation: super::super::ReviewEventMutation<'_>,
    ) -> Result<Vec<crate::domain::comment::ReviewEvent>, Error> {
        let mut events = self.0.lock().unwrap();
        let appended = mutation(&events)?;
        events.extend(appended);
        Ok(events.clone())
    }
}
struct Clock;
impl super::super::ReviewClock for Clock {
    fn now(&self) -> f64 {
        1.0
    }
}
#[derive(Default)]
struct Ids(std::sync::atomic::AtomicU64);
impl super::super::ReviewIdGenerator for Ids {
    fn event_id(&self) -> String {
        self.0
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            .to_string()
    }
}
#[tokio::test]
async fn test_review作成_存在しないsessionはnotfoundとなりstoreを変更しない() {
    // Given
    let session = AgentSession::create(
        "session",
        WorkspaceIdentity::new("/workspace"),
        "/workspace",
        ProviderKind::Codex,
        session_location("session"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(session));
    *repository.stored.lock().unwrap() = None;
    let store = Arc::new(Store::default());
    let comments = Arc::new(ReviewCommentUsecase::new(
        store.clone(),
        Arc::new(Clock),
        Arc::new(Ids::default()),
    ));
    let usecase = SessionReviewUsecase::new(
        ReviewContextUsecase::new(repository, Arc::new(Worktrees)),
        comments.clone(),
    );
    // When
    let result = usecase
        .create_thread(
            PathBuf::from("/unused"),
            "missing",
            Target {
                file_path: None,
                line_number: None,
                end_line: None,
                side: None,
                end_side: None,
            },
            "content".into(),
        )
        .await;
    // Then
    assert!(matches!(result, Err(Error::NotFound(_))));
    assert!(comments
        .list_threads(
            std::path::Path::new("/unused"),
            "/workspace",
            None,
            ReviewActor::human()
        )
        .unwrap()
        .is_empty());
    assert!(store.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_sessionレビュー_解決したworkspaceで作成追記解決と購読を行う() {
    todo!()
}
