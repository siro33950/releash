use crate::test_support::state_subscription::{
    same, Event, StateReadsFixture, StateSubscriptionEvent,
};
use crate::usecase::state_subscription::*;
use futures_util::StreamExt;
use std::sync::Arc;
#[tokio::test]
pub async fn test_workspaces購読_最後の停止と切断で実際のgit監視を解放する() {
    use crate::usecase::state_subscription::WatchRequirement;
    for disconnect in [false, true] {
        // Given
        let fixture = StateReadsFixture::new();
        let repository = fixture.repository_state.clone();
        let files =
            Arc::new(crate::usecase::watcher::watcher_test_helpers::SubscriptionFiles::default());
        let usecase = fixture.subscriptions.clone().with_reads(
            Arc::new(fixture.reads.clone()),
            Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
                Some(repository.clone()),
                files.clone(),
            ))),
            vec![],
            String::new(),
        );
        let first = usecase.open("first".into()).unwrap();
        let second = usecase.open("second".into()).unwrap();
        start_read(&usecase, "first", "workspaces", None)
            .await
            .unwrap();
        start_read(&usecase, "second", "workspaces", None)
            .await
            .unwrap();
        let requirement = WatchRequirement::Git(fixture.path.clone());
        let id = usecase.test_watches()[&requirement];
        let snapshot = repository.get_snapshot(&fixture.path).unwrap();
        assert_eq!(usecase.test_watches().len(), 1);
        // When / Then
        stop_read(&usecase, "first", "workspaces").await.unwrap();
        assert_eq!(usecase.test_watches()[&requirement], id);
        assert!(Arc::ptr_eq(
            &snapshot,
            &repository.get_snapshot(&fixture.path).unwrap()
        ));
        if !disconnect {
            stop_read(&usecase, "second", "workspaces").await.unwrap();
        }
        drop(second);
        assert!(usecase.test_watches().is_empty());
        assert_eq!(usecase.test_worker_count(), 0);
        assert!(!repository.stop_watching(id).unwrap());
        assert!(!Arc::ptr_eq(
            &snapshot,
            &repository.get_snapshot(&fixture.path).unwrap()
        ));
        assert!(files.active.lock().unwrap().is_empty());
        drop(first);
    }
}
#[tokio::test]
pub async fn test_review_threads購読_comment操作で再配信し最後の停止でfile監視を解放する() {
    use crate::usecase::state_subscription::{StateChangeSource, WatchRequirement};
    // Given
    let fixture = StateReadsFixture::new();
    let files =
        Arc::new(crate::usecase::watcher::watcher_test_helpers::SubscriptionFiles::default());
    let usecase = fixture.subscriptions.clone().with_reads(
        Arc::new(fixture.reads.clone()),
        Some(Arc::new(crate::usecase::watcher::WatcherUsecase::new(
            None,
            files.clone(),
        ))),
        vec![],
        String::new(),
    );
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::ReviewThreads("repository".into()).to_string();
    // When
    start_read(&usecase, "client", &target, None).await.unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if same(&value, StateValue::ReviewThreads(vec![])))
    );
    stream.next().await;
    let requirement = WatchRequirement::Files(
        fixture
            .reads
            .review_comments_dir
            .to_string_lossy()
            .into_owned(),
        StateChangeSource::ReviewComments(None),
    );
    assert!(usecase.test_watches().contains_key(&requirement));
    let thread = fixture
        .reads
        .comments
        .create_thread(
            &fixture.reads.data_dir,
            "repository",
            crate::domain::comment::ReviewActor::human(),
            crate::domain::comment::ReviewTarget {
                file_path: None,
                line_number: None,
                end_line: None,
            },
            "subscription comment".into(),
        )
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Change(_, _, value))) if same(&value, StateValue::ReviewThreads(vec![thread.into()])))
    );
    stop_read(&usecase, "client", &target).await.unwrap();
    assert!(usecase.test_watches().is_empty());
    assert_eq!(usecase.test_worker_count(), 0);
    assert!(files.active.lock().unwrap().is_empty());
}
async fn start_read(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    raw: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), StateReadError> {
    let target = SubscriptionTarget::parse(raw).map_err(StateReadError::from_error)?;
    usecase
        .deps()
        .start_subscription(client, &target, &format!("{client}:{raw}"), cursor)
        .await
}
async fn stop_read(
    usecase: &StateSubscriptionUsecase,
    client: &str,
    raw: &str,
) -> Result<(), SubscriptionError> {
    usecase
        .deps()
        .stop_subscription(&format!("{client}:{raw}"))
        .await
}
