use super::*;
use crate::test_support::state_subscription::WakeFlag;

#[test]
fn test_配信失敗_usecaseの失敗分類へ意味を保って変換する() {
    use crate::infrastructure::state_subscription::SubscriptionError as DeliveryError;
    use crate::usecase::state_subscription::SubscriptionError;

    let cases = [
        (DeliveryError::InvalidId, SubscriptionError::InvalidId),
        (
            DeliveryError::AlreadyExists,
            SubscriptionError::AlreadyExists,
        ),
        (DeliveryError::StreamEnded, SubscriptionError::StreamEnded),
        (
            DeliveryError::UnknownTarget,
            SubscriptionError::UnknownTarget,
        ),
        (
            DeliveryError::VersionExhausted,
            SubscriptionError::VersionExhausted,
        ),
        (
            DeliveryError::SnapshotRequired,
            SubscriptionError::SnapshotRequired,
        ),
    ];
    for (source, expected) in cases {
        assert_eq!(SubscriptionError::from(source), expected);
    }
}

#[test]
fn test_terminal登録_nulを含む識別子を失敗として返す() {
    // Given
    let presenter = StateSubscriptionPresenter::new();

    // When
    let path = presenter.initialize(&crate::test_support::state_subscription::registration(
        "session", "/re\0po", None, 1, 0,
    ));
    let session = presenter.initialize(&crate::test_support::state_subscription::registration(
        "session",
        "/repo",
        Some("ses\0sion"),
        1,
        0,
    ));

    // Then
    assert!(path.is_err());
    assert!(session.is_err());
    assert_eq!(presenter.test_runtime().test_terminal_route_count(), 0);
}

#[tokio::test]
async fn test_古いterminal寸法_初回の復元要求だけ待機者へ通知する() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    presenter
        .initialize(&crate::test_support::state_subscription::registration(
            "session", "/repo", None, 1, 2,
        ))
        .unwrap();
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(presenter.stream(usecase, "waiting".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());
    let old_resize = TerminalSurfaceOutputEvent::Resize {
        session_key: "session".into(),
        cols: 80,
        rows: 24,
        sequence: 1,
    };

    // When
    TerminalSurfaceStateSink::publish(presenter.as_ref(), old_resize.clone());
    // Then
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());
    TerminalSurfaceStateSink::publish(presenter.as_ref(), old_resize);
    assert!(!flag.0.load(Ordering::SeqCst));
    TerminalSurfaceStateSink::publish(
        presenter.as_ref(),
        TerminalSurfaceOutputEvent::Exit {
            session_key: "session".into(),
            runtime_generation: 1,
            exit_code: Some(0),
            sequence: 1,
        },
    );
    assert!(!flag.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_購読開始失敗_対象削除を待機中streamへ通知する() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let target = SubscriptionTarget::BranchBase("/repo".into(), "main".into()).to_string();
    let payload = crate::adaptor::presenter::state_subscription_wire::payload(
        &StateValue::RepositoryPaths(vec![]),
    )
    .unwrap();
    presenter
        .runtime
        .update(|state| {
            state
                .register(
                    target.clone(),
                    PublishedState::from(payload),
                    Delivery::Full,
                )
                .map(|_| true)
        })
        .unwrap();
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(presenter.stream(usecase, "waiting".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());

    // When
    assert_eq!(
        presenter.start("absent", &target, None),
        Err(SubscriptionError::StreamEnded)
    );

    // Then
    assert!(!presenter.runtime.inspect(|state| state.registered(&target)));
    assert!(flag.0.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_購読再開始_状態不変なら通知せず初回開始だけ通知する() {
    use crate::infrastructure::state_subscription::Event;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Waker};

    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    let mut stream = Box::pin(presenter.stream(usecase, "client".into()).unwrap());
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Ready)
    ));
    let flag = Arc::new(WakeFlag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());

    let target = SubscriptionTarget::RepositoryPaths.to_string();
    presenter
        .publish_initial(
            &SubscriptionTarget::RepositoryPaths,
            StateValue::RepositoryPaths(vec![]),
        )
        .unwrap();
    presenter.start("client", &target, None).unwrap();
    assert!(flag.0.swap(false, Ordering::SeqCst));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, _)))
    ));
    assert!(matches!(
        stream.next().await,
        Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
    ));
    assert!(stream
        .as_mut()
        .poll_next(&mut Context::from_waker(&waker))
        .is_pending());
    flag.0.store(false, Ordering::SeqCst);

    presenter.start("client", &target, None).unwrap();
    assert!(!flag.0.load(Ordering::SeqCst));
}

#[test]
fn test_terminal読取失敗_出力sequenceを進めず次の出力を配信する() {
    // Given
    let presenter = StateSubscriptionPresenter::new();
    let target = SubscriptionTarget::from_parts("terminal", &["/repo"]).unwrap();
    let version = Version {
        epoch: "terminal".into(),
        sequence: 4,
    };
    presenter
        .runtime
        .update(|state| state.register_delta(&target.to_string(), version.clone(), 1024))
        .unwrap();
    // When
    presenter
        .publish_failure(
            &target,
            StateReadError::from_error(SubscriptionError::SnapshotRequired),
        )
        .unwrap();
    // Then
    assert_eq!(
        presenter
            .runtime
            .inspect(|state| state.current_version(&target.to_string())),
        Some(version)
    );
    let next = Version {
        epoch: "terminal".into(),
        sequence: 5,
    };
    let value = crate::adaptor::presenter::state_subscription_wire::payload(
        &StateValue::RepositoryPaths(vec![]),
    )
    .unwrap();
    presenter
        .runtime
        .update(|state| {
            state
                .publish_delta(
                    &target.to_string(),
                    next.clone(),
                    PublishedState::from(value),
                    1,
                    true,
                )
                .map(|_| true)
        })
        .unwrap();
    assert_eq!(
        presenter
            .runtime
            .inspect(|state| state.current_version(&target.to_string())),
        Some(next)
    );
}

#[tokio::test]
async fn test_購読失敗_つなぎ直した購読へ保持済みのfailure事象を送り直す() {
    // Given
    let presenter = Arc::new(StateSubscriptionPresenter::new());
    let usecase = StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
    );
    presenter
        .publish_failure(
            &SubscriptionTarget::RepositoryPaths,
            StateReadError::from_error(SubscriptionError::SnapshotRequired),
        )
        .unwrap();
    let mut initial = Box::pin(presenter.stream(usecase.clone(), "initial".into()).unwrap());
    initial.next().await;
    presenter
        .start(
            "initial",
            &SubscriptionTarget::RepositoryPaths.to_string(),
            None,
        )
        .unwrap();
    initial.next().await;
    drop(initial);
    // When
    let mut replay = Box::pin(presenter.stream(usecase, "replay".into()).unwrap());
    replay.next().await;
    presenter
        .start(
            "replay",
            &SubscriptionTarget::RepositoryPaths.to_string(),
            None,
        )
        .unwrap();
    let event = replay.next().await.unwrap();
    let event = crate::adaptor::presenter::state_subscription_wire::event(event).unwrap();
    let wire: crate::adaptor::presenter::client::StateSubscriptionEvent =
        crate::adaptor::presenter::connect_wire::to_wire(&event).unwrap();
    // Then
    assert!(matches!(
        wire.event,
        Some(crate::adaptor::presenter::client::state_subscription_event::Event::Failure(_))
    ));
}
