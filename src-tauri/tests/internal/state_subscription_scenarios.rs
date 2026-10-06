use crate::state_subscription_reads::Fixture as StateReadsFixture;
use futures_util::StreamExt;
use releash_lib::test_support::integration::fixtures::fixtures_infrastructure_state_subscription_start_read as start_read;
use releash_lib::test_support::integration::fixtures::fixtures_infrastructure_state_subscription_stop_read as stop_read;
use releash_lib::test_support::integration::subscriptions::same;
use releash_lib::test_support::integration::subscriptions::Event;
use releash_lib::test_support::integration::subscriptions::StateChangeSource;
use releash_lib::test_support::integration::subscriptions::StateSubscriptionEvent;
use releash_lib::test_support::integration::subscriptions::StateValue;
use releash_lib::test_support::integration::subscriptions::SubscriptionTarget;
use releash_lib::test_support::integration::subscriptions::WatchRequirement;
use std::sync::Arc;

#[tokio::test]
pub async fn test_workspaces購読_最後の停止と切断で実際のgit監視を解放する() {
    for disconnect in [false, true] {
        // Given
        let fixture = StateReadsFixture::new();
        let repository = fixture.repository_state.clone();
        let files = Arc::new(
            releash_lib::test_support::integration::platform::SubscriptionFiles::default(),
        );
        let usecase = fixture.subscriptions.clone().with_reads(
            Arc::new(fixture.reads.clone()),
            Some(Arc::new(
                releash_lib::test_support::integration::platform::WatcherUsecase::new(
                    Some(repository.clone()),
                    files.clone(),
                ),
            )),
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
    // Given
    let fixture = StateReadsFixture::new();
    let files =
        Arc::new(releash_lib::test_support::integration::platform::SubscriptionFiles::default());
    let usecase = fixture.subscriptions.clone().with_reads(
        Arc::new(fixture.reads.clone()),
        Some(Arc::new(
            releash_lib::test_support::integration::platform::WatcherUsecase::new(
                None,
                files.clone(),
            ),
        )),
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
            releash_lib::test_support::integration::platform::ReviewActor::human(),
            releash_lib::test_support::integration::platform::ReviewTarget {
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
