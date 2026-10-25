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
    let store = Arc::new(Store::default());
    let subscriptions = crate::test_support::state_subscription::test_subscriptions();
    let mut changes = subscriptions.changes();
    let comments = Arc::new(
        ReviewCommentUsecase::new(store.clone(), Arc::new(Clock), Arc::new(Ids::default()))
            .with_subscriptions(subscriptions),
    );
    let usecase = SessionReviewUsecase::new(
        ReviewContextUsecase::new(repository.clone(), Arc::new(Worktrees)),
        comments.clone(),
    );
    let data = PathBuf::from("/unused");
    // When
    let thread = usecase
        .create_thread(
            data.clone(),
            "session",
            Target {
                file_path: None,
                line_number: None,
                end_line: None,
                side: None,
                end_side: None,
            },
            "first".into(),
        )
        .await
        .unwrap();
    let appended = usecase
        .append_comment(data.clone(), "session", thread.id.clone(), "second".into())
        .await
        .unwrap();
    let resolved = usecase
        .resolve_thread(
            data.clone(),
            "session",
            thread.id.clone(),
            "fixed".into(),
            "done".into(),
        )
        .await
        .unwrap();
    // Then
    for _ in 0..3 {
        assert_eq!(
            changes.try_recv().unwrap(),
            crate::usecase::state_subscription::StateChangeSource::ReviewComments(Some(
                "/workspace".into()
            ))
        );
    }
    assert!(changes.try_recv().is_err());
    assert_eq!(thread.worktree_name, "/workspace");
    assert_eq!(
        thread.comments[0].author,
        ReviewActor::provider_agent("codex".into(), Some("session".into()))
    );
    assert_eq!(appended.comments.len(), 2);
    assert_eq!(
        usecase
            .get_thread(data.clone(), "session", thread.id.clone())
            .await
            .unwrap(),
        Some(resolved.clone())
    );
    assert_eq!(
        usecase
            .history(data.clone(), "session", thread.id.clone())
            .await
            .unwrap()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        usecase
            .list_session_threads(data.clone(), "session", ReviewThreadFilter::default())
            .await
            .unwrap(),
        Some(vec![resolved.clone()])
    );
    assert_eq!(
        usecase
            .list_worktree_threads(data.clone(), "/isolated", ReviewWorktreeFilter::default())
            .await
            .unwrap(),
        vec![resolved]
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
    let count = store.0.lock().unwrap().len();
    // Then
    assert!(usecase
        .get_thread(data.clone(), "session", thread.id.clone())
        .await
        .unwrap()
        .is_some());
    assert!(usecase
        .history(data.clone(), "session", thread.id.clone())
        .await
        .unwrap()
        .is_some());
    assert!(matches!(
        usecase
            .append_comment(
                data.clone(),
                "session",
                thread.id.clone(),
                "rejected".into()
            )
            .await,
        Err(Error::SessionNotOpen(_))
    ));
    assert!(matches!(
        usecase
            .resolve_thread(
                data.clone(),
                "session",
                thread.id.clone(),
                "fixed".into(),
                "rejected".into()
            )
            .await,
        Err(Error::SessionNotOpen(_))
    ));
    assert_eq!(store.0.lock().unwrap().len(), count);
    assert!(usecase
        .get_thread(data.clone(), "session", "missing".into())
        .await
        .unwrap()
        .is_none());
    assert!(usecase
        .history(data.clone(), "session", "missing".into())
        .await
        .unwrap()
        .is_none());
    // When
    comments
        .delete_thread(&data, "/workspace", ReviewActor::human(), &thread.id)
        .unwrap();
    // Then
    assert!(usecase
        .get_thread(data.clone(), "session", thread.id.clone())
        .await
        .unwrap()
        .is_none());
    assert!(usecase
        .history(data.clone(), "session", thread.id.clone())
        .await
        .unwrap()
        .is_none());
    // Given
    *repository.stored.lock().unwrap() = None;
    // When / Then
    assert!(usecase
        .get_thread(data.clone(), "missing", thread.id.clone())
        .await
        .unwrap()
        .is_none());
    assert!(usecase
        .history(data.clone(), "missing", thread.id)
        .await
        .unwrap()
        .is_none());
    assert!(usecase
        .list_session_threads(data, "missing", ReviewThreadFilter::default())
        .await
        .unwrap()
        .is_none());
}
