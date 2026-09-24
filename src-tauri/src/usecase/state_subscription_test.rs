use super::*;

#[tokio::test(start_paused = true)]
async fn test_購読_配信と定期印と終了時の解放() {
    // Given
    let usecase = StateSubscriptionUsecase::new(
        vec!["/repo".into()],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(usecase.open("client".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    // When
    usecase.start("client", REPO_PATHS, None).unwrap();
    // Then
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if *value == StateValue::RepositoryPaths(vec!["/repo".into()]))
    );
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    usecase
        .publisher()
        .publish(
            REPO_PATHS,
            StateValue::RepositoryPaths(vec!["/next".into()]),
            None,
        )
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(
            _,
            Event::Change(Version { sequence: 1, .. }, Delivery::Full, _)
        ))
    ));
    tokio::time::advance(BOOKMARK_INTERVAL).await;
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(
            _,
            Event::Bookmark(Version { sequence: 1, .. })
        ))
    ));
    drop(stream);
    assert_eq!(
        usecase.start("client", REPO_PATHS, None),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(usecase.open("client".into()).is_ok());
}

#[tokio::test(start_paused = true)]
async fn test_購読_開始と配信と停止が待機中streamを起こす() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll, Wake, Waker};

    struct WakeFlag(AtomicBool);
    impl Wake for WakeFlag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    // Given
    let usecase = StateSubscriptionUsecase::new(
        vec![],
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(usecase.open("waiting".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    let mut cx = Context::from_waker(&waker);
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    flag.0.store(false, Ordering::SeqCst);

    assert_eq!(
        usecase.start("waiting", "missing", None),
        Err(SubscriptionError::UnknownTarget)
    );
    assert_eq!(
        usecase
            .publisher()
            .publish("missing", StateValue::RepositoryPaths(vec![]), None),
        Err(SubscriptionError::UnknownTarget)
    );
    assert!(!flag.0.load(Ordering::SeqCst));

    // When
    usecase.start("waiting", REPO_PATHS, None).unwrap();

    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _))))
    ));
    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_))))
    ));
    while let Poll::Ready(Some(event)) = stream.as_mut().poll_next(&mut cx) {
        assert!(matches!(
            event,
            StateSubscriptionEvent::Item(_, Event::Bookmark(_))
        ));
    }
    flag.0.store(false, Ordering::SeqCst);

    // When
    usecase
        .publisher()
        .publish(
            REPO_PATHS,
            StateValue::RepositoryPaths(vec!["/next".into()]),
            None,
        )
        .unwrap();

    // Then
    assert!(flag.0.load(Ordering::SeqCst));
    assert!(
        matches!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(StateSubscriptionEvent::Item(_, Event::Change(Version { sequence: 1, .. }, Delivery::Full, value)))) if *value == StateValue::RepositoryPaths(vec!["/next".into()]))
    );

    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    flag.0.store(false, Ordering::SeqCst);
    assert_eq!(
        usecase.stop("missing", REPO_PATHS),
        Err(SubscriptionError::StreamEnded)
    );
    assert!(!flag.0.load(Ordering::SeqCst));

    // When
    usecase.stop("waiting", REPO_PATHS).unwrap();

    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    usecase
        .publisher()
        .publish(REPO_PATHS, StateValue::RepositoryPaths(vec![]), None)
        .unwrap();
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    tokio::time::advance(BOOKMARK_INTERVAL).await;
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
}
